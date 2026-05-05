//! # pneuma-demo
//!
//! Library surface for the runnable Tier 2 demo. The binary in
//! `src/main.rs` is a thin shim that calls [`Demo::run_rename`].
//!
//! The library form lets integration tests drive the same demo with
//! a [`pneuma_ratify::MockRatifier`] instead of stdin.

#![doc = include_str!("../README.md")]

mod parser;

pub use parser::{ParseError, ParsedUtterance, parse_utterance};

use std::path::{Path, PathBuf};

use chrono::Utc;
use thiserror::Error;

use pneuma_acts::registry;
use pneuma_arcan_bridge::{ArcanError, ArcanExecutor};
use pneuma_core::act::ResolvedSlotValue;
use pneuma_core::{
    Act, AppId, BindingKind, Confidence, ConfidenceProducer, ConfidenceScore, ContextRef,
    ContextSnapshotId, ContractError, Directive, FileRef, PolicyEnvelope, Provenance,
    ReferentValue, ResolvedAct, ResolvedSlot, SpeechAct,
};
use pneuma_hud::{HudFrame, HudRenderer};
use pneuma_lago_bridge::{JournalRecord, JournalWriter};
use pneuma_praxis_bridge::{ExecutionOutcome, Executor, LocalPraxis, PraxisError};
use pneuma_ratify::{ApprovalDecision, Ratifier};
use pneuma_router::{Dispatch, dispatch};
use sensorium_context::Observer;
use sensorium_core::WorkspaceContext;

// --- Errors ----------------------------------------------------------------

/// Demo-level errors. Wraps lower-level errors with context about
/// which phase failed.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DemoError {
    /// Error during contract finalization.
    #[error("contract: {0}")]
    Contract(ContractError),

    /// Error during Praxis executor dispatch.
    #[error("executor: {0}")]
    Executor(#[from] PraxisError),

    /// Error during Arcan executor dispatch.
    #[error("agent: {0}")]
    Arcan(ArcanError),

    /// Journal I/O error.
    #[error("journal: {0}")]
    Journal(#[from] pneuma_lago_bridge::JournalError),

    /// User cancelled the demo.
    #[error("user cancelled")]
    Cancelled,

    /// Router refused dispatch.
    #[error("router refused: {0}")]
    Refused(String),
}

impl From<ContractError> for DemoError {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}

// --- Demo configuration ----------------------------------------------------

/// Demo configuration — everything wires off this.
#[derive(Debug)]
pub struct DemoConfig<'a> {
    /// Where the focused file lives.
    pub source_path: &'a Path,
    /// Name to rename it to (basename only).
    pub new_name: &'a str,
    /// Where to write the journal.
    pub journal_path: &'a Path,
    /// Width hint for the HUD.
    pub hud_width: usize,
    /// Optional natural-language utterance to attach to the
    /// directive. If `Some`, it's recorded verbatim in the
    /// directive's `utterance` field (used by Arcan dispatch and
    /// available to the journal). Phase 2 acts derive `new_name`
    /// upstream via [`parse_utterance`] before constructing the
    /// config.
    pub utterance: Option<&'a str>,
}

// --- Demo runner -----------------------------------------------------------

/// The runnable demo.
///
/// Type parameter `O` is anywhere `&mut O: std::io::Write` so callers
/// can pass `std::io::stdout().lock()` for the binary path or a
/// `Vec<u8>` for tests.
///
/// The `observer` is a `Box<dyn Observer>` so callers can swap
/// implementations at runtime — `ManualObserver` for scripted demos,
/// `FsObserver` for real filesystem observation, custom mocks for
/// tests.
pub struct Demo<'a, O: std::io::Write, R: Ratifier> {
    config: DemoConfig<'a>,
    out: O,
    ratifier: R,
    observer: Box<dyn Observer>,
    renderer: HudRenderer,
    journal: JournalWriter,
}

