//! End-to-end router tests. Each test builds a real
//! `Directive<Committed>` from a real `pneuma-acts` act, runs through
//! the router, and asserts the dispatch decision.
//!
//! Properties (cross-references):
//!
//! - Praxis acts route to `Dispatch::Praxis` with the right payload.
//! - Arcan acts route to `Dispatch::Arcan` with the utterance as
//!   instruction.
//! - Spaces acts route to `Dispatch::Spaces` with body extracted.
//! - Drift detection fires when the context snapshot differs from
//!   the directive's committed snapshot.
//! - Expired policies refuse via `RefusalReason::PolicyExpired`.
//! - `permitted_executors` lockdown overrides the act's hint.

use chrono::{Duration, Utc};
use pneuma_acts::registry;
use pneuma_core::{
    Act, BindingKind, Confidence, ConfidenceProducer, ConfidenceScore, ContextRef,
    ContextSnapshotId, Directive, ExecutorHint, ExecutorKind, FileRef, PolicyEnvelope, Provenance,
    ReferentValue, ResolvedAct, ResolvedSlot, SpeechAct,
};
use pneuma_core::act::ResolvedSlotValue;
use pneuma_router::{dispatch, dispatch_at, Dispatch, RefusalReason};
use sensorium_core::{Timestamp, WorkspaceContext, WorkspaceContextBuilder};

// --- Helpers ----------------------------------------------------------------

fn find_act(id: &str) -> Act {
    registry()
        .into_iter()
        .find(|a| a.id.as_str() == id)
        .unwrap_or_else(|| panic!("act {id} must be registered"))
}

fn calibrated(value: f32) -> ConfidenceScore {
    ConfidenceScore::new(value, true, ConfidenceProducer::Deterministic).unwrap()
}

fn det_provenance() -> Provenance {
    Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now())
}

/// Build a committed directive for `act`, binding `target` to a file,
/// using the substrate snapshot from `context`.
fn commit_with_file_target(
    act: &Act,
    file_path: &str,
    extra_slots: Vec<(&str, ResolvedSlotValue)>,
    context: &WorkspaceContext,
) -> (Directive<pneuma_core::Committed>, ContextSnapshotId) {
    let snapshot = context.snapshot();
    let snapshot_id = ContextSnapshotId::from_uuid(snapshot.id.into_inner());
    let context_ref = ContextRef::new(
        snapshot_id,
        snapshot.taken_at.into_inner(),
    );

    let resolved = ResolvedAct::empty(act.clone());
    let mut composing = Directive::new(SpeechAct::Directive, resolved).bind_slot(
        ResolvedSlot::new(
            "target",
            ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new(file_path))),
            det_provenance(),
        )
        .unwrap(),
    );
    for (name, value) in extra_slots {
        composing = composing.bind_slot(
            ResolvedSlot::new(name, value, det_provenance()).unwrap(),
        );
    }

    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    let mut slot_scores = vec![("target".to_owned(), calibrated(0.95))];
    for s in &composing.act.act.slots {
        if s.name != "target" && s.arity == pneuma_core::Arity::Required {
            slot_scores.push((s.name.clone(), calibrated(0.95)));
        }
    }
    let confidence = Confidence::from_slots(slot_scores).unwrap();

    let ready = composing.try_finalize(context_ref, policy, confidence).unwrap();
    let requires_ratify = ready.policy.as_ref().is_some_and(|p| p.requires_ratify);
    let committed = if requires_ratify {
        ready.propose().ratify()
    } else {
        ready.commit().unwrap()
    };
    (committed, snapshot_id)
}

// --- Praxis routing ---------------------------------------------------------

#[test]
fn file_open_routes_to_praxis() {
    let context = WorkspaceContext::neutral(Timestamp::now());
    let (committed, _) = commit_with_file_target(
        &find_act("file.open"),
        "/tmp/x.txt",
        vec![],
        &context,
    );

    match dispatch(&committed, &context) {
        Dispatch::Praxis(call) => {
            assert_eq!(call.act_id.as_str(), "file.open");
            assert_eq!(call.slots.len(), 1);
            assert_eq!(call.slots[0].0, "target");
            assert_eq!(call.reverse_recipe.as_deref(), Some("close_file"));
        }
        other => panic!("expected Praxis, got {other:?}"),
    }
}

