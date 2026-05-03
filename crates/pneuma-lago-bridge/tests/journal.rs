//! Journal append + replay tests.
//!
//! Properties under test:
//!
//! - Records round-trip through JSONL append + read.
//! - All five record kinds round-trip.
//! - `record_id` and `directive_id` accessors are correct for each variant.
//! - Iter handles blank lines gracefully.
//! - Iter reports line number on parse error.

use std::fs::OpenOptions;
use std::io::Write;

use chrono::Utc;
use pneuma_acts::registry;
use pneuma_core::act::ResolvedSlotValue;
use pneuma_core::{
    Act, BindingKind, Confidence, ConfidenceProducer, ConfidenceScore, ContextRef,
    ContextSnapshotId, Directive, DirectiveId, FileRef, PolicyEnvelope, Provenance, ReferentValue,
    ResolvedAct, ResolvedSlot, SpeechAct,
};
use pneuma_lago_bridge::{JournalReader, JournalRecord, JournalWriter};
use pneuma_praxis_bridge::{ExecutionOutcome, ReverseAction};

// --- Helpers ---------------------------------------------------------------

fn rename_act() -> Act {
    registry()
        .into_iter()
        .find(|a| a.id.as_str() == "file.rename")
        .unwrap()
}

fn build_committed_directive() -> Directive<pneuma_core::Committed> {
    let act = rename_act();
    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    let resolved = ResolvedAct::empty(act);
    let bound_target = ResolvedSlot::new(
        "target",
        ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new("/tmp/x.txt"))),
        Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now()),
    )
    .unwrap();
    let bound_name = ResolvedSlot::new(
        "new_name",
        ResolvedSlotValue::String("y.txt".to_owned()),
        Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now()),
    )
    .unwrap();
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

    Directive::new(SpeechAct::Directive, resolved)
        .bind_slot(bound_target)
        .bind_slot(bound_name)
        .try_finalize(
            ContextRef::new(ContextSnapshotId::new(), Utc::now()),
            policy,
            confidence,
        )
        .unwrap()
        .propose()
        .ratify()
}

// --- Round-trip tests ------------------------------------------------------

#[test]
fn committed_record_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.ndjson");

    let directive = build_committed_directive();
    let original_id = directive.id;

    let mut writer = JournalWriter::open(&path).unwrap();
    writer.append(&JournalRecord::committed(directive)).unwrap();
    writer.flush().unwrap();
    drop(writer);

    let reader = JournalReader::open(&path);
    let records: Vec<_> = reader.iter().unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(records.len(), 1);
    match &records[0] {
        JournalRecord::Committed { directive, .. } => {
            assert_eq!(directive.id, original_id);
        }
        other => panic!("expected Committed, got {other:?}"),
    }
}

#[test]
fn all_five_record_kinds_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.ndjson");

    let directive = build_committed_directive();
    let directive_id = directive.id;

    let outcome = ExecutionOutcome {
        act_id: pneuma_core::ActId::new("file.rename").unwrap(),
        result: serde_json::json!({"from": "/tmp/x.txt", "to": "/tmp/y.txt"}),
        reverse_action: ReverseAction::RenameBack {
            from: "/tmp/x.txt".into(),
            to: "/tmp/y.txt".into(),
        },
    };

    let mut writer = JournalWriter::open(&path).unwrap();
    writer.append(&JournalRecord::committed(directive)).unwrap();
    writer
        .append(&JournalRecord::executed(directive_id, outcome.clone()))
        .unwrap();
    writer
        .append(&JournalRecord::reversed(
            directive_id,
            outcome.reverse_action.clone(),
        ))
        .unwrap();
    writer
        .append(&JournalRecord::cancelled(directive_id, "user pressed Esc"))
        .unwrap();
    writer
        .append(&JournalRecord::failed(directive_id, "permission denied"))
        .unwrap();
    writer.flush().unwrap();
    drop(writer);

    let reader = JournalReader::open(&path);
    let records: Vec<_> = reader.iter().unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(records.len(), 5);
    assert!(matches!(records[0], JournalRecord::Committed { .. }));
    assert!(matches!(records[1], JournalRecord::Executed { .. }));
    assert!(matches!(records[2], JournalRecord::Reversed { .. }));
    assert!(matches!(records[3], JournalRecord::Cancelled { .. }));
    assert!(matches!(records[4], JournalRecord::Failed { .. }));

    // All records share the same directive_id (except the Committed
    // record, where the id comes from the embedded Directive).
    for r in &records {
        assert_eq!(r.directive_id(), directive_id);
    }
}

#[test]
fn record_id_is_unique_per_record() {
    let did = DirectiveId::new();
    let r1 = JournalRecord::cancelled(did, "first");
    let r2 = JournalRecord::cancelled(did, "second");
    assert_ne!(r1.record_id(), r2.record_id());
}

#[test]
fn iter_skips_blank_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.ndjson");

    let did = DirectiveId::new();
    {
        let mut writer = JournalWriter::open(&path).unwrap();
        writer
            .append(&JournalRecord::cancelled(did, "alpha"))
            .unwrap();
        writer.flush().unwrap();
    }
    // Append blank line + another record manually.
    let mut f = OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all(b"\n").unwrap();
    f.write_all(b"\n").unwrap();
    drop(f);
    {
        let mut writer = JournalWriter::open(&path).unwrap();
        writer
            .append(&JournalRecord::cancelled(did, "beta"))
            .unwrap();
        writer.flush().unwrap();
    }

    let reader = JournalReader::open(&path);
    let records: Vec<_> = reader.iter().unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(records.len(), 2);
}

#[test]
fn iter_reports_line_number_on_parse_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.ndjson");

    let did = DirectiveId::new();
    {
        let mut writer = JournalWriter::open(&path).unwrap();
        writer.append(&JournalRecord::cancelled(did, "ok")).unwrap();
        writer.flush().unwrap();
    }
    // Append a malformed line.
    let mut f = OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all(b"not json at all\n").unwrap();
    drop(f);

    let reader = JournalReader::open(&path);
    let mut iter = reader.iter().unwrap();
    let first = iter.next().unwrap();
    assert!(first.is_ok());
    let second = iter.next().unwrap();
    let err = second.unwrap_err();
    match err {
        pneuma_lago_bridge::JournalError::Deserialize { line, .. } => {
            assert_eq!(line, 2);
        }
        other => panic!("expected Deserialize, got {other:?}"),
    }
}

#[test]
fn writer_path_is_accessible() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.ndjson");
    let writer = JournalWriter::open(&path).unwrap();
    assert_eq!(writer.path(), path.as_path());
}
