//! # pneuma-hud
//!
//! Pure rendering for MIL's live-preview HUD. The Tier 2 Week 3
//! deliverable that closes the correction loop.
//!
//! ## Why pure rendering
//!
//! The synthesis doc flagged risk #3 — live-preview cadence — as a
//! Tier-2 finding waiting to happen. Our stance: **rendering is pure;
//! cadence is the caller's choice.** Each render is a function of its
//! input alone. A v0.2 demo prints frames on state transitions; a v0.3
//! production HUD will repaint at 30 Hz from a render thread; both use
//! the same render functions. Decoupling them is the architectural
//! invariant.
//!
//! ## What gets rendered
//!
//! Every load-bearing state of the directive lifecycle, plus the
//! executor outcome / error states:
//!
//! | State                  | Rendering goal                                          |
//! |------------------------|---------------------------------------------------------|
//! | `Directive<Composing>` | Slot-by-slot fill bar; unbound required slots flagged   |
//! | `Directive<Ready>`     | "Ready to commit" + policy summary                      |
//! | `Directive<Proposed>`  | "Awaiting ratification" + ratify-window countdown       |
//! | `Directive<Committed>` | "Dispatched" + executor target                          |
//! | `ExecutionOutcome`     | "Done" + result payload + reverse-action availability   |
//! | `ContractError`        | Slot or threshold violation, before dispatch            |
//! | `PraxisError`          | Executor-time failure                                   |
//!
//! ## Rendering style
//!
//! ASCII boxes, no ANSI colors yet. v0.3 will add color via a feature
//! flag. The current width target is 80 columns; pass a larger width
//! to [`HudRenderer::with_width`] for wider panes.

#![doc = include_str!("../README.md")]

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use pneuma_core::act::ResolvedSlotValue;
use pneuma_core::{
    Arity, Committed, Composing, ContractError, Directive, Modifier, PolicyEnvelope, Proposed,
    Ready,
};
use pneuma_praxis_bridge::{ExecutionOutcome, PraxisError, ReverseAction};

// --- HudFrame --------------------------------------------------------------

/// What kind of frame this is. Used by callers that journal frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum HudFrameKind {
    /// Mid-composition. Slots being filled.
    Composing,
    /// Finalized; ready to commit.
    Ready,
    /// Awaiting user ratification.
    Proposed,
    /// Committed and dispatched.
    Committed,
    /// Execution succeeded.
    Outcome,
    /// Contract-level error (pre-dispatch).
    ContractError,
    /// Executor-level error (during dispatch).
    PraxisError,
    /// Free-form info / status frame.
    Info,
}

/// A single rendered HUD frame.
///
/// Owned `String` body plus typed `kind`. Cheap to clone; cheap to
/// journal. The body is meant to be printed verbatim — no further
/// processing expected.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HudFrame {
    /// What this frame represents.
    pub kind: HudFrameKind,
    /// The rendered text. Multi-line; trailing newline omitted by
    /// convention so callers can choose whether to add one.
    pub body: String,
}

impl HudFrame {
    /// Construct.
    #[must_use]
    pub fn new(kind: HudFrameKind, body: impl Into<String>) -> Self {
        Self {
            kind,
            body: body.into(),
        }
    }
}

// --- HudRenderer -----------------------------------------------------------

/// Stateless renderer.
///
/// Configurable width; everything else is implicit in the methods.
#[derive(Debug, Clone, Copy)]
pub struct HudRenderer {
    width: usize,
}

impl Default for HudRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl HudRenderer {
    /// Default 80-column renderer.
    #[must_use]
    pub fn new() -> Self {
        Self { width: 80 }
    }

    /// Override the width. Capped at 200 columns to keep things sane.
    #[must_use]
    pub fn with_width(mut self, width: usize) -> Self {
        self.width = width.clamp(40, 200);
        self
    }

    /// The width the renderer is using.
    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    // --- Composing ---------------------------------------------------------