#[test]
fn file_rename_carries_reverse_recipe() {
    let context = WorkspaceContext::neutral(Timestamp::now());
    let (committed, _) = commit_with_file_target(
        &find_act("file.rename"),
        "/tmp/x.txt",
        vec![("new_name", ResolvedSlotValue::String("y.txt".to_owned()))],
        &context,
    );

    match dispatch(&committed, &context) {
        Dispatch::Praxis(call) => {
            assert_eq!(call.act_id.as_str(), "file.rename");
            assert_eq!(call.reverse_recipe.as_deref(), Some("rename_back"));
            assert!(call.slots.iter().any(|(n, _)| n == "new_name"));
        }
        other => panic!("expected Praxis, got {other:?}"),
    }
}

// --- Arcan routing ----------------------------------------------------------

#[test]
fn agent_explain_routes_to_arcan_with_utterance_as_instruction() {
    let context = WorkspaceContext::neutral(Timestamp::now());
    let act = find_act("agent.explain");

    let snapshot = context.snapshot();
    let snapshot_id = ContextSnapshotId::from_uuid(snapshot.id.into_inner());
    let ctx_ref = ContextRef::new(snapshot_id, snapshot.taken_at.into_inner());

    let resolved = ResolvedAct::empty(act.clone());
    let composing = Directive::new(SpeechAct::Directive, resolved)
        .bind_slot(
            ResolvedSlot::new(
                "target",
                ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new("/tmp/x.txt"))),
                det_provenance(),
            )
            .unwrap(),
        )
        .with_utterance("explain this file");

    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    let confidence = Confidence::from_slots(vec![("target".to_owned(), calibrated(0.95))]).unwrap();
    let ready = composing.try_finalize(ctx_ref, policy, confidence).unwrap();
    let committed = ready.commit().unwrap();

    match dispatch(&committed, &context) {
        Dispatch::Arcan(prompt) => {
            assert_eq!(prompt.act_id.as_str(), "agent.explain");
            assert_eq!(prompt.instruction, "explain this file");
        }
        other => panic!("expected Arcan, got {other:?}"),
    }
}

// --- Drift detection (caller-side helper) -----------------------------------

#[test]
fn caller_side_drift_helper_fires_after_rebuild() {
    use pneuma_router::drift_detected;

    let context_a = WorkspaceContext::neutral(Timestamp::now());
    let snap_committed = context_a.snapshot();

    // Substrate observes a change — a new context is built.
    let context_b = WorkspaceContextBuilder::from_context(&context_a)
        .assembled_at(Timestamp::now())
        .build();
    let snap_current = context_b.snapshot();

    assert!(
        drift_detected(&snap_committed, &snap_current),
        "rebuild must register as drift via Arc::ptr_eq"
    );
}

#[test]
fn caller_side_drift_helper_quiet_when_context_unchanged() {
    use pneuma_router::drift_detected;

    let context = WorkspaceContext::neutral(Timestamp::now());
    let a = context.snapshot();
    let b = context.snapshot();
    assert!(
        !drift_detected(&a, &b),
        "two snapshots from same un-rebuilt context observe same Arc"
    );
}

// --- Validity window --------------------------------------------------------

#[test]
fn expired_policy_refuses_dispatch() {
    let context = WorkspaceContext::neutral(Timestamp::now());
    let act = find_act("file.open");

    // Build a directive with policy.valid_until in the future relative
    // to construction, but call dispatch_at with a `now` past that.
    let snapshot = context.snapshot();
    let snapshot_id = ContextSnapshotId::from_uuid(snapshot.id.into_inner());
    let ctx_ref = ContextRef::new(snapshot_id, snapshot.taken_at.into_inner());

    let resolved = ResolvedAct::empty(act.clone());
    let composing = Directive::new(SpeechAct::Directive, resolved).bind_slot(
        ResolvedSlot::new(
            "target",
            ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new("/tmp/x.txt"))),
            det_provenance(),
        )
        .unwrap(),
    );
    let mut policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    let now = Utc::now();
    policy.valid_until = Some(now + Duration::seconds(10));
    let confidence = Confidence::from_slots(vec![("target".to_owned(), calibrated(0.95))]).unwrap();

    let committed = composing
        .try_finalize(ctx_ref, policy, confidence)
        .unwrap()
        .commit()
        .unwrap();

    // Fast-forward time past the deadline.
    let later = now + Duration::seconds(60);
    match dispatch_at(&committed, &context, later) {
        Dispatch::Refuse(RefusalReason::PolicyExpired) => {}
        other => panic!("expected PolicyExpired refusal, got {other:?}"),
    }
}

