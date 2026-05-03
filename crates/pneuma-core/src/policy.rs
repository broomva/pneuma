//! Policy envelope — the safety contract attached to every directive.
//!
//! From `MIL-PROJECT.md` §6.1:
//!
//! ```text
//! struct PolicyEnvelope {
//!     reversibility: Reversibility,
//!     blast_radius: BlastRadius,
//!     requires_ratify: bool,
//!     ratify_window_ms: Option<u32>,
//!     min_confidence: f32,
//!     valid_until: Option<Timestamp>,
//!     permitted_executors: Vec<ExecutorKind>,
//!     redactions: Vec<RedactionRule>,
//!     tightened_by_user: bool,
//!     tightened_by_state: bool,
//! }
//! ```
//!
//! ## The threshold table (`intrinsic`)
//!
//! From §10.2:
//!
//! > `PolicyEnvelope::intrinsic()` encodes the threshold table
//! > explicitly. Irreversible → 0.9 min confidence, free local → 0.55,
//! > etc.
//!
//! [`PolicyEnvelope::intrinsic`] computes the *baseline* envelope from
//! the act's `(reversibility, blast_radius)`. Modifiers and observed
//! state then *tighten* (carefulness, fatigue, high arousal) or
//! *adjust dwell* (urgency). Tightening ratchets — there is no
//! "loosen by user" path.
//!
//! ## The asymmetry
//!
//! - [`PolicyEnvelope::tighten_by_carefulness`] raises `min_confidence`.
//! - [`PolicyEnvelope::tighten_by_state`] raises `min_confidence`.
//! - [`PolicyEnvelope::loosen_by_urgency`] **only** shortens
//!   `ratify_window_ms`; never touches `min_confidence`.
//!
//! From §10.2:
//!
//! > Urgency does not lower the confidence threshold. Only shortens
//! > ratify dwell. This asymmetry is tested directly.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{ContractError, Result};

/// Re-export of `chrono::DateTime<Utc>` for the contract's wire format.
pub type Timestamp = DateTime<Utc>;

// --- Reversibility -----------------------------------------------------------

/// How easily a dispatched act can be undone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Reversibility {
    /// Free to undo — no side effects, or fully captured by the
    /// reverse-action recipe. Lowest threshold.
    Free,
    /// Costly to undo — reversal works but takes time / money / user
    /// attention. Mid threshold.
    Costly,
    /// Cannot be undone (delete, send-message, payment). Highest
    /// threshold; ratification required.
    Irreversible,
}

// --- BlastRadius -------------------------------------------------------------

/// What scope a dispatched act affects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BlastRadius {
    /// Affects only local in-process state (cursor, focus).
    Local,
    /// Affects the current project (file change, code edit).
    Project,
    /// Affects user-wide state (preferences, files outside the
    /// project).
    User,
    /// Affects system-wide state (settings, install).
    System,
    /// External-facing (network call, payment, send message).
    /// Highest blast.
    External,
}

// --- ExecutorKind ------------------------------------------------------------

/// Which downstream executor a directive may dispatch to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ExecutorKind {
    /// Praxis — deterministic OS-level execution (file ops, app
    /// control, system calls).
    Praxis,
    /// Arcan — agent runtime (natural-language tasks, LLM-bound
    /// work).
    Arcan,
    /// Spaces — multi-agent protocol-level dispatch.
    Spaces,
    /// Custom executor — pluggable. Free-form name.
    Custom,
}

// --- RedactionRule -----------------------------------------------------------

/// A redaction rule applied when this directive is journaled or
/// shown to a remote agent.
///
/// Used by Anima to enforce the privacy manifest on directive content
/// (e.g. "redact the file path before forwarding to a remote agent").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RedactionRule {
    /// JSON path expression of the field to redact.
    pub path: String,
    /// Replacement value (or sentinel like `"<redacted>"`).
    pub replacement: String,
    /// Why the redaction applies (free-form for diagnostics).
    pub reason: String,
}

// --- PolicyEnvelope ----------------------------------------------------------

/// The safety contract for a directive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyEnvelope {
    /// How reversible this directive's act is.
    pub reversibility: Reversibility,
    /// What scope this directive affects.
    pub blast_radius: BlastRadius,
    /// `true` if the directive must transit through `Proposed` →
    /// `Committed` rather than commit directly.
    pub requires_ratify: bool,
    /// How long the user has to ratify, in milliseconds. Modulated by
    /// urgency.
    pub ratify_window_ms: Option<u32>,
    /// Minimum effective confidence required to commit. Tightened by
    /// carefulness / state; never loosened by urgency.
    pub min_confidence: f32,
    /// Wall-clock deadline after which the directive cannot commit.
    pub valid_until: Option<Timestamp>,
    /// Which executors are allowed to dispatch this directive.
    pub permitted_executors: Vec<ExecutorKind>,
    /// Redactions to apply when journaling or forwarding.
    pub redactions: Vec<RedactionRule>,
    /// `true` if user input (carefulness modifier) tightened this
    /// envelope.
    pub tightened_by_user: bool,
    /// `true` if observed user state (fatigue, high arousal,
    /// cognitive load) tightened this envelope.
    pub tightened_by_state: bool,
}