impl<'a, O: std::io::Write, R: Ratifier> Demo<'a, O, R> {
    /// Construct from config + sinks + observer.
    ///
    /// The observer is queried once in [`Demo::run_rename`] to read
    /// the substrate state at finalize-time. Pass a
    /// [`sensorium_context::ManualObserver`] populated with the
    /// focused file for the simplest case.
    pub fn new(
        config: DemoConfig<'a>,
        out: O,
        ratifier: R,
        observer: Box<dyn Observer>,
    ) -> Result<Self, DemoError> {
        let journal = JournalWriter::open(config.journal_path)?;
        let renderer = HudRenderer::new().with_width(config.hud_width);
        Ok(Self {
            config,
            out,
            ratifier,
            observer,
            renderer,
            journal,
        })
    }

    /// Run the canonical "rename the focused file" demo end-to-end.
    ///
    /// Thin wrapper over [`Self::run_directive_lifecycle`] that supplies
    /// the rename-specific directive, confidence, and policy.
    /// See [`Self::run_directive_lifecycle`] for the full lifecycle.
    pub fn run_rename(&mut self) -> Result<DemoSummary, DemoError> {
        let act = Self::find_rename_act();
        let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
        let composing = build_rename_directive(
            act,
            self.config.source_path,
            self.config.new_name,
            self.config.utterance,
        );
        let confidence = build_rename_confidence();
        self.run_directive_lifecycle(composing, confidence, policy)
    }

    /// Run a `browser.navigate` directive end-to-end.
    ///
    /// Same correction-loop machinery as [`Self::run_rename`] — composes,
    /// finalizes, prompts, commits, dispatches via the router, executes
    /// against [`LocalPraxis`], journals, optionally reverses. The
    /// difference is the directive shape: `browser.navigate` carries a
    /// `Referent::Url` slot, no `target` file, no tempfile setup. On
    /// non-macOS platforms execution surfaces
    /// [`PraxisError::PlatformUnsupported`][unsup]; the demo still walks
    /// the contract chain up to that point.
    ///
    /// `url` is the destination URL — the parser produces it, the
    /// caller supplies it. v0.2 trusts the caller; future versions will
    /// validate via `url::Url::parse`.
    ///
    /// [unsup]: pneuma_praxis_bridge::PraxisError::PlatformUnsupported
    pub fn run_navigate(&mut self, url: &str) -> Result<DemoSummary, DemoError> {
        let act = Self::find_navigate_act();
        let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
        let composing = build_navigate_directive(act, url, self.config.utterance);
        let confidence = build_navigate_confidence();
        self.run_directive_lifecycle(composing, confidence, policy)
    }

    /// Run a `workspace.switch_app` directive end-to-end (step #14).
    ///
    /// Same correction-loop machinery as [`Self::run_navigate`]. Slot is
    /// `target: Referent::App(AppId)`. The act is `Reversibility::Free`,
    /// so committing executes immediately without ratify (per
    /// `PolicyEnvelope::intrinsic`); the demo still walks the prompt
    /// loop because the demo always asks before doing.
    ///
    /// On non-macOS platforms execution surfaces
    /// `PraxisError::PlatformUnsupported`.
    ///
    /// `app_name` is validated upstream by [`AppId::new`] (rejects
    /// empty / whitespace-only). Other validation happens at the
    /// AppleScript boundary in `pneuma-praxis-bridge`.
    pub fn run_switch_app(&mut self, app_name: &str) -> Result<DemoSummary, DemoError> {
        let act = Self::find_switch_app_act();
        let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
        let composing = build_switch_app_directive(act, app_name, self.config.utterance)?;
        let confidence = build_switch_app_confidence();
        self.run_directive_lifecycle(composing, confidence, policy)
    }

    /// Run an `agent.*` directive through the supplied
    /// [`ArcanExecutor`] (step #16b of `MIL-PROJECT.md` §11.2).
    ///
    /// This is the user-facing surface for the agent path. The
    /// directive's `instruction` field is the natural-language ask
    /// (taken verbatim from the parsed utterance); slots carry whatever
    /// resolved entities the parser surfaced (target file, app name,
    /// URL, etc.). The router emits `Dispatch::Arcan(AgentPrompt)`;
    /// the bridge formats it for stdin and dispatches to the
    /// configured agent CLI subprocess.
    ///
    /// `act_id` selects which `agent.*` act to use (`agent.refactor`,
    /// `agent.explain`, `agent.review`, `agent.generate`). The slot
    /// payload binds whichever slots the act requires.
    pub fn run_arcan(
        &mut self,
        act_id: &str,
        instruction: &str,
        payload_slots: Vec<(String, ResolvedSlotValue)>,
        arcan: &dyn ArcanExecutor,
    ) -> Result<DemoSummary, DemoError> {
        let act = Self::find_arcan_act(act_id)?;
        let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
        let composing = build_arcan_directive(act, instruction, payload_slots);
        let confidence = build_arcan_confidence(&composing);
        self.run_directive_lifecycle_arcan(composing, confidence, policy, arcan)
    }

