//! Confidence — calibrated, per-slot decomposable.
//!
//! From `MIL-PROJECT.md` §10.2:
//!
//! > Calibration is explicit. `ConfidenceScore` carries `is_calibrated:
//! > bool` and `producer: ConfidenceProducer`. Uncalibrated scores get
//! > a 20% effective penalty against thresholds.
//!
//! The substrate (`sensorium-core`) reports calibration honestly at the
//! producer level. `pneuma-core` consumes that signal via
//! [`ConfidenceProducer`] and applies the [`UNCALIBRATED_PENALTY`]
//! when computing the effective threshold for the policy envelope.
//!
//! ## Joining confidence across slots
//!
//! [`Confidence::weakest_slot`] returns the minimum per-slot score.
//! That's the conservative join — a directive is only as confident as
//! its weakest binding. This composes correctly across binding kinds
//! (a 0.95 model interpretation joined with a 0.7 anaphora is 0.7).

use serde::{Deserialize, Serialize};

use crate::error::{ContractError, Result};

/// The penalty applied to the effective confidence threshold when the
/// score is not declared calibrated. From `MIL-PROJECT.md` §10.2: 20%.
///
/// Concretely: `effective_threshold = threshold * (1 - UNCALIBRATED_PENALTY)`
/// when `is_calibrated == false`, so a 0.9 nominal threshold against
/// uncalibrated evidence requires 0.72 confidence to clear. The
/// asymmetry: uncalibrated evidence is held to a *stricter* bar, not
/// a more lenient one.
pub const UNCALIBRATED_PENALTY: f32 = 0.20;

// --- ConfidenceProducer ------------------------------------------------------

/// Where a confidence score came from. Used for diagnostics and
/// (eventually) per-producer calibration history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ConfidenceProducer {
    /// Deterministic match — gaze hit-test, exact lookup. Score is 1.0
    /// or near-1.0; declared calibrated.
    Deterministic,
    /// Substrate (sensorium) sensor — typically a per-sample
    /// confidence from the producer's own model.
    SubstrateSensor,
    /// Anaphora resolver — score depends on disambiguator strength.
    AnaphoraResolver,
    /// Cross-modal binder — score depends on temporal binding window
    /// fitness.
    CrossModalBinder,
    /// LLM logprob proxy — calibration is empirically poor; almost
    /// always reported as uncalibrated.
    LlmLogprob,
    /// Retrieval similarity — embedding cosine score.
    RetrievalSimilarity,
    /// Aggregate / joined score from multiple producers.
    Aggregate,
    /// Test fixture or synthetic value.
    Synthetic,
}

// --- ConfidenceScore ---------------------------------------------------------

/// A single confidence measurement.
///
/// Scalars are in `[0.0, 1.0]`. `is_calibrated` is honest — producers
/// must not claim calibration to bypass the penalty.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceScore {
    /// The score, `[0.0, 1.0]`.
    pub value: f32,
    /// `true` if the producer has demonstrated calibrated logprobs;
    /// `false` is the conservative default.
    pub is_calibrated: bool,
    /// Where the score came from.
    pub producer: ConfidenceProducer,
}

impl ConfidenceScore {
    /// Construct, validating the scalar.
    pub fn new(value: f32, is_calibrated: bool, producer: ConfidenceProducer) -> Result<Self> {
        if value.is_nan() || !(0.0..=1.0).contains(&value) {
            return Err(ContractError::NotNormalized {
                field: "ConfidenceScore.value",
                value,
            });
        }
        Ok(Self {
            value,
            is_calibrated,
            producer,
        })
    }

    /// Apply the calibration penalty: if uncalibrated, drop the score
    /// by [`UNCALIBRATED_PENALTY`] (multiplicatively). Used internally
    /// when comparing against thresholds.
    #[must_use]
    pub fn effective_value(&self) -> f32 {
        if self.is_calibrated {
            self.value
        } else {
            self.value * (1.0 - UNCALIBRATED_PENALTY)
        }
    }
}

// --- Confidence (per-slot decomposed) ----------------------------------------

/// A directive's confidence — joined across all bound slots.
///
/// The substrate's confidence join rule is *minimum across slots*. A
/// directive is only as confident as its weakest binding. This is the
/// conservative composition that downstream code can rely on without
/// assuming a particular probabilistic model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Confidence {
    /// Per-slot scores, indexed by slot name. Empty for directives
    /// with no slots (rare).
    pub per_slot: Vec<(String, ConfidenceScore)>,
    /// Aggregate score (default: minimum of `per_slot`). May diverge
    /// if a producer has more sophisticated joining logic.
    pub aggregate: ConfidenceScore,
}

impl Confidence {
    /// Construct from a list of per-slot scores. The aggregate is
    /// computed as the [`Self::weakest_slot`] score, marked
    /// uncalibrated if any input is uncalibrated, and attributed to
    /// [`ConfidenceProducer::Aggregate`].
    pub fn from_slots(per_slot: Vec<(String, ConfidenceScore)>) -> Result<Self> {
        if per_slot.is_empty() {
            // Empty directive (no slots): perfect confidence by
            // vacuous truth, but mark as Synthetic so policy threshold
            // gates still fire. Most acts have at least one slot.
            return Ok(Self {
                per_slot,
                aggregate: ConfidenceScore::new(1.0, true, ConfidenceProducer::Synthetic)?,
            });
        }

        let weakest = per_slot
            .iter()
            .map(|(_, s)| s.value)
            .fold(f32::INFINITY, f32::min);
        let all_calibrated = per_slot.iter().all(|(_, s)| s.is_calibrated);

        Ok(Self {
            aggregate: ConfidenceScore::new(
                weakest,
                all_calibrated,
                ConfidenceProducer::Aggregate,
            )?,
            per_slot,
        })
    }

    /// Construct directly from an aggregate score. Used when the
    /// producer wants to override the default minimum-join.
    #[must_use]
    pub fn from_aggregate(aggregate: ConfidenceScore) -> Self {
        Self {
            per_slot: Vec::new(),
            aggregate,
        }
    }

    /// The weakest per-slot score, if any. Returns the aggregate's
    /// value when there are no per-slot scores.
    #[must_use]
    pub fn weakest_slot(&self) -> f32 {
        self.per_slot
            .iter()
            .map(|(_, s)| s.value)
            .fold(f32::INFINITY, f32::min)
            .min(self.aggregate.value)
    }

    /// `true` if the aggregate score is calibrated.
    #[must_use]
    pub fn is_calibrated(&self) -> bool {
        self.aggregate.is_calibrated
    }

    /// Effective aggregate value, with calibration penalty applied if
    /// uncalibrated. Pneuma's policy gate uses this against
    /// `min_confidence`.
    #[must_use]
    pub fn effective_value(&self) -> f32 {
        self.aggregate.effective_value()
    }
}
