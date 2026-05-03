//! # pneuma-demo
//!
//! Library surface for the runnable Tier 2 demo. The binary in
//! `src/main.rs` is a thin shim that calls [`Demo::run_rename`].
//!
//! The library form lets integration tests drive the same demo with
//! a [`pneuma_ratify::MockRatifier`] instead of stdin.

#![doc = include_str!("../README.md")]

use std::path::{Path, PathBuf};

use chrono::Utc;
use thiserror::Error;

use pneuma_acts::registry;
use pneuma_core::act::ResolvedSlotValue;
use pneuma_core::{
    Act, BindingKind, Confidence, ConfidenceProducer, ConfidenceScore, ContextRef,
    ContextSnapshotId, ContractError, Directive, FileRef, PolicyEnvelope, Provenance,
    ReferentValue, ResolvedAct, ResolvedSlot, SpeechAct,
};
use pneuma_hud::{HudFrame, HudRenderer};
use pneuma_lago_bridge::{JournalRecord, JournalWriter};
use pneuma_praxis_bridge::{ExecutionOutcome, Executor, LocalPraxis, PraxisError};
use pneuma_ratify::{ApprovalDecision, Ratifier};
use pneuma_router::{Dispatch, dispatch};
use sensorium_core::{Timestamp, WorkspaceContext, WorkspaceContextBuilder};

// --- Errors ----------------------------------------------------------------

/// Demo-level errors. Wraps lower-level errors with context about
/// which phase failed.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DemoError {
    /// Error during contract finalization.
    #[error("contract: {0}")]
    Contract(ContractError),

    /// Error during executor dispatch.
    #[error("executor: {0}")]
    Executor(#[from] PraxisError),

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
}

// --- Demo runner -----------------------------------------------------------

/// The runnable demo.
///
/// Type parameter `O` is anywhere `&mut O: std::io::Write` so callers
/// can pass `std::io::stdout().lock()` for the binary path or a
/// `Vec<u8>` for tests.
pub struct Demo<'a, O: std::io::Write, R: Ratifier> {
    config: DemoConfig<'a>,
    out: O,
    ratifier: R,
    renderer: HudRenderer,
    journal: JournalWriter,
}

impl<'a, O: std::io::Write, R: Ratifier> Demo<'a, O, R> {
    /// Construct from config + sinks.
    pub fn new(config: DemoConfig<'a>, out: O, ratifier: R) -> Result<Self, DemoError> {
        let journal = JournalWriter::open(config.journal_path)?;
        let renderer = HudRenderer::new().with_width(config.hud_width);
        Ok(Self {
            config,
            out,
            ratifier,
            renderer,
            journal,
        })
    }

    /// Run the canonical "rename the focused file" demo end-to-end.
    ///
    /// Steps:
    ///
    /// 1. Build a workspace context.
    /// 2. Compose a `file.rename` directive.
    /// 3. Render the composing frame.
    /// 4. Finalize → `Ready`.
    /// 5. Render the ready frame.
    /// 6. Read approval decision.
    ///    - `Commit` → commit + execute + journal + render outcome.
    ///    - `Cancel` → return `Cancelled`.
    /// 7. After execute: read approval decision.
    ///    - `Undo` → reverse + journal.
    ///    - any other terminal → exit gracefully.
    ///
    /// Returns the [`DemoSummary`] on the happy path so callers /
    /// tests can inspect what happened.
    pub fn run_rename(&mut self) -> Result<DemoSummary, DemoError> {
        // 1. Substrate.
        let context = WorkspaceContextBuilder::neutral(Timestamp::now())
            .with_visible_files(vec![sensorium_core::FileRef::new(self.config.source_path)])
            .build();

        // 2. Compose directive.
        let act = Self::find_rename_act();
        let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);

        let composing = build_rename_directive(act, self.config.source_path, self.config.new_name);

        // 3. Render composing.
        let frame = self.renderer.render_composing(&composing);
        self.print_frame(&frame)?;

        // 4. Finalize.
        let confidence = build_confidence();
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

        // 5. Render ready.
        let frame = self.renderer.render_ready(&ready);
        self.print_frame(&frame)?;

        // 6. First approval read: commit or cancel.
        let decision = self.ratifier.read_decision();
        let committed = match decision {
            ApprovalDecision::Commit | ApprovalDecision::Approve => match ready.commit() {
                Ok(c) => c,
                Err((_, err)) => {
                    let frame = self.renderer.render_contract_error(&err);
                    self.print_frame(&frame)?;
                    return Err(err.into());
                }
            },
            ApprovalDecision::Cancel | ApprovalDecision::Reject => {
                self.journal_cancel(ready.id, "user cancelled at ready")?;
                return Err(DemoError::Cancelled);
            }
            other => {
                self.journal_cancel(ready.id, &format!("unexpected decision: {other:?}"))?;
                return Err(DemoError::Cancelled);
            }
        };

        // 7. Render committed frame.
        let frame = self.renderer.render_committed(&committed);
        self.print_frame(&frame)?;

        // 8. Journal commit.
        self.journal
            .append(&JournalRecord::committed(committed.clone()))?;
        self.journal.flush()?;

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

        // 11. Second approval read: undo or done.
        let post_decision = self.ratifier.read_decision();
        let reversed = matches!(post_decision, ApprovalDecision::Undo);
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

    fn find_rename_act() -> Act {
        registry()
            .into_iter()
            .find(|a| a.id.as_str() == "file.rename")
            .expect("file.rename canonical")
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
) -> Directive<pneuma_core::Composing> {
    let resolved = ResolvedAct::empty(act);
    let provenance = Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now());
    Directive::new(SpeechAct::Directive, resolved)
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
        )
        .with_utterance(format!("rename {} to {}", source_path.display(), new_name))
}

fn build_confidence() -> Confidence {
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

/// Return a [`WorkspaceContext`] populated with a single visible file.
/// Exposed for tests that want to drive the demo with their own
/// substrate.
#[must_use]
pub fn build_demo_context(focused_file: &Path) -> WorkspaceContext {
    WorkspaceContextBuilder::neutral(Timestamp::now())
        .with_visible_files(vec![sensorium_core::FileRef::new(focused_file)])
        .build()
}
