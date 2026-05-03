//! Ratifier tests. The FSM is small but safety-critical.
//!
//! Properties:
//!
//! - The six discourse moves all map from at least one input.
//! - Unknown keys produce `Continue` (no decision yet).
//! - Blank stdin line → `Commit` (Enter).
//! - EOF stdin → `Cancel` (don't hang).
//! - `?text` → `Clarify("text")`.
//! - `MockRatifier::from_keystream` drives a sequence correctly.
//! - Drained mock returns `Cancel` to avoid infinite loops.

use pneuma_ratify::{ApprovalDecision, MockRatifier, Ratifier, parse_line};

#[test]
fn six_canonical_moves_all_have_a_keybinding() {
    assert_eq!(ApprovalDecision::from_char('e'), ApprovalDecision::Engage);
    assert_eq!(ApprovalDecision::from_char('\n'), ApprovalDecision::Commit);
    assert_eq!(ApprovalDecision::from_char('q'), ApprovalDecision::Cancel);
    assert_eq!(ApprovalDecision::from_char('y'), ApprovalDecision::Approve);
    assert_eq!(ApprovalDecision::from_char('n'), ApprovalDecision::Reject);
    assert_eq!(ApprovalDecision::from_char('u'), ApprovalDecision::Undo);
}

#[test]
fn unknown_keys_produce_continue() {
    assert_eq!(ApprovalDecision::from_char('x'), ApprovalDecision::Continue);
    assert_eq!(ApprovalDecision::from_char('1'), ApprovalDecision::Continue);
    assert_eq!(ApprovalDecision::from_char(' '), ApprovalDecision::Continue);
}

#[test]
fn case_insensitive_for_letter_keys() {
    assert_eq!(ApprovalDecision::from_char('Y'), ApprovalDecision::Approve);
    assert_eq!(ApprovalDecision::from_char('N'), ApprovalDecision::Reject);
    assert_eq!(ApprovalDecision::from_char('U'), ApprovalDecision::Undo);
    assert_eq!(ApprovalDecision::from_char('Q'), ApprovalDecision::Cancel);
}

#[test]
fn esc_byte_maps_to_cancel() {
    assert_eq!(
        ApprovalDecision::from_char('\x1b'),
        ApprovalDecision::Cancel
    );
}

#[test]
fn amend_alias_maps_to_cancel_for_v0_2() {
    // v0.2 routes amend → cancel; the caller is responsible for
    // re-entering compose state. v0.3 will distinguish.
    assert_eq!(ApprovalDecision::from_char('a'), ApprovalDecision::Cancel);
}

#[test]
fn question_mark_yields_empty_clarify() {
    assert_eq!(
        ApprovalDecision::from_char('?'),
        ApprovalDecision::Clarify(String::new())
    );
}

#[test]
fn blank_line_is_commit() {
    assert_eq!(parse_line(""), ApprovalDecision::Commit);
    assert_eq!(parse_line("   \n"), ApprovalDecision::Commit);
    assert_eq!(parse_line("\n"), ApprovalDecision::Commit);
}

#[test]
fn line_starting_with_question_mark_carries_clarification_text() {
    assert_eq!(
        parse_line("? what does this do?\n"),
        ApprovalDecision::Clarify("what does this do?".to_owned())
    );
    assert_eq!(
        parse_line("?which file?"),
        ApprovalDecision::Clarify("which file?".to_owned())
    );
}

#[test]
fn line_with_first_char_drives_decision() {
    assert_eq!(parse_line("y\n"), ApprovalDecision::Approve);
    assert_eq!(parse_line("nope\n"), ApprovalDecision::Reject);
    assert_eq!(parse_line("undo me\n"), ApprovalDecision::Undo);
    assert_eq!(parse_line("quit\n"), ApprovalDecision::Cancel);
    assert_eq!(parse_line("xxxx\n"), ApprovalDecision::Continue);
}

// --- is_terminal -----------------------------------------------------------

#[test]
fn continue_is_only_non_terminal_state() {
    assert!(!ApprovalDecision::Continue.is_terminal());
    assert!(ApprovalDecision::Commit.is_terminal());
    assert!(ApprovalDecision::Cancel.is_terminal());
    assert!(ApprovalDecision::Approve.is_terminal());
    assert!(ApprovalDecision::Reject.is_terminal());
    assert!(ApprovalDecision::Undo.is_terminal());
    assert!(ApprovalDecision::Engage.is_terminal());
    assert!(ApprovalDecision::Clarify("?".to_owned()).is_terminal());
}

// --- MockRatifier ---------------------------------------------------------

#[test]
fn mock_keystream_replays_in_order() {
    let mut m = MockRatifier::from_keystream("eyu");
    assert_eq!(m.read_decision(), ApprovalDecision::Engage);
    assert_eq!(m.read_decision(), ApprovalDecision::Approve);
    assert_eq!(m.read_decision(), ApprovalDecision::Undo);
}

#[test]
fn mock_drained_returns_cancel_to_avoid_loop() {
    let mut m = MockRatifier::from_decisions(vec![ApprovalDecision::Commit]);
    assert_eq!(m.read_decision(), ApprovalDecision::Commit);
    assert!(m.is_drained());
    // Subsequent reads return Cancel — caller's loop terminates.
    assert_eq!(m.read_decision(), ApprovalDecision::Cancel);
    assert_eq!(m.read_decision(), ApprovalDecision::Cancel);
}

#[test]
fn mock_supports_pushing_decisions() {
    let mut m = MockRatifier::default();
    assert!(m.is_drained());
    m.push(ApprovalDecision::Commit);
    assert_eq!(m.read_decision(), ApprovalDecision::Commit);
}

#[test]
fn approval_decision_round_trips_through_serde_json() {
    let cases = [
        ApprovalDecision::Engage,
        ApprovalDecision::Commit,
        ApprovalDecision::Cancel,
        ApprovalDecision::Approve,
        ApprovalDecision::Reject,
        ApprovalDecision::Undo,
        ApprovalDecision::Clarify("hello".to_owned()),
        ApprovalDecision::Continue,
    ];
    for d in cases {
        let json = serde_json::to_string(&d).unwrap();
        let de: ApprovalDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(de, d);
    }
}
