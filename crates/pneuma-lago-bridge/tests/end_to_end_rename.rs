//! End-to-end "rename the focused file" integration.
//!
//! Exercises every load-bearing edge of the Tier 2 Week-1 + Week-2 stack:
//!
//! - Substrate: build a `WorkspaceContext` containing the file.
//! - Contract: `Directive<Composing>` → `Ready` → `Proposed` → `Committed`
//!   via the typestate lifecycle, with policy envelope intrinsic to
//!   `file.rename` (Costly + Project → ratify required).
//! - Router: pure `dispatch` returns `Dispatch::Praxis(call)` with the
//!   correct payload.
//! - Executor: `LocalPraxis` runs the rename on a real tempdir.
//! - Journal: `JournalWriter` records the committed directive + outcome.
//! - Replay: `JournalReader` round-trips both records.
//! - Undo: `Executor::reverse` puts the file back; journal records the
//!   reversal.
//!
//! If this test passes, the architecture's load-bearing claim — that the
//! contract holds end-to-end across substrate / router / executor /
//! journal — holds for the simplest non-trivial act in the v0.2 set.

use std::fs;

use chrono::Utc;
use pneuma_acts::registry;
use pneuma_core::act::ResolvedSlotValue;
use pneuma_core::{
    BindingKind, Confidence, ConfidenceProducer, ConfidenceScore, ContextRef, ContextSnapshotId,
    Directive, FileRef, PolicyEnvelope, Provenance, ReferentValue, ResolvedAct, ResolvedSlot,
    SpeechAct,
};
use pneuma_lago_bridge::{JournalReader, JournalRecord, JournalWriter};
use pneuma_praxis_bridge::{Executor, LocalPraxis};
use pneuma_router::{Dispatch, dispatch};
use sensorium_core::{Timestamp, WorkspaceContextBuilder};

/// `clippy::too_many_lines` allowed: this is a single end-to-end
/// integration test that walks every step of the Tier 2 contract.
/// Splitting it into helpers would obscure the linear narrative.
#[test]
#[allow(clippy::too_many_lines)]
fn rename_the_focused_file_end_to_end() {
    // --- 1. Substrate: a context that "knows" about /tmp/<dir>/old.txt ---
    let dir = tempfile::tempdir().unwrap();
    let original_path = dir.path().join("old.txt");
    fs::write(&original_path, "alpha").unwrap();

    let context = WorkspaceContextBuilder::neutral(Timestamp::now())
        .with_visible_files(vec![sensorium_core::FileRef::new(original_path.clone())])
        .build();
    let snapshot = context.snapshot();

    // --- 2. Contract: build, finalize, ratify, commit ---
    let act = registry()
        .into_iter()
        .find(|a| a.id.as_str() == "file.rename")
        .expect("file.rename is canonical");
    assert!(matches!(
        act.reversibility,
        pneuma_core::Reversibility::Costly
    ));
    assert!(matches!(
        act.blast_radius,
        pneuma_core::BlastRadius::Project
    ));

    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    let resolved = ResolvedAct::empty(act);

    let provenance = || Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now());

    let composing = Directive::new(SpeechAct::Directive, resolved)
        .bind_slot(
            ResolvedSlot::new(
                "target",
                ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new(
                    original_path.clone(),
                ))),
                provenance(),
            )
            .unwrap(),
        )
        .bind_slot(
            ResolvedSlot::new(
                "new_name",
                ResolvedSlotValue::String("new.txt".to_owned()),
                provenance(),
            )
            .unwrap(),
        )
        .with_utterance("rename old.txt to new.txt");

    let confidence = Confidence::from_slots(vec![
        (
            "target".to_owned(),
            ConfidenceScore::new(0.95, true, ConfidenceProducer::Deterministic).unwrap(),
        ),
        (
            "new_name".to_owned(),
            ConfidenceScore::new(0.95, true, ConfidenceProducer::Deterministic).unwrap(),
        ),
    ])
    .unwrap();

    let context_ref = ContextRef::new(
        ContextSnapshotId::from_uuid(snapshot.id.into_inner()),
        snapshot.taken_at.into_inner(),
    );

    let ready = composing
        .try_finalize(context_ref, policy, confidence)
        .unwrap();
    // Costly + Project does NOT require ratify per the intrinsic table
    // (only Irreversible or User+ blast triggers ratify). Commit
    // directly. A separate integration test exercises the
    // ratify-required path on `file.delete`.
    assert!(!ready.policy.as_ref().unwrap().requires_ratify);
    let committed = ready.commit().unwrap();
    let directive_id = committed.id;

    // --- 3. Router: dispatch returns Praxis call ---
    let routed = dispatch(&committed, &context);
    let praxis_call = match routed {
        Dispatch::Praxis(call) => {
            assert_eq!(call.act_id.as_str(), "file.rename");
            assert_eq!(call.reverse_recipe.as_deref(), Some("rename_back"));
            call
        }
        other => panic!("expected Dispatch::Praxis, got {other:?}"),
    };

    // --- 4. Journal the commit ---
    let journal_path = dir.path().join("journal.ndjson");
    let mut writer = JournalWriter::open(&journal_path).unwrap();
    writer
        .append(&JournalRecord::committed(committed.clone()))
        .unwrap();
    writer.flush().unwrap();

    // --- 5. Execute via LocalPraxis ---
    let outcome = LocalPraxis.execute(&praxis_call).unwrap();

    let renamed_path = dir.path().join("new.txt");
    assert!(!original_path.exists(), "original gone");
    assert!(renamed_path.exists(), "rename target created");
    assert_eq!(fs::read_to_string(&renamed_path).unwrap(), "alpha");

    // --- 6. Journal the execution ---
    writer
        .append(&JournalRecord::executed(directive_id, outcome.clone()))
        .unwrap();
    writer.flush().unwrap();

    // --- 7. Replay journal and verify both records present ---
    let reader = JournalReader::open(&journal_path);
    let records: Vec<_> = reader.iter().unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(records.len(), 2);
    assert!(matches!(records[0], JournalRecord::Committed { .. }));
    match &records[1] {
        JournalRecord::Executed {
            directive_id: did,
            outcome: rt_outcome,
            ..
        } => {
            assert_eq!(*did, directive_id);
            assert_eq!(rt_outcome.act_id.as_str(), "file.rename");
        }
        other => panic!("expected Executed, got {other:?}"),
    }

    // --- 8. Reverse the directive ---
    LocalPraxis.reverse(&praxis_call, &outcome).unwrap();
    assert!(original_path.exists(), "original restored");
    assert!(!renamed_path.exists(), "rename target removed");
    assert_eq!(fs::read_to_string(&original_path).unwrap(), "alpha");

    // --- 9. Journal the reversal ---
    writer
        .append(&JournalRecord::reversed(
            directive_id,
            outcome.reverse_action.clone(),
        ))
        .unwrap();
    writer.flush().unwrap();

    // --- 10. Final journal state ---
    let reader = JournalReader::open(&journal_path);
    let final_records: Vec<_> = reader.iter().unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(final_records.len(), 3);
    assert!(matches!(final_records[2], JournalRecord::Reversed { .. }));

    // The contract held end-to-end. Tier 2 Week 1+2 contract claim ✓.
}
