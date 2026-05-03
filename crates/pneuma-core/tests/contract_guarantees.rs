//! The five categorical guarantees from `MIL-PROJECT.md` §6.3.
//!
//! 1. No directive dispatches without all required slots bound.
//! 2. No directive dispatches with mismatched referent types.
//! 3. No directive dispatches below its policy envelope's confidence
//!    threshold.
//! 4. No irreversible-or-large-blast directive bypasses ratification.
//! 5. Every committed directive carries the workspace snapshot it was
//!    committed against.

use chrono::Utc;
use pneuma_core::act::ResolvedSlotValue;
use pneuma_core::{
    Act, ActId, ActPrimitive, Arity, BindingKind, BlastRadius, Confidence, ConfidenceProducer,
    ConfidenceScore, ContextRef, ContextSnapshotId, ContractError, Directive, ExecutorHint,
    FileRef, PolicyEnvelope, Provenance, ReferentType, ReferentValue, ResolvedAct, ResolvedSlot,
    Reversibility, SlotKind, SlotSignature, SpeechAct, WindowId,
};

fn act_with_required_file_slot(id: &str, rev: Reversibility, blast: BlastRadius) -> Act {
    Act {
        id: ActId::new(id).unwrap(),
        primitive: ActPrimitive::Custom,
        slots: vec![
            SlotSignature::new(
                "target",
                SlotKind::Referent(ReferentType::File),
                Arity::Required,
            )
            .unwrap(),
        ],
        reversibility: rev,
        blast_radius: blast,
        executor_hint: ExecutorHint::Praxis,
        reverse_recipe: None,
        description: None,
    }
}

fn calibrated(value: f32) -> ConfidenceScore {
    ConfidenceScore::new(value, true, ConfidenceProducer::Deterministic).unwrap()
}

fn det_provenance() -> Provenance {
    Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now())
}

fn fresh_context() -> ContextRef {
    ContextRef::new(ContextSnapshotId::new(), Utc::now())
}

// --- Guarantee 1: required slots bound -------------------------------------

#[test]
fn unbound_required_slot_blocks_finalization() {
    let act = act_with_required_file_slot("file.read", Reversibility::Free, BlastRadius::Local);
    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    let resolved = ResolvedAct::empty(act);
    let composing = Directive::new(SpeechAct::Directive, resolved);
    // No slot bound — try_finalize must fail.

    let confidence = Confidence::from_slots(vec![]).unwrap();
    let result = composing.try_finalize(fresh_context(), policy, confidence);
    let (_, err) = result.unwrap_err();
    assert!(matches!(
        err,
        ContractError::UnboundRequiredSlot { ref slot, .. } if slot == "target"
    ));
}

#[test]
fn optional_slot_unbound_does_not_block() {
    let act = Act {
        id: ActId::new("file.touch").unwrap(),
        primitive: ActPrimitive::Custom,
        slots: vec![
            SlotSignature::new(
                "target",
                SlotKind::Referent(ReferentType::File),
                Arity::Required,
            )
            .unwrap(),
            SlotSignature::new("mode", SlotKind::String, Arity::Optional).unwrap(),
        ],
        reversibility: Reversibility::Free,
        blast_radius: BlastRadius::Local,
        executor_hint: ExecutorHint::Praxis,
        reverse_recipe: None,
        description: None,
    };
    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    let resolved = ResolvedAct::empty(act);
    let composing = Directive::new(SpeechAct::Directive, resolved).bind_slot(
        ResolvedSlot::new(
            "target",
            ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new("/tmp/x"))),
            det_provenance(),
        )
        .unwrap(),
    );

    let confidence = Confidence::from_slots(vec![("target".to_owned(), calibrated(0.9))]).unwrap();
    composing
        .try_finalize(fresh_context(), policy, confidence)
        .expect("optional slot can be left unbound");
}

// --- Guarantee 2: type matching ---------------------------------------------

