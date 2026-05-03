//! Policy envelope: intrinsic threshold table, asymmetry of tightening
//! vs urgency, calibration penalty.
//!
//! Properties (cross-references to `MIL-PROJECT.md` §10.2):
//!
//! - Intrinsic threshold table is monotone in both reversibility and
//!   blast radius.
//! - `tighten_by_carefulness` raises `min_confidence`.
//! - `tighten_by_state` raises `min_confidence`.
//! - `loosen_by_urgency` shortens `ratify_window_ms`. **Never** touches
//!   `min_confidence`.
//! - Effective threshold under uncalibrated input is *higher* than
//!   under calibrated input — uncalibrated evidence is held to a
//!   stricter bar.

use pneuma_core::{BlastRadius, PolicyEnvelope, Reversibility};

#[test]
fn intrinsic_threshold_monotone_in_reversibility() {
    let free = PolicyEnvelope::intrinsic(Reversibility::Free, BlastRadius::Local).min_confidence;
    let costly =
        PolicyEnvelope::intrinsic(Reversibility::Costly, BlastRadius::Local).min_confidence;
    let irrev =
        PolicyEnvelope::intrinsic(Reversibility::Irreversible, BlastRadius::Local).min_confidence;
    assert!(free < costly, "Costly tighter than Free");
    assert!(costly < irrev, "Irreversible tightest");
}

#[test]
fn intrinsic_threshold_monotone_in_blast() {
    let local = PolicyEnvelope::intrinsic(Reversibility::Free, BlastRadius::Local).min_confidence;
    let project =
        PolicyEnvelope::intrinsic(Reversibility::Free, BlastRadius::Project).min_confidence;
    let user = PolicyEnvelope::intrinsic(Reversibility::Free, BlastRadius::User).min_confidence;
    let system = PolicyEnvelope::intrinsic(Reversibility::Free, BlastRadius::System).min_confidence;
    let external =
        PolicyEnvelope::intrinsic(Reversibility::Free, BlastRadius::External).min_confidence;
    assert!(local < project);
    assert!(project < user);
    assert!(user < system);
    assert!(system < external);
}

#[test]
fn intrinsic_threshold_table_matches_spec() {
    // From PolicyEnvelope::intrinsic doc table.
    use BlastRadius::{External, Local, Project, System, User};
    use Reversibility::{Costly, Free, Irreversible};

    let cases = [
        (Free, Local, 0.55),
        (Free, Project, 0.60),
        (Free, User, 0.65),
        (Free, System, 0.70),
        (Free, External, 0.75),
        (Costly, Local, 0.65),
        (Costly, Project, 0.70),
        (Costly, User, 0.75),
        (Costly, System, 0.80),
        (Costly, External, 0.85),
        (Irreversible, Local, 0.85),
        (Irreversible, Project, 0.88),
        (Irreversible, User, 0.90),
        (Irreversible, System, 0.92),
        (Irreversible, External, 0.95),
    ];
    for (rev, blast, expected) in cases {
        let p = PolicyEnvelope::intrinsic(rev, blast);
        assert!(
            (p.min_confidence - expected).abs() < 1e-6,
            "({rev:?}, {blast:?}) expected {expected}, got {}",
            p.min_confidence
        );
    }
}

#[test]
fn ratify_required_when_irreversible_or_blast_user_plus() {
    use BlastRadius::{External, Local, Project, System, User};
    use Reversibility::{Costly, Free, Irreversible};

    // Free + Local/Project: no ratify.
    assert!(!PolicyEnvelope::intrinsic(Free, Local).requires_ratify);
    assert!(!PolicyEnvelope::intrinsic(Free, Project).requires_ratify);
    // Costly + Local/Project: no ratify.
    assert!(!PolicyEnvelope::intrinsic(Costly, Local).requires_ratify);
    assert!(!PolicyEnvelope::intrinsic(Costly, Project).requires_ratify);
    // User+ blast OR Irreversible: requires ratify.
    assert!(PolicyEnvelope::intrinsic(Free, User).requires_ratify);
    assert!(PolicyEnvelope::intrinsic(Free, System).requires_ratify);
    assert!(PolicyEnvelope::intrinsic(Free, External).requires_ratify);
    assert!(PolicyEnvelope::intrinsic(Costly, User).requires_ratify);
    assert!(PolicyEnvelope::intrinsic(Irreversible, Local).requires_ratify);
    assert!(PolicyEnvelope::intrinsic(Irreversible, External).requires_ratify);
}

