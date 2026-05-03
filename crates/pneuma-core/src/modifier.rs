//! Modifiers — qualifiers attached to a directive.
//!
//! From `MIL-PROJECT.md` §6.1:
//!
//! ```text
//! enum Modifier {
//!     Magnitude(f32), Carefulness(f32), Urgency(f32), Commitment(f32),
//!     AbstractionLevel(f32),
//!     Distributive, Negation,
//!     TimeWindow(TimeWindowSpec),
//!     Custom { kind: String, payload: Json },
//! }
//! ```
//!
//! ## Asymmetry — urgency does *not* lower confidence thresholds
//!
//! From §10.2:
//!
//! > Urgency does not lower the confidence threshold. Only shortens
//! > ratify dwell. This asymmetry is tested directly.
//!
//! [`PolicyEnvelope::tighten_by_carefulness`][tc] tightens
//! `min_confidence`; [`PolicyEnvelope::loosen_by_urgency`][lu] only
//! shortens `ratify_window_ms`. Both operators read from these
//! modifiers.
//!
//! [tc]: crate::policy::PolicyEnvelope::tighten_by_carefulness
//! [lu]: crate::policy::PolicyEnvelope::loosen_by_urgency

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{ContractError, Result};

// --- TimeWindowSpec ----------------------------------------------------------

/// A time window for `TimeWindow` modifiers — "do this for the next
/// hour", "schedule for tomorrow at 9".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeWindowSpec {
    /// Window start (inclusive).
    pub start: DateTime<Utc>,
    /// Window end (exclusive). Must be `>= start`.
    pub end: DateTime<Utc>,
}

impl TimeWindowSpec {
    /// Construct, rejecting `end < start`.
    pub fn new(start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Self> {
        if end < start {
            // For diagnostic display only; clamp to non-negative and
            // tag explicitly as a lossy diagnostic conversion.
            #[allow(clippy::cast_sign_loss)]
            let start_ms = start.timestamp_millis().max(0) as u64;
            #[allow(clippy::cast_sign_loss)]
            let end_ms = end.timestamp_millis().max(0) as u64;
            return Err(ContractError::InvalidSpan { start: start_ms, end: end_ms });
        }
        Ok(Self { start, end })
    }
}

// --- ModifierKind ------------------------------------------------------------

/// The discriminant of a [`Modifier`], for type-driven routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ModifierKind {
    /// "How much" — magnitude scalar.
    Magnitude,
    /// "How carefully" — tightens confidence threshold.
    Carefulness,
    /// "How urgently" — shortens ratify dwell only (asymmetry).
    Urgency,
    /// "How committed" — pinch tension / explicit confirmation level.
    Commitment,
    /// Abstraction level (gesture height vs low).
    AbstractionLevel,
    /// "Apply distributively" — to each item of a Set referent.
    Distributive,
    /// Negate the act ("don't rename it").
    Negation,
    /// Time-bounded scope.
    TimeWindow,
    /// User-supplied custom modifier.
    Custom,
}

// --- Modifier ----------------------------------------------------------------

/// A qualifier on a directive.
///
/// Scalar variants carry values normalized to `[0.0, 1.0]`. Construction
/// validates the range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Modifier {
    /// "How much", `[0.0, 1.0]`.
    Magnitude(f32),
    /// "How carefully", `[0.0, 1.0]`. Tightens confidence threshold.
    Carefulness(f32),
    /// "How urgently", `[0.0, 1.0]`. **Does not lower threshold**;
    /// only shortens ratify dwell.
    Urgency(f32),
    /// "How committed", `[0.0, 1.0]`. Pinch tension / hold strength.
    Commitment(f32),
    /// Abstraction level, `[0.0, 1.0]`.
    AbstractionLevel(f32),
    /// Apply distributively to each item of a Set referent.
    Distributive,
    /// Negate the act.
    Negation,
    /// Time-bounded scope.
    TimeWindow(TimeWindowSpec),
    /// User-supplied custom modifier.
    Custom {
        /// Vendor-specific kind tag.
        kind: String,
        /// Free-form JSON payload.
        payload: serde_json::Value,
    },
}

impl Modifier {
    /// Construct a [`Modifier::Magnitude`], validating the scalar.
    pub fn magnitude(v: f32) -> Result<Self> {
        validate_normalized("Magnitude", v)?;
        Ok(Self::Magnitude(v))
    }

    /// Construct a [`Modifier::Carefulness`], validating.
    pub fn carefulness(v: f32) -> Result<Self> {
        validate_normalized("Carefulness", v)?;
        Ok(Self::Carefulness(v))
    }

    /// Construct a [`Modifier::Urgency`], validating.
    pub fn urgency(v: f32) -> Result<Self> {
        validate_normalized("Urgency", v)?;
        Ok(Self::Urgency(v))
    }

    /// Construct a [`Modifier::Commitment`], validating.
    pub fn commitment(v: f32) -> Result<Self> {
        validate_normalized("Commitment", v)?;
        Ok(Self::Commitment(v))
    }

    /// Construct a [`Modifier::AbstractionLevel`], validating.
    pub fn abstraction_level(v: f32) -> Result<Self> {
        validate_normalized("AbstractionLevel", v)?;
        Ok(Self::AbstractionLevel(v))
    }

    /// The discriminant of this modifier.
    #[must_use]
    pub fn kind(&self) -> ModifierKind {
        match self {
            Self::Magnitude(_) => ModifierKind::Magnitude,
            Self::Carefulness(_) => ModifierKind::Carefulness,
            Self::Urgency(_) => ModifierKind::Urgency,
            Self::Commitment(_) => ModifierKind::Commitment,
            Self::AbstractionLevel(_) => ModifierKind::AbstractionLevel,
            Self::Distributive => ModifierKind::Distributive,
            Self::Negation => ModifierKind::Negation,
            Self::TimeWindow(_) => ModifierKind::TimeWindow,
            Self::Custom { .. } => ModifierKind::Custom,
        }
    }

    /// Extract the scalar value if this is a scalar modifier.
    #[must_use]
    pub fn as_scalar(&self) -> Option<f32> {
        match self {
            Self::Magnitude(v)
            | Self::Carefulness(v)
            | Self::Urgency(v)
            | Self::Commitment(v)
            | Self::AbstractionLevel(v) => Some(*v),
            _ => None,
        }
    }
}

// --- Helpers -----------------------------------------------------------------

fn validate_normalized(field: &'static str, v: f32) -> Result<()> {
    if v.is_nan() || !(0.0..=1.0).contains(&v) {
        return Err(ContractError::NotNormalized { field, value: v });
    }
    Ok(())
}
