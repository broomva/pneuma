//! End-to-end correction-loop tests for the switch-app flow
//! (step #14's user-facing surface).
//!
//! Same shape as `full_navigate_loop.rs` — drives `Demo::run_switch_app`
//! with `MockRatifier`. Cross-platform tests assert cancel / reject
//! behavior and the typed `PlatformUnsupported` failure on non-macOS;
//! a `#[ignore]`-gated macOS test exercises a real Finder activation.
//!
//! Properties under test:
//!
//! 1. **Cancel-at-ready**: cancel before commit. Journal records a
//!    single Cancelled entry. No execution attempted.
//! 2. **Linux/Windows execute path**: commit fires; executor returns
//!    `PlatformUnsupported`; demo surfaces a typed Executor error;
//!    journal records `Failed`.
//! 3. (macOS only, gated) commit fires; activates Finder. No undo
//!    expected — `workspace.switch_app` is `Reversibility::Free`.

use std::path::Path;

use pneuma_demo::{Demo, DemoConfig, DemoError};
use pneuma_lago_bridge::{JournalReader, JournalRecord};
use pneuma_ratify::{ApprovalDecision, MockRatifier};
use sensorium_context::ManualObserver;
use sensorium_core::Timestamp;

const TEST_APP: &str = "Finder";

fn run_switch_app_with_decisions(
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
        utterance: Some("switch to Finder"),
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
        demo.run_switch_app(TEST_APP)
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
fn switch_app_cancel_at_ready_records_only_cancelled() {
    let (records, out, _journal, result) =
        run_switch_app_with_decisions(vec![ApprovalDecision::Cancel]);

    let err = result.unwrap_err();
    assert!(
        matches!(err, DemoError::Cancelled),
        "expected DemoError::Cancelled, got {err:?}"
    );

    assert_eq!(records.len(), 1);
    assert!(matches!(records[0], JournalRecord::Cancelled { .. }));

    let stdout = String::from_utf8_lossy(&out);
    assert!(
        stdout.to_uppercase().contains("CANCELLED"),
        "demo must surface CANCELLED frame on stdout"
    );
}

// --- Empty app name (caught upstream by AppId::new) -----------------------

#[test]
fn switch_app_with_empty_name_errors_at_directive_build() {
    // Demo::run_switch_app calls AppId::new(""), which errors before
    // any journal entries are written. This is the typestate boundary
    // doing its job.
    let dir = tempfile::tempdir().unwrap();
    let journal_path = dir.path().join("journal.ndjson");

    let config = DemoConfig {
        source_path: Path::new(""),
        new_name: "",
        journal_path: &journal_path,
        hud_width: 60,
        utterance: Some("switch to "),
    };
    let mut out = Vec::<u8>::new();
    let result = {
        let observer = Box::new(ManualObserver::new(Timestamp::now()));
        let mut demo = Demo::new(
            config,
            &mut out,
            MockRatifier::from_decisions(vec![ApprovalDecision::Cancel]),
            observer,
        )
        .unwrap();
        demo.run_switch_app("")
    };
    let err = result.unwrap_err();
    assert!(
        matches!(err, DemoError::Contract(_)),
        "empty app name must surface a Contract error from AppId::new, got {err:?}"
    );
    std::mem::forget(dir);
}

// --- Linux / Windows execute path -----------------------------------------

#[cfg(not(target_os = "macos"))]
#[test]
fn switch_app_commit_on_non_macos_journals_committed_then_failed() {
    let (records, _out, _journal, result) =
        run_switch_app_with_decisions(vec![ApprovalDecision::Commit]);

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

/// Real end-to-end test against macOS. Activates Finder (always
/// present, no Automation prompt for typical setups). Run manually:
///
/// ```bash
/// cargo test -p pneuma-demo --test full_switch_app_loop -- --ignored
/// ```
#[cfg(target_os = "macos")]
#[test]
#[ignore = "activates Finder on macOS — run manually"]
fn macos_switch_app_commit_no_undo() {
    // workspace.switch_app is Reversibility::Free — no Undo decision.
    // The demo's post-execute prompt runs once with Continue (no undo)
    // and exits clean.
    let (records, _out, _journal, result) =
        run_switch_app_with_decisions(vec![ApprovalDecision::Commit, ApprovalDecision::Cancel]);
    let summary = result.unwrap();
    assert!(
        !summary.reversed,
        "Free-reversibility act with Cancel-after-commit should not reverse"
    );
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
}