    /// The act-agnostic correction-loop driver for **Praxis** flows.
    ///
    /// Steps:
    ///
    /// 1. Build a workspace context.
    /// 2. Render the composing frame for the supplied directive.
    /// 3. Finalize → `Ready`.
    /// 4. Render the ready frame.
    /// 5. Pre-commit prompt loop.
    ///    - `Commit` → commit + execute + journal + render outcome.
    ///    - `Cancel` → return `Cancelled`.
    /// 6. Post-execute prompt loop.
    ///    - `Undo` → reverse + journal.
    ///    - any other terminal → exit gracefully.
    ///
    /// The caller supplies a fully-composed [`Directive`], its slot
    /// confidences, and its policy envelope. Everything downstream
    /// (HUD frames, journal records, ratify prompts) is act-agnostic.
    ///
    /// For Arcan flows, see [`Self::run_directive_lifecycle_arcan`].
    pub fn run_directive_lifecycle(
        &mut self,
        composing: Directive<pneuma_core::Composing>,
        confidence: Confidence,
        policy: PolicyEnvelope,
    ) -> Result<DemoSummary, DemoError> {
        let (committed, context) = self.compose_ratify_and_commit(composing, confidence, policy)?;

        // 9. Route + execute.
        let routed = dispatch(&committed, &context);
        let praxis_call = match routed {
            Dispatch::Praxis(call) => call,
            Dispatch::Refuse(reason) => {
                return Err(DemoError::Refused(format!("{reason:?}")));
            }
            other => {
                return Err(DemoError::Refused(format!(
                    "demo only supports Praxis dispatch; got {other:?}"
                )));
            }
        };
        let outcome = match LocalPraxis.execute(&praxis_call) {
            Ok(o) => o,
            Err(err) => {
                let frame = self.renderer.render_praxis_error(&err);
                self.print_frame(&frame)?;
                self.journal
                    .append(&JournalRecord::failed(committed.id, format!("{err}")))?;
                self.journal.flush()?;
                return Err(err.into());
            }
        };

        // 10. Render + journal outcome.
        let frame = self.renderer.render_outcome(&outcome);
        self.print_frame(&frame)?;
        self.journal
            .append(&JournalRecord::executed(committed.id, outcome.clone()))?;
        self.journal.flush()?;

        // 11. Post-execute prompt loop. Only Undo runs the reversal;
        //     anything else exits cleanly. (UX finding #2: distinct
        //     prompt for the post-execute context.)
        //
        //     `clippy::match_same_arms` is allowed because the
        //     "exit-clean" arms (Cancel/Reject/Commit/Approve) and the
        //     non-exhaustive `_` fallthrough are conceptually distinct
        //     even though all map to `break false` in v0.2.
        #[allow(clippy::match_same_arms)]
        let reversed = loop {
            self.print_post_execute_prompt()?;
            match self.ratifier.read_decision() {
                ApprovalDecision::Undo => break true,
                ApprovalDecision::Cancel
                | ApprovalDecision::Reject
                | ApprovalDecision::Commit
                | ApprovalDecision::Approve => break false,
                ApprovalDecision::Continue => {
                    self.print_info(
                        "NOTE",
                        "unrecognized input; press u to undo, Enter to keep.",
                    )?;
                }
                ApprovalDecision::Engage | ApprovalDecision::Clarify(_) => {
                    self.print_info("NOTE", "no-op here; press u to undo, Enter to keep.")?;
                }
                // ApprovalDecision is #[non_exhaustive]; future variants
                // exit cleanly without undoing.
                _ => break false,
            }
        };

        if reversed {
            LocalPraxis.reverse(&praxis_call, &outcome)?;
            self.journal.append(&JournalRecord::reversed(
                committed.id,
                outcome.reverse_action.clone(),
            ))?;
            self.journal.flush()?;
            let frame = self
                .renderer
                .render_info("UNDONE", "reverse-action ran; file restored.");
            self.print_frame(&frame)?;
        }

        Ok(DemoSummary {
            directive_id: committed.id,
            outcome: Some(outcome),
            reversed,
            journal_path: self.config.journal_path.to_path_buf(),
        })
    }

