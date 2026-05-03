//! The error taxonomy for `pneuma-core`.
//!
//! Errors here represent **contract violations** — a slot the act
//! schema declared required wasn't bound, a referent's type didn't
//! match its slot, a confidence score fell below the policy
//! envelope's threshold, an irreversible directive bypassed
//! ratification.
//!
//! ## Why a closed enum
//!
//! The directive contract has a small, stable set of failure modes.
//! We use `thiserror` and avoid `Box<dyn Error>` so callers can match
//! on specific variants and so the wire format stays inspectable.

use thiserror::Error;

/// Contract violation.
///
/// `Eq` is intentionally not derived because [`ContractError::ConfidenceBelowThreshold`]
/// carries `f32` values (NaN is non-reflexive).
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum ContractError {
    /// A required slot was not bound by the time
    /// [`crate::Directive::try_finalize`] was called. Guarantee 1 of
    /// the five categorical guarantees.
    #[error("required slot {slot} for act {act_id} was not bound")]
    UnboundRequiredSlot {
        /// The act being finalized.
        act_id: String,
        /// The slot left unbound.
        slot: String,
    },

    /// A bound slot's referent value did not match the slot's
    /// declared type. Guarantee 2.
    #[error("slot {slot} expected referent type {expected:?} but got {actual:?}")]
    TypeMismatch {
        /// The slot whose type was violated.
        slot: String,
        /// The type the slot's signature declared.
        expected: crate::referent::ReferentType,
        /// The type the bound value carries.
        actual: crate::referent::ReferentType,
    },

    /// The directive's joined confidence fell below the policy
    /// envelope's `min_confidence` threshold. Guarantee 3.
    #[error(
        "confidence {confidence:.3} below policy threshold {threshold:.3} \
        (effective threshold after calibration penalty: {effective_threshold:.3})"
    )]
    ConfidenceBelowThreshold {
        /// The directive's joined confidence.
        confidence: f32,
        /// The policy envelope's nominal threshold.
        threshold: f32,
        /// The threshold after the calibration penalty was applied.
        effective_threshold: f32,
    },

    /// `commit()` was called on a `Ready` directive whose policy
    /// envelope required ratification. Guarantee 4.
    #[error("policy requires ratification; call propose() then ratify(), not commit()")]
    RatifyRequired,

    /// A directive in the wrong state was passed to a state-transition
    /// method. Should be unreachable due to typestate, but safety-net
    /// for runtime construction paths.
    #[error("invalid state transition from {from} to {to}")]
    InvalidTransition {
        /// The state the directive is in.
        from: &'static str,
        /// The state the caller attempted to transition to.
        to: &'static str,
    },

    /// A normalized field (confidence, threshold, modifier scalar)
    /// was outside `[0.0, 1.0]`.
    #[error("value {value} outside normalized [0.0, 1.0] domain for {field}")]
    NotNormalized {
        /// The struct field whose value was out of range.
        field: &'static str,
        /// The offending value.
        value: f32,
    },

    /// An ID or name field was empty or whitespace-only.
    #[error("empty or whitespace-only identifier for {field}")]
    EmptyIdentifier {
        /// The struct field whose value was empty.
        field: &'static str,
    },

    /// The directive's `valid_until` timestamp has passed; the
    /// directive can no longer be committed.
    #[error("directive expired at {expired_at}")]
    Expired {
        /// The wall-clock time at which the directive expired.
        expired_at: chrono::DateTime<chrono::Utc>,
    },

    /// A text span was constructed with `end < start`.
    #[error("invalid text span: end ({end}) < start ({start})")]
    InvalidSpan {
        /// Span start.
        start: u64,
        /// Span end.
        end: u64,
    },
}

/// Convenient `Result` alias for contract operations.
pub type Result<T, E = ContractError> = core::result::Result<T, E>;
