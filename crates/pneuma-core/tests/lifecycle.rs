//! Directive lifecycle: typestate transitions enforce the state machine
//! at compile time; runtime validation gates `try_finalize`.
//!
//! Properties under test (cross-references to `MIL-PROJECT.md` §6.2):
//!
//! - Composing → Ready (via `try_finalize`): runtime-validated.
//! - Ready → Committed (no ratify) or Proposed (ratify required).
//! - Proposed → Committed (ratify) / Composing (amend) / cancelled.

use chrono::Utc;
use pneuma_core::{
    Act, ActId, ActPrimitive, Arity, BindingKind, BlastRadius, Confidence, ConfidenceProducer,
    ConfidenceScore, ContextRef, ContextSnapshotId, Directive, DirectiveState, ExecutorHint,
    FileRef, PolicyEnvelope, Provenance, ReferentType, ReferentValue, ResolvedAct, ResolvedSlot,
    Reversibility, SlotKind, SlotSignature, SpeechAct,
};
use pneuma_core::act::ResolvedSlotValue;

/// Construct a tiny `file.read` act for tests — `file: File` required,
/// reversible (read has no side effects).
fn read_act() -> Act {
    Act {
        id: ActId::new("file.read").unwrap(),
        primitive: ActPrimitive::Custom,
        slots: vec![
            SlotSignature::new("file", SlotKind::Referent(ReferentType::File), Arity::Required)
                .unwrap(),
        ],
        reversibility: Reversibility::Free,
        blast_radius: BlastRadius::Local,
        executor_hint: ExecutorHint::Praxis,
        reverse_recipe: None,
        description: None,
    }
}

/// Construct a `file.delete` act — irreversible, requires ratification.
fn delete_act() -> Act {
    Act {
        id: ActId::new("file.delete").unwrap(),
        primitive: ActPrimitive::Custom,
        slots: vec![
            SlotSignature::new("file", SlotKind::Referent(ReferentType::File), Arity::Required)
                .unwrap(),
        ],
        reversibility: Reversibility::Irreversible,
        blast_radius: BlastRadius::Project,
        executor_hint: ExecutorHint::Praxis,
        reverse_recipe: None,
        description: None,
    }
}

fn calibrated_score(value: f32) -> ConfidenceScore {
    ConfidenceScore::new(value, true, ConfidenceProducer::Deterministic).unwrap()
}

fn deterministic_provenance() -> Provenance {
    Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now())
}

fn bound_file_slot(name: &str, path: &str) -> ResolvedSlot {
    ResolvedSlot::new(
        name,
        ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new(path))),
        deterministic_provenance(),
    )
    .unwrap()
}

fn fresh_context() -> ContextRef {
    ContextRef::new(ContextSnapshotId::new(), Utc::now())
}

#[test]
fn composing_to_ready_to_committed_no_ratify() {
    let act = read_act();
    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    let resolved = ResolvedAct::empty(act);

    let composing = Directive::new(SpeechAct::Directive, resolved)
        .bind_slot(bound_file_slot("file", "/tmp/x.txt"));

    assert_eq!(composing.state, DirectiveState::Composing);

    let confidence =
        Confidence::from_slots(vec![("file".to_owned(), calibrated_score(0.9))]).unwrap();
    let ready = composing.try_finalize(fresh_context(), policy, confidence).unwrap();

    assert_eq!(ready.state, DirectiveState::Ready);

    let committed = ready.commit().unwrap();
    assert_eq!(committed.state, DirectiveState::Committed);
    assert!(committed.committed_at.is_some());
    // Guarantee 5: snapshot is carried.
    assert!(committed.context.is_some());
}

#[test]
fn composing_to_ready_to_proposed_to_committed_ratify() {
    let act = delete_act();
    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    assert!(policy.requires_ratify);

    let resolved = ResolvedAct::empty(act);
    let composing = Directive::new(SpeechAct::Directive, resolved)
        .bind_slot(bound_file_slot("file", "/tmp/junk.txt"));

    let confidence =
        Confidence::from_slots(vec![("file".to_owned(), calibrated_score(0.95))]).unwrap();
    let ready = composing.try_finalize(fresh_context(), policy, confidence).unwrap();
    assert_eq!(ready.state, DirectiveState::Ready);

    // Direct commit must error (Guarantee 4: ratify required).
    let (ready_back, err) = ready.commit().expect_err("commit must reject when ratify required");
    assert!(matches!(err, pneuma_core::ContractError::RatifyRequired));

    // Use propose() instead.
    let proposed = ready_back.propose();
    assert_eq!(proposed.state, DirectiveState::Proposed);

    let committed = proposed.ratify();
    assert_eq!(committed.state, DirectiveState::Committed);
}

#[test]
fn proposed_reject_for_amendment_returns_to_composing() {
    let act = delete_act();
    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    let resolved = ResolvedAct::empty(act);

    let composing = Directive::new(SpeechAct::Directive, resolved)
        .bind_slot(bound_file_slot("file", "/tmp/junk.txt"));

    let confidence =
        Confidence::from_slots(vec![("file".to_owned(), calibrated_score(0.95))]).unwrap();
    let ready = composing.try_finalize(fresh_context(), policy, confidence).unwrap();
    let proposed = ready.propose();
    let amended = proposed.reject_for_amendment();

    // Back to composing — policy and confidence reset to None so the
    // user can amend slots and re-finalize.
    assert_eq!(amended.state, DirectiveState::Composing);
    assert_eq!(amended.policy, None);
    assert_eq!(amended.confidence, None);
}

#[test]
fn typestate_prevents_committing_from_composing() {
    // The typestate is a *compile-time* guarantee — `commit()` only
    // exists on `Directive<Ready>`. We can't observe a missing method
    // at runtime, but the related compile_fail doctest in `directive.rs`
    // would catch a regression. Here we verify the runtime mirror state
    // is correct.
    let act = read_act();
    let resolved = ResolvedAct::empty(act);
    let composing = Directive::new(SpeechAct::Directive, resolved);
    assert_eq!(composing.state, DirectiveState::Composing);
    // (Cannot call composing.commit() — does not exist. Verified by
    // compile_fail in directive.rs documentation tests.)
}

#[test]
fn directive_id_is_unique_per_construction() {
    let act = read_act();
    let resolved_a = ResolvedAct::empty(act.clone());
    let resolved_b = ResolvedAct::empty(act);

    let a = Directive::new(SpeechAct::Directive, resolved_a);
    let b = Directive::new(SpeechAct::Directive, resolved_b);
    assert_ne!(a.id, b.id);
}

#[test]
fn grammar_version_is_stamped() {
    let act = read_act();
    let resolved = ResolvedAct::empty(act);
    let d = Directive::new(SpeechAct::Directive, resolved);
    assert_eq!(d.grammar_version, pneuma_core::GRAMMAR_VERSION);
}