    /// Run an `agent.*` directive end-to-end via the supplied
    /// [`ArcanExecutor`] (step #16).
    ///
    /// Same correction-loop machinery as [`Self::run_directive_lifecycle`]
    /// up to the dispatch boundary, then routes via Arcan instead of
    /// Praxis. The agent's response is rendered into a HUD frame and
    /// recorded as `JournalRecord::AgentExecuted`.
    ///
    /// Arcan acts have no v0.2 reverse path: the agent is responsible
    /// for its own undo recipe at completion-time. The post-execute
    /// prompt simply lets the user acknowledge and exit.
    pub fn run_directive_lifecycle_arcan(
        &mut self,
        composing: Directive<pneuma_core::Composing>,
        confidence: Confidence,
        policy: PolicyEnvelope,
        arcan: &dyn ArcanExecutor,
    ) -> Result<DemoSummary, DemoError> {
        let (committed, context) = self.compose_ratify_and_commit(composing, confidence, policy)?;

        // Route. Agent acts have ExecutorHint::Arcan, so we expect
        // Dispatch::Arcan(prompt). Anything else surfaces as Refused.
        let routed = dispatch(&committed, &context);
        let prompt = match routed {
            Dispatch::Arcan(p) => p,
            Dispatch::Refuse(reason) => {
                return Err(DemoError::Refused(format!("{reason:?}")));
            }
            other => {
                return Err(DemoError::Refused(format!(
                    "expected Arcan dispatch for agent act; got {other:?}"
                )));
            }
        };

        // Execute via the supplied ArcanExecutor.
        let outcome = match arcan.execute(&prompt) {
            Ok(o) => o,
            Err(err) => {
                let frame = self.renderer.render_info("AGENT ERROR", &format!("{err}"));
                self.print_frame(&frame)?;
                self.journal
                    .append(&JournalRecord::failed(committed.id, format!("{err}")))?;
                self.journal.flush()?;
                return Err(DemoError::Arcan(err));
            }
        };

        // Render + journal the agent's response.
        let frame = self.renderer.render_arcan_outcome(
            outcome.act_id.as_str(),
            &outcome.executor,
            &outcome.response,
            outcome.exit_code,
        );
        self.print_frame(&frame)?;
        self.journal.append(&JournalRecord::agent_executed(
            committed.id,
            &outcome.executor,
            &outcome.response,
            outcome.exit_code,
        ))?;
        self.journal.flush()?;

        // Post-execute prompt — Arcan has no v0.2 reverse, so any
        // terminal decision exits cleanly. The "exit-clean" arms and
        // the non-exhaustive _ fallthrough are conceptually distinct
        // even though all map to `break` in v0.2.
        #[allow(clippy::match_same_arms)]
        loop {
            self.print_info(
                "POST-EXECUTE",
                "agent response recorded — [Enter / q] to exit",
            )?;
            match self.ratifier.read_decision() {
                ApprovalDecision::Cancel
                | ApprovalDecision::Reject
                | ApprovalDecision::Commit
                | ApprovalDecision::Approve => break,
                ApprovalDecision::Undo => {
                    self.print_info(
                        "NOTE",
                        "agent acts have no v0.2 reverse — the agent's response is recorded.",
                    )?;
                }
                ApprovalDecision::Continue => {
                    self.print_info("NOTE", "press Enter or q to exit.")?;
                }
                ApprovalDecision::Engage | ApprovalDecision::Clarify(_) => {
                    self.print_info("NOTE", "no-op here; press Enter or q to exit.")?;
                }
                _ => break,
            }
        }

        Ok(DemoSummary {
            directive_id: committed.id,
            // ArcanOutcome is structurally distinct from
            // ExecutionOutcome; v0.2 leaves DemoSummary's Praxis-shaped
            // outcome field as None for arcan flows. The journal carries
            // the typed AgentExecuted record.
            outcome: None,
            reversed: false,
            journal_path: self.config.journal_path.to_path_buf(),
        })
    }