#[test]
fn tighten_by_carefulness_raises_threshold() {
    let mut p = PolicyEnvelope::intrinsic(Reversibility::Free, BlastRadius::Local);
    let baseline = p.min_confidence; // 0.55
    p.tighten_by_carefulness(0.5).unwrap();
    assert!(
        p.min_confidence > baseline,
        "carefulness must raise threshold"
    );
    assert!(p.tightened_by_user);
}

#[test]
fn tighten_by_state_raises_threshold_less_aggressively_than_carefulness() {
    let mut by_user = PolicyEnvelope::intrinsic(Reversibility::Free, BlastRadius::Local);
    by_user.tighten_by_carefulness(0.8).unwrap();

    let mut by_state = PolicyEnvelope::intrinsic(Reversibility::Free, BlastRadius::Local);
    by_state.tighten_by_state(0.8).unwrap();

    assert!(
        by_user.min_confidence > by_state.min_confidence,
        "carefulness lifts more than state at the same intensity"
    );
    assert!(by_state.tightened_by_state);
    assert!(!by_state.tightened_by_user);
}

// --- THE ASYMMETRY: urgency does NOT lower the threshold ------------------

#[test]
fn urgency_does_not_lower_threshold() {
    let mut p = PolicyEnvelope::intrinsic(Reversibility::Irreversible, BlastRadius::Project);
    let baseline = p.min_confidence; // 0.88
    p.loosen_by_urgency(1.0).unwrap();
    // Bit-equality check via tiny epsilon — the spec property is that
    // urgency *never* moves the threshold.
    assert!(
        (p.min_confidence - baseline).abs() < f32::EPSILON,
        "urgency must not change min_confidence — this is the asymmetry"
    );
}

#[test]
fn urgency_shortens_ratify_window_only() {
    let mut p = PolicyEnvelope::intrinsic(Reversibility::Irreversible, BlastRadius::Project);
    let baseline_window = p.ratify_window_ms.unwrap(); // 800ms intrinsic
    p.loosen_by_urgency(1.0).unwrap();
    let new_window = p.ratify_window_ms.unwrap();
    assert!(
        new_window < baseline_window,
        "urgency 1.0 must shorten dwell"
    );
    assert!(new_window >= 200, "dwell clamps to 200ms minimum");
}

#[test]
fn urgency_zero_is_a_no_op() {
    let mut p = PolicyEnvelope::intrinsic(Reversibility::Irreversible, BlastRadius::Project);
    let baseline_window = p.ratify_window_ms.unwrap();
    p.loosen_by_urgency(0.0).unwrap();
    assert_eq!(p.ratify_window_ms.unwrap(), baseline_window);
}

#[test]
fn urgency_on_no_ratify_envelope_is_silent() {
    // Local-Free has no ratify window — urgency should not panic.
    let mut p = PolicyEnvelope::intrinsic(Reversibility::Free, BlastRadius::Local);
    assert!(p.ratify_window_ms.is_none());
    p.loosen_by_urgency(1.0).unwrap();
    assert!(p.ratify_window_ms.is_none(), "no window to shorten");
}

#[test]
fn out_of_range_carefulness_rejected() {
    let mut p = PolicyEnvelope::intrinsic(Reversibility::Free, BlastRadius::Local);
    assert!(p.tighten_by_carefulness(1.5).is_err());
    assert!(p.tighten_by_carefulness(-0.1).is_err());
    assert!(p.tighten_by_carefulness(f32::NAN).is_err());
}

// --- Effective threshold under calibration ---------------------------------

#[test]
fn uncalibrated_evidence_faces_higher_effective_threshold() {
    let p = PolicyEnvelope::intrinsic(Reversibility::Free, BlastRadius::Local);
    let calibrated_thr = p.effective_threshold(true);
    let uncalibrated_thr = p.effective_threshold(false);
    assert!(uncalibrated_thr > calibrated_thr);
    assert!((calibrated_thr - p.min_confidence).abs() < f32::EPSILON);
    // Effective threshold is min_confidence / (1 - 0.20) = min_confidence / 0.8
    assert!((uncalibrated_thr - p.min_confidence / 0.8).abs() < 1e-6);
}

#[test]
fn effective_threshold_clamps_at_one() {
    let p = PolicyEnvelope::intrinsic(Reversibility::Irreversible, BlastRadius::External);
    // 0.95 / 0.8 = 1.1875, must clamp to 1.0.
    assert!((p.effective_threshold(false) - 1.0).abs() < f32::EPSILON);
}