#[test]
fn type_mismatch_blocks_finalization() {
    let act = act_with_required_file_slot("file.read", Reversibility::Free, BlastRadius::Local);
    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    let resolved = ResolvedAct::empty(act);
    // Slot declared `File`, but bind a `Window` value.
    let bad = ResolvedSlot::new(
        "target",
        ResolvedSlotValue::Referent(ReferentValue::Window(WindowId::new("win-1").unwrap())),
        det_provenance(),
    )
    .unwrap();

    let composing = Directive::new(SpeechAct::Directive, resolved).bind_slot(bad);
    let confidence = Confidence::from_slots(vec![("target".to_owned(), calibrated(0.9))]).unwrap();

    let (_, err) = composing
        .try_finalize(fresh_context(), policy, confidence)
        .expect_err("type mismatch must reject");
    assert!(matches!(
        err,
        ContractError::TypeMismatch {
            expected: ReferentType::File,
            actual: ReferentType::Window,
            ..
        }
    ));
}

#[test]
fn any_referent_type_accepts_anything() {
    let act = Act {
        id: ActId::new("debug.show").unwrap(),
        primitive: ActPrimitive::Custom,
        slots: vec![
            SlotSignature::new(
                "target",
                SlotKind::Referent(ReferentType::Any),
                Arity::Required,
            )
            .unwrap(),
        ],
        reversibility: Reversibility::Free,
        blast_radius: BlastRadius::Local,
        executor_hint: ExecutorHint::Praxis,
        reverse_recipe: None,
        description: None,
    };
    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    let resolved = ResolvedAct::empty(act);
    // Any type works.
    let bound = ResolvedSlot::new(
        "target",
        ResolvedSlotValue::Referent(ReferentValue::Window(WindowId::new("win-1").unwrap())),
        det_provenance(),
    )
    .unwrap();
    let confidence = Confidence::from_slots(vec![("target".to_owned(), calibrated(0.9))]).unwrap();
    Directive::new(SpeechAct::Directive, resolved)
        .bind_slot(bound)
        .try_finalize(fresh_context(), policy, confidence)
        .expect("Any accepts Window");
}

// --- Guarantee 3: confidence threshold --------------------------------------

#[test]
fn below_threshold_confidence_blocks_finalization() {
    let act = act_with_required_file_slot("file.read", Reversibility::Free, BlastRadius::Local);
    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    // intrinsic for (Free, Local) = 0.55; bind value below.
    let resolved = ResolvedAct::empty(act);
    let bound = ResolvedSlot::new(
        "target",
        ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new("/tmp/x"))),
        det_provenance(),
    )
    .unwrap();
    let confidence = Confidence::from_slots(vec![("target".to_owned(), calibrated(0.5))]).unwrap();

    let (_, err) = Directive::new(SpeechAct::Directive, resolved)
        .bind_slot(bound)
        .try_finalize(fresh_context(), policy, confidence)
        .expect_err("low confidence must reject");
    assert!(matches!(
        err,
        ContractError::ConfidenceBelowThreshold { .. }
    ));
}

#[test]
fn uncalibrated_confidence_pays_penalty() {
    let act = act_with_required_file_slot("file.read", Reversibility::Free, BlastRadius::Local);
    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    // policy.min_confidence = 0.55. Uncalibrated penalty raises the
    // effective threshold to 0.55 / (1 - 0.20) = 0.6875. A score of
    // 0.6 cleared the *nominal* threshold but does not clear the
    // effective one once uncalibrated.
    let resolved = ResolvedAct::empty(act);
    let bound = ResolvedSlot::new(
        "target",
        ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new("/tmp/x"))),
        det_provenance(),
    )
    .unwrap();
    let uncalibrated_score =
        ConfidenceScore::new(0.6, false, ConfidenceProducer::LlmLogprob).unwrap();
    let confidence =
        Confidence::from_slots(vec![("target".to_owned(), uncalibrated_score)]).unwrap();

    let result = Directive::new(SpeechAct::Directive, resolved)
        .bind_slot(bound)
        .try_finalize(fresh_context(), policy, confidence);
    let (_, err) = result.expect_err("uncalibrated 0.6 must not clear effective 0.6875");
    if let ContractError::ConfidenceBelowThreshold {
        effective_threshold,
        ..
    } = err
    {
        assert!(
            (effective_threshold - 0.6875).abs() < 1e-3,
            "effective threshold = nominal / (1 - 0.20)"
        );
    } else {
        panic!("expected ConfidenceBelowThreshold");
    }
}