    /// Shared compose-ratify-commit-journal phase.
    ///
    /// Steps 1–8 of `run_directive_lifecycle*` — read context, render
    /// composing, finalize, render ready, pre-commit prompt loop,
    /// commit, render committed, journal commit. Returns the
    /// committed directive plus the workspace context it was committed
    /// against, so the caller can dispatch through the router.
    fn compose_ratify_and_commit(
        &mut self,
        composing: Directive<pneuma_core::Composing>,
        confidence: Confidence,
        policy: PolicyEnvelope,
    ) -> Result<(Directive<pneuma_core::Committed>, WorkspaceContext), DemoError> {
        // 1. Substrate. Read from the observer.
        let context: WorkspaceContext = self.observer.current();

        // 2. Render composing.
        let frame = self.renderer.render_composing(&composing);
        self.print_frame(&frame)?;

        // 3. Finalize.
        let snapshot = context.snapshot();
        let context_ref = ContextRef::new(
            ContextSnapshotId::from_uuid(snapshot.id.into_inner()),
            snapshot.taken_at.into_inner(),
        );
        let ready = match composing.try_finalize(context_ref, policy, confidence) {
            Ok(r) => r,
            Err((_, err)) => {
                let frame = self.renderer.render_contract_error(&err);
                self.print_frame(&frame)?;
                return Err(err.into());
            }
        };

        // 4. Render ready.
        let frame = self.renderer.render_ready(&ready);
        self.print_frame(&frame)?;

        // 5. Pre-commit prompt loop.
        let committed = loop {
            self.print_pre_commit_prompt()?;
            match self.ratifier.read_decision() {
                ApprovalDecision::Commit | ApprovalDecision::Approve => match ready.commit() {
                    Ok(c) => break c,
                    Err((_returned_ready, err)) => {
                        let frame = self.renderer.render_contract_error(&err);
                        self.print_frame(&frame)?;
                        return Err(err.into());
                    }
                },
                ApprovalDecision::Cancel | ApprovalDecision::Reject => {
                    self.journal_cancel(ready.id, "user cancelled at ready")?;
                    self.print_info("CANCELLED", "directive discarded; nothing executed.")?;
                    return Err(DemoError::Cancelled);
                }
                ApprovalDecision::Undo => {
                    self.print_info(
                        "NOTE",
                        "nothing to undo yet — directive hasn't committed. Press Enter or q.",
                    )?;
                }
                ApprovalDecision::Engage => {
                    self.print_info("NOTE", "engage gesture is for compose-time; ignored here.")?;
                }
                ApprovalDecision::Clarify(_) => {
                    self.print_info(
                        "NOTE",
                        "clarification is a v0.3 feature; press Enter to commit, q to cancel.",
                    )?;
                }
                ApprovalDecision::Continue => {
                    self.print_info(
                        "NOTE",
                        "unrecognized input; press Enter to commit, q to cancel.",
                    )?;
                }
                _ => {
                    self.print_info(
                        "NOTE",
                        "unsupported decision; press Enter to commit, q to cancel.",
                    )?;
                }
            }
        };

        // 6. Render committed frame.
        let frame = self.renderer.render_committed(&committed);
        self.print_frame(&frame)?;

        // 7. Journal commit.
        self.journal
            .append(&JournalRecord::committed(committed.clone()))?;
        self.journal.flush()?;

        Ok((committed, context))
    }

    fn print_pre_commit_prompt(&mut self) -> Result<(), DemoError> {
        self.print_info(
            "RATIFY",
            "[Enter] commit and execute    [q / Esc] cancel and discard",
        )
    }

    fn print_post_execute_prompt(&mut self) -> Result<(), DemoError> {
        self.print_info(
            "POST-EXECUTE",
            "[u] undo the action    [Enter / q] keep and exit",
        )
    }

    fn print_info(&mut self, label: &str, body: &str) -> Result<(), DemoError> {
        let frame = self.renderer.render_info(label, body);
        self.print_frame(&frame)
    }

    fn find_rename_act() -> Act {
        registry()
            .into_iter()
            .find(|a| a.id.as_str() == "file.rename")
            .expect("file.rename canonical")
    }