    /// Render an in-flight (`Composing`) directive.
    ///
    /// Shows: act id, speech act, every slot signature with its current
    /// binding (or `…` if unbound), and the count of unbound required
    /// slots.
    pub fn render_composing(&self, d: &Directive<Composing>) -> HudFrame {
        let mut body = String::with_capacity(self.width * 8);
        self.title(
            &mut body,
            "COMPOSING",
            &format!("act: {}", d.act.act.id.as_str()),
        );
        let _ = writeln!(body, "│ speech-act: {:?}", d.speech_act);
        if let Some(utt) = &d.utterance {
            let _ = writeln!(body, "│ utterance:  {}", utt);
        }
        body.push_str("│\n");
        body.push_str("│ slots:\n");
        for sig in &d.act.act.slots {
            let bound = d.act.binding(&sig.name);
            let mark = match (bound, sig.arity) {
                (Some(_), _) => "✓",
                (None, Arity::Required) => "✗",
                (None, _) => "·",
            };
            let render_value = bound
                .map(|b| render_slot_value(&b.value))
                .unwrap_or_else(|| "…".to_owned());
            let _ = writeln!(
                body,
                "│   {} {:<14} ({:?}) = {}",
                mark, sig.name, sig.arity, render_value
            );
        }
        let unbound = d.act.unbound_required().count();
        if !d.modifiers.is_empty() {
            body.push_str("│\n│ modifiers:\n");
            for m in &d.modifiers {
                let _ = writeln!(body, "│   • {}", render_modifier(m));
            }
        }
        body.push_str("│\n");
        let _ = writeln!(
            body,
            "│ unbound required: {}{}",
            unbound,
            if unbound == 0 {
                "  (ready to finalize)"
            } else {
                ""
            }
        );
        self.bottom(&mut body);
        HudFrame::new(HudFrameKind::Composing, body)
    }

    // --- Ready -------------------------------------------------------------

    /// Render a `Ready` directive: about to commit, policy known.
    pub fn render_ready(&self, d: &Directive<Ready>) -> HudFrame {
        let mut body = String::with_capacity(self.width * 8);
        self.title(
            &mut body,
            "READY",
            &format!("act: {}", d.act.act.id.as_str()),
        );
        let policy = d.policy.as_ref().expect("Ready has policy");
        let confidence = d.confidence.as_ref().expect("Ready has confidence");
        body.push_str(&render_policy_lines(policy));
        let _ = writeln!(
            body,
            "│ confidence: {:.3} (calibrated: {})",
            confidence.aggregate.value, confidence.aggregate.is_calibrated
        );
        body.push_str("│\n");
        if policy.requires_ratify {
            body.push_str("│ ✋ requires ratification (call .propose())\n");
        } else {
            body.push_str("│ → ready to .commit() directly\n");
        }
        self.bottom(&mut body);
        HudFrame::new(HudFrameKind::Ready, body)
    }

    // --- Proposed ----------------------------------------------------------

    /// Render a `Proposed` directive — awaiting user ratification.
    pub fn render_proposed(&self, d: &Directive<Proposed>) -> HudFrame {
        let mut body = String::with_capacity(self.width * 8);
        self.title(
            &mut body,
            "PROPOSED",
            &format!("act: {}", d.act.act.id.as_str()),
        );
        let policy = d.policy.as_ref().expect("Proposed has policy");
        body.push_str(&render_policy_lines(policy));
        if let Some(window) = policy.ratify_window_ms {
            let _ = writeln!(body, "│ ratify-window: {} ms", window);
        }
        body.push_str("│\n");
        body.push_str("│ [Enter] approve   [Esc] reject   [a] amend\n");
        self.bottom(&mut body);
        HudFrame::new(HudFrameKind::Proposed, body)
    }

    // --- Committed ---------------------------------------------------------

    /// Render a `Committed` directive — dispatched, awaiting outcome.
    pub fn render_committed(&self, d: &Directive<Committed>) -> HudFrame {
        let mut body = String::with_capacity(self.width * 8);
        self.title(
            &mut body,
            "COMMITTED",
            &format!("act: {}", d.act.act.id.as_str()),
        );
        let _ = writeln!(body, "│ directive-id: {}", d.id.into_inner());
        if let Some(at) = d.committed_at {
            // Drop sub-second precision for HUD readability; the journal
            // keeps full nanosecond fidelity.
            let _ = writeln!(
                body,
                "│ committed-at: {}",
                at.format("%Y-%m-%d %H:%M:%S UTC")
            );
        }
        body.push_str("│ ⚡ dispatched\n");
        self.bottom(&mut body);
        HudFrame::new(HudFrameKind::Committed, body)
    }

