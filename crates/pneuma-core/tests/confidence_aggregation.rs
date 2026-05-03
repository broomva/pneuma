//! Confidence: calibration propagation, weakest-slot, calibration penalty.
//!
//! Properties (cross-references to `MIL-PROJECT.md` §10.2):
//!
//! - Aggregate confidence is the *minimum* per-slot score (conservative).
//! - Aggregate is calibrated only if every per-slot score is calibrated.
//! - `effective_value()` applies the 20% penalty to uncalibrated scores.

use pneuma_core::{Confidence, ConfidenceProducer, ConfidenceScore};
use pneuma_core::confidence::UNCALIBRATED_PENALTY;

fn cal(v: f32) -> ConfidenceScore {
    ConfidenceScore::new(v, true, ConfidenceProducer::Deterministic).unwrap()
}

fn uncal(v: f32) -> ConfidenceScore {
    ConfidenceScore::new(v, false, ConfidenceProducer::LlmLogprob).unwrap()
}

#[test]
fn aggregate_is_minimum_of_per_slot() {
    let conf = Confidence::from_slots(vec![
        ("a".to_owned(), cal(0.95)),
        ("b".to_owned(), cal(0.7)),
        ("c".to_owned(), cal(0.85)),
    ])
    .unwrap();
    assert!((conf.aggregate.value - 0.7).abs() < 1e-6);
}

#[test]
fn weakest_slot_returns_minimum() {
    let conf = Confidence::from_slots(vec![
        ("a".to_owned(), cal(0.95)),
        ("b".to_owned(), cal(0.6)),
        ("c".to_owned(), cal(0.85)),
    ])
    .unwrap();
    assert!((conf.weakest_slot() - 0.6).abs() < 1e-6);
}

#[test]
fn aggregate_calibrated_only_if_all_slots_calibrated() {
    let all_cal = Confidence::from_slots(vec![
        ("a".to_owned(), cal(0.9)),
        ("b".to_owned(), cal(0.85)),
    ])
    .unwrap();
    assert!(all_cal.is_calibrated());

    let mixed = Confidence::from_slots(vec![
        ("a".to_owned(), cal(0.9)),
        ("b".to_owned(), uncal(0.85)),
    ])
    .unwrap();
    assert!(!mixed.is_calibrated(), "any uncalibrated taints aggregate");
}

#[test]
fn calibration_penalty_applied_to_effective_value() {
    let s = uncal(0.9);
    let expected = 0.9 * (1.0 - UNCALIBRATED_PENALTY); // 0.72
    assert!((s.effective_value() - expected).abs() < 1e-6);
}

#[test]
fn calibrated_score_effective_value_unchanged() {
    let s = cal(0.9);
    // Calibrated path is a no-op; the value is identity-preserved.
    // Use a tiny epsilon to satisfy clippy::float_cmp without
    // weakening the assertion meaningfully.
    assert!((s.effective_value() - 0.9).abs() < 1e-6);
}

#[test]
fn out_of_range_score_rejected() {
    assert!(ConfidenceScore::new(1.5, true, ConfidenceProducer::Deterministic).is_err());
    assert!(ConfidenceScore::new(-0.1, true, ConfidenceProducer::Deterministic).is_err());
    assert!(ConfidenceScore::new(f32::NAN, true, ConfidenceProducer::Deterministic).is_err());
}

#[test]
fn empty_slots_gives_synthetic_perfect_confidence() {
    // No slots — aggregate is 1.0 calibrated synthetic. Policy
    // threshold gates still fire normally because the *effective*
    // value is what's compared.
    let conf = Confidence::from_slots(vec![]).unwrap();
    assert!((conf.aggregate.value - 1.0).abs() < 1e-6);
    assert!(conf.is_calibrated());
}