impl PolicyEnvelope {
    /// Compute the **intrinsic** envelope for an act's
    /// `(reversibility, blast_radius)` pair. This is the baseline
    /// that modifiers and state can tighten — but never loosen.
    ///
    /// The threshold table (`min_confidence` defaults):
    ///
    /// | Reversibility \ Blast | Local | Project | User | System | External |
    /// |-----------------------|-------|---------|------|--------|----------|
    /// | Free                  | 0.55  | 0.60    | 0.65 | 0.70   | 0.75     |
    /// | Costly                | 0.65  | 0.70    | 0.75 | 0.80   | 0.85     |
    /// | Irreversible          | 0.85  | 0.88    | 0.90 | 0.92   | 0.95     |
    ///
    /// Ratification follows naturally: anything Irreversible or with
    /// blast ≥ User requires ratification.
    ///
    /// `clippy::match_same_arms` is allowed: several entries
    /// coincidentally share a value (e.g. `(Free, External)` and
    /// `(Irreversible, Local)` are both 0.75/0.85), but they are
    /// distinct cases in the threshold table. Merging them with `|`
    /// would erase the table's structure.
    #[must_use]
    #[allow(clippy::match_same_arms)]
    pub fn intrinsic(reversibility: Reversibility, blast_radius: BlastRadius) -> Self {
        let min_confidence = match (reversibility, blast_radius) {
            (Reversibility::Free, BlastRadius::Local) => 0.55,
            (Reversibility::Free, BlastRadius::Project) => 0.60,
            (Reversibility::Free, BlastRadius::User) => 0.65,
            (Reversibility::Free, BlastRadius::System) => 0.70,
            (Reversibility::Free, BlastRadius::External) => 0.75,
            (Reversibility::Costly, BlastRadius::Local) => 0.65,
            (Reversibility::Costly, BlastRadius::Project) => 0.70,
            (Reversibility::Costly, BlastRadius::User) => 0.75,
            (Reversibility::Costly, BlastRadius::System) => 0.80,
            (Reversibility::Costly, BlastRadius::External) => 0.85,
            (Reversibility::Irreversible, BlastRadius::Local) => 0.85,
            (Reversibility::Irreversible, BlastRadius::Project) => 0.88,
            (Reversibility::Irreversible, BlastRadius::User) => 0.90,
            (Reversibility::Irreversible, BlastRadius::System) => 0.92,
            (Reversibility::Irreversible, BlastRadius::External) => 0.95,
        };

        let requires_ratify = matches!(reversibility, Reversibility::Irreversible)
            || matches!(blast_radius, BlastRadius::User | BlastRadius::System | BlastRadius::External);

        Self {
            reversibility,
            blast_radius,
            requires_ratify,
            ratify_window_ms: if requires_ratify { Some(800) } else { None },
            min_confidence,
            valid_until: None,
            permitted_executors: Vec::new(),
            redactions: Vec::new(),
            tightened_by_user: false,
            tightened_by_state: false,
        }
    }

    /// Tighten `min_confidence` by carefulness scalar in `[0.0, 1.0]`.
    ///
    /// Concretely: lifts the threshold toward 1.0 by
    /// `carefulness * (1 - threshold) * 0.5`. Carefulness `1.0` raises
    /// a `0.6` threshold to `~0.8`; carefulness `0.0` leaves it
    /// unchanged.
    pub fn tighten_by_carefulness(&mut self, carefulness: f32) -> Result<()> {
        validate_normalized("carefulness", carefulness)?;
        let lift = carefulness * (1.0 - self.min_confidence) * 0.5;
        self.min_confidence = (self.min_confidence + lift).min(1.0);
        self.tightened_by_user = true;
        Ok(())
    }

    /// Tighten by observed user state — fatigue, high arousal, high
    /// cognitive load. Same formula as carefulness but with a smaller
    /// max lift (0.3 of remaining headroom) so state tightening is
    /// less aggressive than explicit user carefulness.
    pub fn tighten_by_state(&mut self, intensity: f32) -> Result<()> {
        validate_normalized("intensity", intensity)?;
        let lift = intensity * (1.0 - self.min_confidence) * 0.3;
        self.min_confidence = (self.min_confidence + lift).min(1.0);
        self.tightened_by_state = true;
        Ok(())
    }

    /// **Asymmetric**: shortens `ratify_window_ms` by urgency in
    /// `[0.0, 1.0]`. Does NOT touch `min_confidence`.
    ///
    /// Concretely: scales the ratify window by `(1 - urgency * 0.7)`,
    /// clamped to a minimum of 200 ms. Urgency 1.0 collapses an
    /// 800 ms window to 240 ms (clamped to 240 since 0.3 × 800 = 240).
    pub fn loosen_by_urgency(&mut self, urgency: f32) -> Result<()> {
        validate_normalized("urgency", urgency)?;
        if let Some(window) = self.ratify_window_ms {
            #[allow(clippy::cast_precision_loss)] // u32 to f32 in safe range
            let scaled = (window as f32) * (1.0 - urgency * 0.7);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let clamped = scaled.max(200.0).round() as u32;
            self.ratify_window_ms = Some(clamped);
        }
        Ok(())
    }

    /// Effective threshold after applying the calibration penalty for
    /// an uncalibrated score. From §10.2: 20% penalty.
    #[must_use]
    pub fn effective_threshold(&self, is_calibrated: bool) -> f32 {
        if is_calibrated {
            self.min_confidence
        } else {
            // Make uncalibrated evidence harder to clear: divide
            // through (1 - penalty) so the *raw* score has to be
            // higher to clear the *nominal* threshold once penalty
            // is multiplied in.
            (self.min_confidence / (1.0 - crate::confidence::UNCALIBRATED_PENALTY)).min(1.0)
        }
    }
}

fn validate_normalized(field: &'static str, v: f32) -> Result<()> {
    if v.is_nan() || !(0.0..=1.0).contains(&v) {
        return Err(ContractError::NotNormalized {
            field: match field {
                "carefulness" => "carefulness",
                "urgency" => "urgency",
                "intensity" => "intensity",
                _ => "policy.scalar",
            },
            value: v,
        });
    }
    Ok(())
}
