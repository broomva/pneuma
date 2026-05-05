//! End-to-end correction-loop tests for the Arcan flow
//! (step #16b's user-facing surface).
//!
//! Drives `Demo::run_arcan` with `MockRatifier` + `MockArcan` so no
//! real claude subprocess is involved. Validates the full directive-
//! contract chain against a synthetic agent runtime.
//!
//! Properties under test:
//!
//! 1. **Cancel-at-ready** — cancel before commit. Journal records a
//!    single Cancelled entry. No Arcan execution attempted.
//! 2. **Commit-then-exit** — commit fires; MockArcan returns canned
//!    response; journal records Committed + AgentExecuted.
//! 3. **Empty response** — MockArcan returns `""`; demo still
//!    journals AgentExecuted with empty response (defensible).
//! 4. **agent.refactor with target slot** — slot binding propagates
//!    through directive contract → router → arcan executor.
//!
//! macOS-interactive test against real `claude` lives in
//! `pneuma-arcan-bridge/tests/executor_dispatch.rs::claude_code_round_trip`.

use std::path::Path;

use pneuma_arcan_bridge::MockArcan;
use pneuma_core::FileRef;
use pneuma_core::ReferentValue;
use pneuma_core::act::ResolvedSlotValue;
use pneuma_demo::{Demo, DemoConfig, DemoError};
use pneuma_lago_bridge::{JournalReader, JournalRecord};
use pneuma_ratify::{ApprovalDecision, MockRatifier};
use sensorium_context::ManualObserver;
use sensorium_core::Timestamp;

const REFACTOR_INSTRUCTION: &str = "Refactor the authentication module to use ed25519.";
const CANNED_RESPONSE: &str = "Refactored 3 functions: parse_token, sign_token, verify_token.";

fn run_arcan_with_decisions(
    decisions: Vec<ApprovalDecision>,
    canned_response: &str,
) -> (
    Vec<JournalRecord>,
    Vec<u8>,
    std::path::PathBuf,
    Result<pneuma_demo::DemoSummary, DemoError>,
) {
    let dir = tempfile::tempdir().unwrap();
    let journal_path = dir.path().join("journal.ndjson");

    // Use a real path so the demo's "promote to FileRef if path exists"
    // logic kicks in, exercising the slot-resolution path.
    let target_path = dir.path().join("auth.rs");
    std::fs::write(&target_path, "// stub").unwrap();
    let payload_slots = vec![(
        "target".to_owned(),
        ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new(&target_path))),
    )];

    let config = DemoConfig {
        source_path: Path::new(""),
        new_name: "",
        journal_path: &journal_path,
        hud_width: 60,
        utterance: Some(REFACTOR_INSTRUCTION),
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
        let arcan = MockArcan::new(canned_response);
        demo.run_arcan(
            "agent.refactor",
            REFACTOR_INSTRUCTION,
            payload_slots,
            &arcan,
        )
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

// --- Property 1: Cancel before commit -------------------------------------

#[test]
fn arcan_cancel_at_ready_records_only_cancelled() {
    let (records, out, _journal, result) =
        run_arcan_with_decisions(vec![ApprovalDecision::Cancel], CANNED_RESPONSE);

    let err = result.unwrap_err();
    assert!(
        matches!(err, DemoError::Cancelled),
        "expected DemoError::Cancelled, got {err:?}"
    );

    assert_eq!(records.len(), 1);
    assert!(matches!(records[0], JournalRecord::Cancelled { .. }));

    let stdout = String::from_utf8_lossy(&out);
    assert!(stdout.to_uppercase().contains("CANCELLED"));
}

// --- Property 2: Commit fires; MockArcan responds; journal records --------

#[test]
fn arcan_commit_executes_mock_and_journals_agent_executed() {
    let (records, out, _journal, result) = run_arcan_with_decisions(
        vec![ApprovalDecision::Commit, ApprovalDecision::Cancel],
        CANNED_RESPONSE,
    );

    let summary = result.expect("agent run should succeed against MockArcan");
    assert!(!summary.reversed, "agent acts have no v0.2 reverse");
    // outcome: Option<ExecutionOutcome> stays None for arcan flows —
    // the response is in the journal as AgentExecuted.
    assert!(summary.outcome.is_none());

    // Journal records: Committed + AgentExecuted (in order).
    assert_eq!(
        records.len(),
        2,
        "expected exactly 2 records; got {records:?}"
    );
    assert!(matches!(records[0], JournalRecord::Committed { .. }));
    match &records[1] {
        JournalRecord::AgentExecuted {
            executor,
            response,
            exit_code,
            ..
        } => {
            assert_eq!(executor, "mock");
            assert_eq!(response, CANNED_RESPONSE);
            assert_eq!(*exit_code, 0);
        }
        other => panic!("expected AgentExecuted, got {other:?}"),
    }

    // Stdout shows the agent response in a HUD frame.
    let stdout = String::from_utf8_lossy(&out);
    assert!(
        stdout.contains("AGENT DONE"),
        "demo must surface AGENT DONE frame; got: {stdout}"
    );
    assert!(
        stdout.contains("Refactored 3 functions"),
        "agent response must appear in HUD"
    );
}

// --- Property 3: Empty response is recorded faithfully --------------------

#[test]
fn arcan_empty_response_is_journaled_verbatim() {
    let (records, out, _journal, result) =
        run_arcan_with_decisions(vec![ApprovalDecision::Commit, ApprovalDecision::Cancel], "");

    let _summary = result.expect("agent run should succeed even with empty response");
    let executed = records
        .iter()
        .find(|r| matches!(r, JournalRecord::AgentExecuted { .. }));
    match executed {
        Some(JournalRecord::AgentExecuted { response, .. }) => {
            assert_eq!(response, "", "empty response is recorded verbatim");
        }
        other => panic!("expected AgentExecuted, got {other:?}"),
    }
    let stdout = String::from_utf8_lossy(&out);
    assert!(
        stdout.contains("(empty response from agent)"),
        "HUD must surface 'empty response' marker; got: {stdout}"
    );
}

// --- Property 4: Undo-after-commit is a no-op for agent acts --------------

#[test]
fn arcan_undo_decision_after_commit_is_recognized_but_no_op() {
    let (_records, out, _journal, result) = run_arcan_with_decisions(
        vec![
            ApprovalDecision::Commit,
            ApprovalDecision::Undo,
            ApprovalDecision::Cancel,
        ],
        CANNED_RESPONSE,
    );
    let summary = result.unwrap();
    assert!(!summary.reversed);
    let stdout = String::from_utf8_lossy(&out);
    assert!(
        stdout.contains("agent acts have no v0.2 reverse"),
        "Undo decision must surface a NOTE explaining no-reverse for agents; got: {stdout}"
    );
}

// --- Property 5: Unknown act_id fails before lifecycle starts -------------

#[test]
fn arcan_unknown_act_id_errors_with_refused() {
    let dir = tempfile::tempdir().unwrap();
    let journal_path = dir.path().join("journal.ndjson");
    let config = DemoConfig {
        source_path: Path::new(""),
        new_name: "",
        journal_path: &journal_path,
        hud_width: 60,
        utterance: Some("do something"),
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
        let arcan = MockArcan::new("hi");
        demo.run_arcan("agent.never_existed", "do something", vec![], &arcan)
    };
    let err = result.unwrap_err();
    assert!(
        matches!(err, DemoError::Refused(_)),
        "expected DemoError::Refused for unknown act_id, got {err:?}"
    );
    std::mem::forget(dir);
}
