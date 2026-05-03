//! End-to-end ratify-required path.
//!
//! Counterpart to `end_to_end_rename`: exercises the case where the
//! intrinsic policy demands ratification (Irreversible or User+ blast).
//! No real executor runs — we route, refuse direct commit, propose,
//! ratify, then journal. This is the full Tier-2 Week-1 contract path
//! at the safety-critical end of the policy table.

use chrono::Utc;
use pneuma_acts::registry;
use pneuma_core::act::ResolvedSlotValue;
use pneuma_core::{
    BindingKind, Confidence, ConfidenceProducer, ConfidenceScore, ContextRef, ContextSnapshotId,
    ContractError, Directive, PolicyEnvelope, Provenance, ResolvedAct, ResolvedSlot, SpeechAct,
};
use pneuma_lago_bridge::{JournalReader, JournalRecord, JournalWriter};
use sensorium_core::{Timestamp, WorkspaceContext};

#[test]
fn spaces_broadcast_must_ratify_before_commit() {
    let dir = tempfile::tempdir().unwrap();
    let context = WorkspaceContext::neutral(Timestamp::now());

    // spaces.broadcast: Irreversible + External → very high threshold,
    // ratify required.
    let act = registry()
        .into_iter()
        .find(|a| a.id.as_str() == "spaces.broadcast")
        .expect("spaces.broadcast canonical");
    assert!(matches!(
        act.reversibility,
        pneuma_core::Reversibility::Irreversible
    ));
    assert!(matches!(
        act.blast_radius,
        pneuma_core::BlastRadius::External
    ));

    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    assert!(policy.requires_ratify);
    // Threshold for Irreversible+External = 0.95.
    assert!((policy.min_confidence - 0.95).abs() < 1e-6);

    let resolved = ResolvedAct::empty(act);
    let composing = Directive::new(SpeechAct::Directive, resolved).bind_slot(
        ResolvedSlot::new(
            "body",
            ResolvedSlotValue::String("Hello, world.".to_owned()),
            Provenance::new(Vec::new(), BindingKind::UserExplicit, Utc::now()),
        )
        .unwrap(),
    );

    // Confidence 0.96 — clears the 0.95 threshold for calibrated values.
    let confidence = Confidence::from_slots(vec![(
        "body".to_owned(),
        ConfidenceScore::new(0.96, true, ConfidenceProducer::Deterministic).unwrap(),
    )])
    .unwrap();

    let snapshot = context.snapshot();
    let context_ref = ContextRef::new(
        ContextSnapshotId::from_uuid(snapshot.id.into_inner()),
        snapshot.taken_at.into_inner(),
    );

    let ready = composing
        .try_finalize(context_ref, policy, confidence)
        .unwrap();

    let directive_id = ready.id;

    // Direct commit must error: policy requires ratify.
    let (ready_back, err) = ready
        .commit()
        .expect_err("Irreversible broadcast must reject direct commit");
    assert!(matches!(err, ContractError::RatifyRequired));

    // User ratifies.
    let proposed = ready_back.propose();
    assert_eq!(proposed.state, pneuma_core::DirectiveState::Proposed);
    let committed = proposed.ratify();
    assert_eq!(committed.state, pneuma_core::DirectiveState::Committed);

    // Journal the commit.
    let journal_path = dir.path().join("journal.ndjson");
    {
        let mut writer = JournalWriter::open(&journal_path).unwrap();
        writer.append(&JournalRecord::committed(committed)).unwrap();
        writer.flush().unwrap();
    }

    let reader = JournalReader::open(&journal_path);
    let records: Vec<_> = reader.iter().unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].directive_id(), directive_id);
}

#[test]
fn ratify_path_can_be_rejected_for_amendment() {
    let context = WorkspaceContext::neutral(Timestamp::now());
    let act = registry()
        .into_iter()
        .find(|a| a.id.as_str() == "spaces.broadcast")
        .unwrap();

    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    let resolved = ResolvedAct::empty(act);
    let composing = Directive::new(SpeechAct::Directive, resolved).bind_slot(
        ResolvedSlot::new(
            "body",
            ResolvedSlotValue::String("typo".to_owned()),
            Provenance::new(Vec::new(), BindingKind::UserExplicit, Utc::now()),
        )
        .unwrap(),
    );
    let confidence = Confidence::from_slots(vec![(
        "body".to_owned(),
        ConfidenceScore::new(0.96, true, ConfidenceProducer::Deterministic).unwrap(),
    )])
    .unwrap();

    let snapshot = context.snapshot();
    let context_ref = ContextRef::new(
        ContextSnapshotId::from_uuid(snapshot.id.into_inner()),
        snapshot.taken_at.into_inner(),
    );

    let ready = composing
        .try_finalize(context_ref, policy, confidence)
        .unwrap();
    let proposed = ready.propose();

    // User reviews the proposal, sees the typo, sends it back for
    // amendment. State returns to Composing with policy/confidence
    // cleared — user must re-finalize after amending.
    let amended = proposed.reject_for_amendment();
    assert_eq!(amended.state, pneuma_core::DirectiveState::Composing);
    assert_eq!(amended.policy, None);
    assert_eq!(amended.confidence, None);
}
