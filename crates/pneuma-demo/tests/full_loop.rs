//! Full Tier 2 correction-loop integration tests.
//!
//! Drives the demo with a `MockRatifier` so the test runs hermetically
//! (no stdin) and exercises every branch:
//!
//! 1. **Happy path**: commit → execute → undo. File ends back at
//!    original path. Journal records 4 entries (committed, executed,
//!    reversed; the cancel branch isn't taken).
//! 2. **Commit-only**: commit → execute → no undo. Journal records 2
//!    entries. File is at the renamed path.
//! 3. **Cancel-at-ready**: cancel before commit. Journal records a
//!    single Cancelled entry. File untouched.
//! 4. **Reject-at-ready**: reject before commit. Same as cancel for
//!    v0.2 hotkey FSM.

use std::fs;

use pneuma_demo::{Demo, DemoConfig, DemoError, manual_observer_for};
use pneuma_lago_bridge::{JournalReader, JournalRecord};
use pneuma_ratify::{ApprovalDecision, MockRatifier};

fn run_with_decisions(
    decisions: Vec<ApprovalDecision>,
) -> (
    Vec<JournalRecord>,
    Vec<u8>,
    std::path::PathBuf,
    std::path::PathBuf,
    Result<pneuma_demo::DemoSummary, DemoError>,
) {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("old.txt");
    fs::write(&source_path, "alpha").unwrap();
    let journal_path = dir.path().join("journal.ndjson");

    let config = DemoConfig {
        source_path: &source_path,
        new_name: "new.txt",
        journal_path: &journal_path,
        hud_width: 60,
        utterance: None,
    };
    let mut out = Vec::<u8>::new();
    let result = {
        let observer = Box::new(manual_observer_for(&source_path));
        let mut demo = Demo::new(
            config,
            &mut out,
            MockRatifier::from_decisions(decisions),
            observer,
        )
        .unwrap();
        demo.run_rename()
    };

    let records: Vec<_> = JournalReader::open(&journal_path)
        .iter()
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let renamed_path = dir.path().join("new.txt");

    // Keep the tempdir alive for the duration of the test by leaking
    // it; on test exit the OS reclaims /tmp.
    std::mem::forget(dir);

    (records, out, source_path, renamed_path, result)
}

// --- Happy-path: commit + undo --------------------------------------------

#[test]
fn full_loop_commit_then_undo() {
    let (records, out, original, renamed, result) =
        run_with_decisions(vec![ApprovalDecision::Commit, ApprovalDecision::Undo]);

    let summary = result.unwrap();
    assert!(summary.reversed, "demo must report reversed");
    assert!(original.exists(), "original path restored after undo");
    assert!(!renamed.exists(), "renamed path empty after undo");
    assert_eq!(fs::read_to_string(&original).unwrap(), "alpha");

    // Journal: Committed + Executed + Reversed = 3 entries.
    assert_eq!(records.len(), 3);
    assert!(matches!(records[0], JournalRecord::Committed { .. }));
    assert!(matches!(records[1], JournalRecord::Executed { .. }));
    assert!(matches!(records[2], JournalRecord::Reversed { .. }));

    // HUD output covers all the right frames.
    let body = String::from_utf8(out).unwrap();
    assert!(body.contains("COMPOSING"));
    assert!(body.contains("READY"));
    assert!(body.contains("COMMITTED"));
    assert!(body.contains("DONE"));
    assert!(body.contains("UNDONE"));
}

// --- Commit-only ----------------------------------------------------------

#[test]
fn full_loop_commit_keep() {
    let (records, _out, original, renamed, result) =
        run_with_decisions(vec![ApprovalDecision::Commit, ApprovalDecision::Cancel]);
    let summary = result.unwrap();
    assert!(!summary.reversed);
    assert!(!original.exists(), "original removed by rename");
    assert!(renamed.exists(), "renamed path still there");

    // Journal: Committed + Executed = 2 entries.
    assert_eq!(records.len(), 2);
    assert!(matches!(records[0], JournalRecord::Committed { .. }));
    assert!(matches!(records[1], JournalRecord::Executed { .. }));
}

// --- Cancel at ready ------------------------------------------------------

#[test]
fn full_loop_cancel_at_ready() {
    let (records, out, original, renamed, result) =
        run_with_decisions(vec![ApprovalDecision::Cancel]);
    assert!(matches!(result, Err(DemoError::Cancelled)));
    assert!(original.exists(), "file untouched on cancel");
    assert!(!renamed.exists(), "no rename happened");

    // Journal: 1 Cancelled entry.
    assert_eq!(records.len(), 1);
    assert!(matches!(records[0], JournalRecord::Cancelled { .. }));

    let body = String::from_utf8(out).unwrap();
    assert!(body.contains("READY"));
    assert!(!body.contains("DONE"), "execute did not run");
}

#[test]
fn full_loop_reject_at_ready_is_treated_as_cancel_for_v0_2() {
    let (records, _out, original, renamed, result) =
        run_with_decisions(vec![ApprovalDecision::Reject]);
    assert!(matches!(result, Err(DemoError::Cancelled)));
    assert!(original.exists());
    assert!(!renamed.exists());
    assert_eq!(records.len(), 1);
    assert!(matches!(records[0], JournalRecord::Cancelled { .. }));
}

// --- Approve-as-commit hotkey ---------------------------------------------

#[test]
fn approve_at_ready_is_equivalent_to_commit() {
    // Approve is the discrete hotkey for the proposal flow. v0.2
    // collapses Approve+Commit semantically at the ready gate.
    let (records, _out, original, renamed, result) =
        run_with_decisions(vec![ApprovalDecision::Approve, ApprovalDecision::Cancel]);
    let summary = result.unwrap();
    assert!(!summary.reversed);
    assert_eq!(records.len(), 2);
    assert!(!original.exists());
    assert!(renamed.exists());
}
