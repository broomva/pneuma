//! End-to-end correction-loop tests for the navigate flow
//! (step #13's user-facing surface).
//!
//! Drives `Demo::run_navigate` with `MockRatifier` so no real Safari
//! involvement is needed. The actual AppleScript shell-out is gated
//! by the platform: on Linux CI, `LocalPraxis::execute` returns
//! `PraxisError::PlatformUnsupported` after the directive lifecycle
//! reaches dispatch — which is the production failure mode we want
//! to assert is *typed*, not a panic, not a silent success.
//!
//! Properties under test:
//!
//! 1. **Cancel-at-ready**: cancel before commit. Journal records a
//!    single Cancelled entry. No execution attempted.
//! 2. **Reject-at-ready**: same as cancel for v0.2 hotkey FSM.
//! 3. **Linux/Windows execute path**: commit fires; executor returns
//!    `PlatformUnsupported`; demo surfaces a typed Executor error;
//!    journal records `Failed`.
//! 4. (macOS only, gated `#[ignore]`) commit fires; navigates Safari
//!    real. Optionally undoes back to prior URL.

use std::path::Path;

use pneuma_demo::{Demo, DemoConfig, DemoError};
use pneuma_lago_bridge::{JournalReader, JournalRecord};
use pneuma_ratify::{ApprovalDecision, MockRatifier};
use sensorium_context::ManualObserver;
use sensorium_core::Timestamp;

const TEST_URL: &str = "https://example.com/mil-navigate-test";

fn run_navigate_with_decisions(
    decisions: Vec<ApprovalDecision>,
) -> (
    Vec<JournalRecord>,
    Vec<u8>,
    std::path::PathBuf,
    Result<pneuma_demo::DemoSummary, DemoError>,
) {
    let dir = tempfile::tempdir().unwrap();
    let journal_path = dir.path().join("journal.ndjson");

    let config = DemoConfig {
        source_path: Path::new(""),
        new_name: "",
        journal_path: &journal_path,
        hud_width: 60,
        utterance: Some("navigate to https://example.com/mil-navigate-test"),
    };
    let mut out = Vec::<u8>::new();
    let result = {
        let observer = Box::new(ManualObserver::new(Timestamp::now()));
        let mut demo = Demo::new(
            config,
            &mut out,
            MockRatifier::from_decisions(decisions),
            observer,
        )
        .unwrap();
        demo.run_navigate(TEST_URL)
    };

    let records: Vec<_> = JournalReader::open(&journal_path)
        .iter()
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    let journal_path_owned = journal_path.clone();
    std::mem::forget(dir);
    (records, out, journal_path_owned, result)
}

// --- Cancel before commit -------------------------------------------------

#[test]
fn navigate_cancel_at_ready_records_only_cancelled() {
    let (records, out, _journal, result) =
        run_navigate_with_decisions(vec![ApprovalDecision::Cancel]);

    let err = result.unwrap_err();
    assert!(
        matches!(err, DemoError::Cancelled),
        "expected DemoError::Cancelled, got {err:?}"
    );

    assert_eq!(records.len(), 1, "exactly one journal entry expected");
    assert!(
        matches!(records[0], JournalRecord::Cancelled { .. }),
        "first record must be Cancelled"
    );

    let stdout = String::from_utf8_lossy(&out);
    assert!(
        stdout.to_uppercase().contains("CANCELLED"),
        "demo must surface CANCELLED frame on stdout"
    );
}

#[test]
fn navigate_reject_at_ready_records_only_cancelled() {
    let (records, _out, _journal, result) =
        run_navigate_with_decisions(vec![ApprovalDecision::Reject]);

    let err = result.unwrap_err();
    assert!(
        matches!(err, DemoError::Cancelled),
        "Reject and Cancel collapse to the same v0.2 state"
    );
    assert_eq!(records.len(), 1);
    assert!(matches!(records[0], JournalRecord::Cancelled { .. }));
}

// --- Linux / Windows execute path -----------------------------------------

#[cfg(not(target_os = "macos"))]
#[test]
fn navigate_commit_on_non_macos_journals_committed_then_failed() {
    let (records, _out, _journal, result) =
        run_navigate_with_decisions(vec![ApprovalDecision::Commit]);

    // The commit succeeds (directive lifecycle is platform-agnostic);
    // the executor refuses with PlatformUnsupported. The demo
    // surfaces the executor error, journal records Committed then
    // Failed.
    let err = result.unwrap_err();
    assert!(
        matches!(err, DemoError::Executor(_)),
        "non-macOS execute must surface Executor error, got {err:?}"
    );

    assert!(
        records.len() >= 2,
        "journal should record at least Committed + Failed; got {records:?}"
    );
    assert!(matches!(records[0], JournalRecord::Committed { .. }));
    assert!(matches!(records[1], JournalRecord::Failed { .. }));
}

// --- macOS interactive (gated, ignored by default) ------------------------

/// Real end-to-end test against macOS Safari. Disabled by default
/// (would open a Safari window during `cargo test` and require the
/// user to grant Automation permissions). Run manually with:
///
/// ```bash
/// cargo test -p pneuma-demo --test full_navigate_loop -- --ignored
/// ```
#[cfg(target_os = "macos")]
#[test]
#[ignore = "opens a real Safari window — run manually"]
fn macos_navigate_commit_then_undo_round_trip() {
    let (records, _out, _journal, result) =
        run_navigate_with_decisions(vec![ApprovalDecision::Commit, ApprovalDecision::Undo]);
    let summary = result.unwrap();
    assert!(
        summary.reversed,
        "demo must report reversed when Undo decision is supplied"
    );

    // 4 records: Committed, Executed, Reversed (no Cancel branch
    // taken). Index 0 may also be Committed depending on exact
    // sequencing — assert by kind, not position.
    assert!(
        records
            .iter()
            .any(|r| matches!(r, JournalRecord::Committed { .. }))
    );
    assert!(
        records
            .iter()
            .any(|r| matches!(r, JournalRecord::Executed { .. }))
    );
    assert!(
        records
            .iter()
            .any(|r| matches!(r, JournalRecord::Reversed { .. }))
    );
}