// --- Guarantee 4: irreversible / large-blast forces ratification ------------

#[test]
fn irreversible_act_requires_ratify() {
    let act = act_with_required_file_slot(
        "file.delete",
        Reversibility::Irreversible,
        BlastRadius::Project,
    );
    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    assert!(policy.requires_ratify);
    let resolved = ResolvedAct::empty(act);
    let bound = ResolvedSlot::new(
        "target",
        ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new("/tmp/x"))),
        det_provenance(),
    )
    .unwrap();
    // Confidence above the higher Irreversible threshold (0.88).
    let confidence = Confidence::from_slots(vec![("target".to_owned(), calibrated(0.95))]).unwrap();
    let ready = Directive::new(SpeechAct::Directive, resolved)
        .bind_slot(bound)
        .try_finalize(fresh_context(), policy, confidence)
        .unwrap();

    let (_, err) = ready
        .commit()
        .expect_err("Irreversible must reject direct commit");
    assert!(matches!(err, ContractError::RatifyRequired));
}

#[test]
fn user_blast_requires_ratify_even_when_reversible() {
    let act = act_with_required_file_slot(
        "preferences.update",
        Reversibility::Costly,
        BlastRadius::User,
    );
    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    assert!(policy.requires_ratify);
}

#[test]
fn local_reversible_does_not_require_ratify() {
    let act = act_with_required_file_slot("cursor.move", Reversibility::Free, BlastRadius::Local);
    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    assert!(!policy.requires_ratify);
}

// --- Guarantee 5: snapshot carried -----------------------------------------

#[test]
fn committed_directive_carries_snapshot() {
    let act = act_with_required_file_slot("file.read", Reversibility::Free, BlastRadius::Local);
    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    let resolved = ResolvedAct::empty(act);
    let bound = ResolvedSlot::new(
        "target",
        ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new("/tmp/x"))),
        det_provenance(),
    )
    .unwrap();
    let confidence = Confidence::from_slots(vec![("target".to_owned(), calibrated(0.9))]).unwrap();

    let snapshot = ContextSnapshotId::new();
    let captured = Utc::now();
    let context = ContextRef::new(snapshot, captured);

    let committed = Directive::new(SpeechAct::Directive, resolved)
        .bind_slot(bound)
        .try_finalize(context, policy, confidence)
        .unwrap()
        .commit()
        .unwrap();

    let ctx = committed.context();
    assert_eq!(ctx.snapshot_id, snapshot);
    assert_eq!(ctx.captured_at, captured);
}

// --- Bonus: expiration ------------------------------------------------------

#[test]
fn expired_policy_blocks_finalization() {
    let act = act_with_required_file_slot("file.read", Reversibility::Free, BlastRadius::Local);
    let mut policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    // valid_until in the past.
    policy.valid_until = Some(Utc::now() - chrono::Duration::seconds(60));
    let resolved = ResolvedAct::empty(act);
    let bound = ResolvedSlot::new(
        "target",
        ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new("/tmp/x"))),
        det_provenance(),
    )
    .unwrap();
    let confidence = Confidence::from_slots(vec![("target".to_owned(), calibrated(0.9))]).unwrap();

    let (_, err) = Directive::new(SpeechAct::Directive, resolved)
        .bind_slot(bound)
        .try_finalize(fresh_context(), policy, confidence)
        .expect_err("expired policy must reject");
    assert!(matches!(err, ContractError::Expired { .. }));
}