    fn find_navigate_act() -> Act {
        registry()
            .into_iter()
            .find(|a| a.id.as_str() == "browser.navigate")
            .expect("browser.navigate canonical")
    }

    fn find_switch_app_act() -> Act {
        registry()
            .into_iter()
            .find(|a| a.id.as_str() == "workspace.switch_app")
            .expect("workspace.switch_app canonical")
    }

    fn find_arcan_act(act_id: &str) -> Result<Act, DemoError> {
        registry()
            .into_iter()
            .find(|a| a.id.as_str() == act_id)
            .ok_or_else(|| {
                DemoError::Refused(format!("agent act `{act_id}` is not in the registry"))
            })
    }

    fn print_frame(&mut self, frame: &HudFrame) -> Result<(), DemoError> {
        writeln!(self.out, "{}", frame.body)
            .map_err(|e| DemoError::Journal(pneuma_lago_bridge::JournalError::Io(e)))?;
        Ok(())
    }

    fn journal_cancel(
        &mut self,
        directive_id: pneuma_core::DirectiveId,
        reason: &str,
    ) -> Result<(), DemoError> {
        self.journal
            .append(&JournalRecord::cancelled(directive_id, reason))?;
        self.journal.flush()?;
        Ok(())
    }
}

// --- DemoSummary -----------------------------------------------------------

/// Result of a successful demo run.
#[derive(Debug, Clone)]
pub struct DemoSummary {
    /// The directive's UUIDv7 id.
    pub directive_id: pneuma_core::DirectiveId,
    /// The execution outcome, if execution succeeded.
    pub outcome: Option<ExecutionOutcome>,
    /// Whether the demo also exercised the undo path.
    pub reversed: bool,
    /// Where the journal was written.
    pub journal_path: PathBuf,
}

// --- Helpers --------------------------------------------------------------

fn build_rename_directive(
    act: Act,
    source_path: &Path,
    new_name: &str,
    utterance: Option<&str>,
) -> Directive<pneuma_core::Composing> {
    let resolved = ResolvedAct::empty(act);
    let provenance = Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now());
    let directive = Directive::new(SpeechAct::Directive, resolved)
        .bind_slot(
            ResolvedSlot::new(
                "target",
                ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new(source_path))),
                provenance.clone(),
            )
            .expect("slot is non-empty"),
        )
        .bind_slot(
            ResolvedSlot::new(
                "new_name",
                ResolvedSlotValue::String(new_name.to_owned()),
                provenance,
            )
            .expect("slot is non-empty"),
        );

    // If the caller supplied an utterance (parsed from speech / text
    // input), attach it. Otherwise synthesize a canonical-form
    // utterance for diagnostics.
    let utterance_text = utterance.map_or_else(
        || format!("rename {} to {}", source_path.display(), new_name),
        str::to_owned,
    );
    directive.with_utterance(utterance_text)
}

fn build_rename_confidence() -> Confidence {
    Confidence::from_slots(vec![
        (
            "target".to_owned(),
            ConfidenceScore::new(0.95, true, ConfidenceProducer::Deterministic).unwrap(),
        ),
        (
            "new_name".to_owned(),
            ConfidenceScore::new(0.95, true, ConfidenceProducer::Deterministic).unwrap(),
        ),
    ])
    .expect("confidence is constructible")
}

fn build_navigate_directive(
    act: Act,
    url: &str,
    utterance: Option<&str>,
) -> Directive<pneuma_core::Composing> {
    let resolved = ResolvedAct::empty(act);
    let provenance = Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now());
    let directive = Directive::new(SpeechAct::Directive, resolved).bind_slot(
        ResolvedSlot::new(
            "url",
            ResolvedSlotValue::Referent(ReferentValue::Url(url.to_owned())),
            provenance,
        )
        .expect("slot is non-empty"),
    );

    // Attach a parsed utterance verbatim if supplied; otherwise
    // synthesize a canonical-form utterance for diagnostics.
    let utterance_text = utterance.map_or_else(|| format!("navigate to {url}"), str::to_owned);
    directive.with_utterance(utterance_text)
}