    // --- Outcome -----------------------------------------------------------

    /// Render a successful `ExecutionOutcome` from the bridge.
    pub fn render_outcome(&self, outcome: &ExecutionOutcome) -> HudFrame {
        let mut body = String::with_capacity(self.width * 8);
        self.title(
            &mut body,
            "DONE",
            &format!("act: {}", outcome.act_id.as_str()),
        );
        let _ = writeln!(body, "│ result: {}", short_json(&outcome.result));
        // `ReverseAction` is `#[non_exhaustive]`; future variants get a
        // generic "press [u] to undo" message.
        let undo_label = match &outcome.reverse_action {
            ReverseAction::None => "no reverse needed".to_owned(),
            ReverseAction::RenameBack { .. } => "press [u] to rename back".to_owned(),
            ReverseAction::DeleteCopy { .. } => "press [u] to delete the copy".to_owned(),
            ReverseAction::RestoreContent { .. } => "press [u] to restore prior content".to_owned(),
            _ => "press [u] to undo".to_owned(),
        };
        let _ = writeln!(body, "│ undo: {}", undo_label);
        self.bottom(&mut body);
        HudFrame::new(HudFrameKind::Outcome, body)
    }

    /// Render the response of an Arcan (agent-runtime) execution.
    ///
    /// Takes primitive args rather than an `ArcanOutcome` struct so
    /// `pneuma-hud` stays free of a dependency on
    /// `pneuma-arcan-bridge`. The demo crate (which depends on both)
    /// extracts the fields and passes them here.
    ///
    /// `response` is rendered as a multi-line block with line breaks
    /// preserved, prefixed with `│ ` for HUD frame consistency.
    pub fn render_arcan_outcome(
        &self,
        act_id: &str,
        executor: &str,
        response: &str,
        exit_code: i32,
    ) -> HudFrame {
        let mut body = String::with_capacity(self.width * 16);
        self.title(&mut body, "AGENT DONE", &format!("act: {act_id}"));
        let _ = writeln!(body, "│ executor: {executor}  exit_code: {exit_code}");
        body.push_str("│\n│ response:\n");
        for line in response.lines() {
            let _ = writeln!(body, "│   {line}");
        }
        if response.is_empty() {
            body.push_str("│   (empty response from agent)\n");
        }
        body.push_str("│\n│ no reverse — agent responses are recorded, not undone\n");
        self.bottom(&mut body);
        HudFrame::new(HudFrameKind::Outcome, body)
    }

    // --- Errors ------------------------------------------------------------

    /// Render a contract-level error (pre-dispatch).
    pub fn render_contract_error(&self, err: &ContractError) -> HudFrame {
        let mut body = String::with_capacity(self.width * 4);
        self.title(&mut body, "CONTRACT ERROR", "");
        let _ = writeln!(body, "│ {}", err);
        body.push_str("│\n│ The directive cannot finalize. Amend slots or threshold.\n");
        self.bottom(&mut body);
        HudFrame::new(HudFrameKind::ContractError, body)
    }

    /// Render an executor-level error (during dispatch).
    pub fn render_praxis_error(&self, err: &PraxisError) -> HudFrame {
        let mut body = String::with_capacity(self.width * 4);
        self.title(&mut body, "EXECUTOR ERROR", "");
        let _ = writeln!(body, "│ {}", err);
        body.push_str("│\n│ Dispatch failed. Journal records the error.\n");
        self.bottom(&mut body);
        HudFrame::new(HudFrameKind::PraxisError, body)
    }

    /// Render a free-form info/status frame.
    pub fn render_info(&self, label: &str, body_text: &str) -> HudFrame {
        let mut body = String::with_capacity(self.width * 4);
        self.title(&mut body, label, "");
        for line in body_text.lines() {
            let _ = writeln!(body, "│ {}", line);
        }
        self.bottom(&mut body);
        HudFrame::new(HudFrameKind::Info, body)
    }