// --- Permitted executors ----------------------------------------------------

#[test]
fn executor_lockdown_overrides_act_hint() {
    let context = WorkspaceContext::neutral(Timestamp::now());
    let act = find_act("file.open"); // hints Praxis
    assert_eq!(act.executor_hint, ExecutorHint::Praxis);

    let snapshot = context.snapshot();
    let snapshot_id = ContextSnapshotId::from_uuid(snapshot.id.into_inner());
    let ctx_ref = ContextRef::new(snapshot_id, snapshot.taken_at.into_inner());

    let resolved = ResolvedAct::empty(act.clone());
    let composing = Directive::new(SpeechAct::Directive, resolved).bind_slot(
        ResolvedSlot::new(
            "target",
            ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new("/tmp/x.txt"))),
            det_provenance(),
        )
        .unwrap(),
    );

    let mut policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    // Lock down to Arcan only.
    policy.permitted_executors = vec![ExecutorKind::Arcan];

    let confidence = Confidence::from_slots(vec![("target".to_owned(), calibrated(0.95))]).unwrap();
    let committed = composing
        .try_finalize(ctx_ref, policy, confidence)
        .unwrap()
        .commit()
        .unwrap();

    match dispatch(&committed, &context) {
        Dispatch::Refuse(RefusalReason::ExecutorNotPermitted { wanted, permitted }) => {
            assert_eq!(wanted, ExecutorHint::Praxis);
            assert_eq!(permitted, vec![ExecutorKind::Arcan]);
        }
        other => panic!("expected ExecutorNotPermitted, got {other:?}"),
    }
}

// --- Determinism / pure-function property ----------------------------------

#[test]
fn router_is_deterministic_at_fixed_now() {
    let context = WorkspaceContext::neutral(Timestamp::now());
    let (committed, _) = commit_with_file_target(
        &find_act("file.open"),
        "/tmp/x.txt",
        vec![],
        &context,
    );
    let now = Utc::now();
    let a = dispatch_at(&committed, &context, now);
    let b = dispatch_at(&committed, &context, now);
    assert_eq!(a, b, "router is pure: same inputs → same output");
}

#[test]
fn dispatch_is_refused_returns_true_for_refusal() {
    // Build a directive whose policy locks down to an executor the
    // act doesn't hint, so the router refuses.
    let context = WorkspaceContext::neutral(Timestamp::now());
    let act = find_act("file.open"); // hints Praxis

    let snapshot = context.snapshot();
    let snapshot_id = ContextSnapshotId::from_uuid(snapshot.id.into_inner());
    let ctx_ref = ContextRef::new(snapshot_id, snapshot.taken_at.into_inner());

    let resolved = ResolvedAct::empty(act.clone());
    let composing = Directive::new(SpeechAct::Directive, resolved).bind_slot(
        ResolvedSlot::new(
            "target",
            ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new("/tmp/x.txt"))),
            det_provenance(),
        )
        .unwrap(),
    );
    let mut policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    policy.permitted_executors = vec![ExecutorKind::Spaces]; // act hints Praxis; mismatch → refuse
    let confidence = Confidence::from_slots(vec![("target".to_owned(), calibrated(0.95))]).unwrap();
    let committed = composing
        .try_finalize(ctx_ref, policy, confidence)
        .unwrap()
        .commit()
        .unwrap();

    let d = dispatch(&committed, &context);
    assert!(d.is_refused());
}