fn build_navigate_confidence() -> Confidence {
    Confidence::from_slots(vec![(
        "url".to_owned(),
        ConfidenceScore::new(0.95, true, ConfidenceProducer::Deterministic).unwrap(),
    )])
    .expect("confidence is constructible")
}

fn build_switch_app_directive(
    act: Act,
    app_name: &str,
    utterance: Option<&str>,
) -> Result<Directive<pneuma_core::Composing>, DemoError> {
    let resolved = ResolvedAct::empty(act);
    let provenance = Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now());
    let app_id = AppId::new(app_name).map_err(DemoError::Contract)?;
    let directive = Directive::new(SpeechAct::Directive, resolved).bind_slot(
        ResolvedSlot::new(
            "target",
            ResolvedSlotValue::Referent(ReferentValue::App(app_id)),
            provenance,
        )
        .expect("slot is non-empty"),
    );

    let utterance_text = utterance.map_or_else(|| format!("switch to {app_name}"), str::to_owned);
    Ok(directive.with_utterance(utterance_text))
}

fn build_arcan_directive(
    act: Act,
    instruction: &str,
    payload_slots: Vec<(String, ResolvedSlotValue)>,
) -> Directive<pneuma_core::Composing> {
    // Inspect the act's required slot signatures so we can auto-bind
    // an `instruction` slot (some agent acts — refactor, generate —
    // declare it as a required String slot in addition to the
    // directive's utterance field).
    let needs_instruction_slot = act.slots.iter().any(|s| s.name == "instruction");

    // payload_slots may already supply `target`; if `instruction`
    // is required and not in payload_slots, fall back to `instruction`
    // parameter.
    let already_has_instruction = payload_slots.iter().any(|(n, _)| n == "instruction");

    let resolved = ResolvedAct::empty(act);
    let provenance = Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now());
    let mut directive = Directive::new(SpeechAct::Directive, resolved);
    for (name, value) in payload_slots {
        directive = directive.bind_slot(
            ResolvedSlot::new(&name, value, provenance.clone()).expect("slot is non-empty"),
        );
    }
    if needs_instruction_slot && !already_has_instruction {
        directive = directive.bind_slot(
            ResolvedSlot::new(
                "instruction",
                ResolvedSlotValue::String(instruction.to_owned()),
                provenance,
            )
            .expect("instruction slot is non-empty"),
        );
    }
    // The directive's utterance carries the natural-language
    // instruction. The Pneuma router's `dispatch` function pulls this
    // into AgentPrompt::instruction so the agent CLI sees the
    // user's actual ask.
    directive.with_utterance(instruction.to_owned())
}

fn build_arcan_confidence(composing: &Directive<pneuma_core::Composing>) -> Confidence {
    // Confidence is per-slot; agent acts have variable slot signatures.
    // Build a 0.95 score for every bound slot so finalize() passes.
    let entries: Vec<_> = composing
        .act
        .bindings
        .iter()
        .map(|s| {
            (
                s.name.clone(),
                ConfidenceScore::new(0.95, true, ConfidenceProducer::Deterministic).unwrap(),
            )
        })
        .collect();
    if entries.is_empty() {
        // No slots — Confidence::from_slots requires at least one
        // entry. Synthesize a placeholder for the act itself.
        return Confidence::from_slots(vec![(
            "_act".to_owned(),
            ConfidenceScore::new(0.95, true, ConfidenceProducer::Deterministic).unwrap(),
        )])
        .expect("placeholder confidence is constructible");
    }
    Confidence::from_slots(entries).expect("agent confidence is constructible")
}

fn build_switch_app_confidence() -> Confidence {
    Confidence::from_slots(vec![(
        "target".to_owned(),
        ConfidenceScore::new(0.95, true, ConfidenceProducer::Deterministic).unwrap(),
    )])
    .expect("confidence is constructible")
}

/// Build a [`sensorium_context::ManualObserver`] populated with the
/// focused file as its only visible file. Used by the binary and
/// integration tests as the standard observer for the rename demo.
#[must_use]
pub fn manual_observer_for(focused_file: &Path) -> sensorium_context::ManualObserver {
    let observer = sensorium_context::ManualObserver::new(sensorium_core::Timestamp::now());
    observer.set_focused_file(sensorium_core::FileRef::new(focused_file), false);
    observer
}