    // --- Frame chrome ------------------------------------------------------

    fn title(&self, out: &mut String, label: &str, subtitle: &str) {
        // Top border with embedded label.
        let label_part = format!("┤ {} ", label);
        let subtitle_part = if subtitle.is_empty() {
            String::new()
        } else {
            format!("│ {}", subtitle)
        };
        let top_inner = "─".repeat(self.width.saturating_sub(label_part.chars().count() + 1));
        let _ = writeln!(out, "┌{}{}", label_part, top_inner);
        if !subtitle.is_empty() {
            let _ = writeln!(out, "{}", subtitle_part);
        }
    }

    fn bottom(&self, out: &mut String) {
        let line = "─".repeat(self.width.saturating_sub(1));
        let _ = writeln!(out, "└{}", line);
    }
}

// --- Slot / modifier rendering --------------------------------------------

fn render_slot_value(v: &ResolvedSlotValue) -> String {
    match v {
        ResolvedSlotValue::Referent(rv) => render_referent(rv),
        ResolvedSlotValue::String(s) => format!("\"{}\"", truncate_for_hud(s, 50)),
        ResolvedSlotValue::Number(n) => format!("{}", n),
        ResolvedSlotValue::Boolean(b) => format!("{}", b),
    }
}

fn render_referent(rv: &pneuma_core::ReferentValue) -> String {
    use pneuma_core::ReferentValue as RV;
    match rv {
        RV::File(f) => format!("File({})", f.path.display()),
        RV::Selection(s) => format!(
            "Selection({} [{}..{}])",
            s.file.path.display(),
            s.span.start,
            s.span.end
        ),
        RV::Window(w) => format!("Window({})", w.as_str()),
        RV::App(a) => format!("App({})", a.as_str()),
        RV::Symbol(s) => format!("Symbol({})", s.qualified_name),
        RV::Url(u) => format!("Url({})", truncate_for_hud(u, 50)),
        RV::Set(items) => format!("Set[len={}]", items.len()),
        RV::Range { .. } => "Range(…)".to_owned(),
        RV::Anaphor(a) => format!("Anaphor(\"{}\")", a.surface),
        RV::Locus(_) => "Locus(…)".to_owned(),
    }
}

fn render_modifier(m: &Modifier) -> String {
    // `Modifier` is `#[non_exhaustive]`. Future variants render as
    // their Debug form until we add explicit cases.
    match m {
        Modifier::Magnitude(v) => format!("magnitude={:.2}", v),
        Modifier::Carefulness(v) => format!("carefulness={:.2}", v),
        Modifier::Urgency(v) => format!("urgency={:.2}", v),
        Modifier::Commitment(v) => format!("commitment={:.2}", v),
        Modifier::AbstractionLevel(v) => format!("abstraction={:.2}", v),
        Modifier::Distributive => "distributive".to_owned(),
        Modifier::Negation => "negation".to_owned(),
        Modifier::TimeWindow(_) => "time-window".to_owned(),
        Modifier::Custom { kind, .. } => format!("custom[{}]", kind),
        other => format!("{other:?}"),
    }
}

fn render_policy_lines(p: &PolicyEnvelope) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "│ policy: {:?} / blast: {:?} / threshold ≥ {:.3}",
        p.reversibility, p.blast_radius, p.min_confidence
    );
    if p.tightened_by_user || p.tightened_by_state {
        let by = match (p.tightened_by_user, p.tightened_by_state) {
            (true, true) => "user+state",
            (true, false) => "user",
            (false, true) => "state",
            (false, false) => "",
        };
        let _ = writeln!(out, "│ tightened-by: {}", by);
    }
    out
}

fn truncate_for_hud(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let truncated: String = s.chars().take(max - 1).collect();
        format!("{}…", truncated)
    }
}

fn short_json(v: &serde_json::Value) -> String {
    // 240 chars is enough for typical file.rename / file.copy results
    // (two paths) without forcing the user to read the journal.
    let s = serde_json::to_string(v).unwrap_or_default();
    truncate_for_hud(&s, 240)
}
