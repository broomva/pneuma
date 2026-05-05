//! Tests for [`parse_utterance`] — the v0.2 deterministic parser.

use pneuma_acts::ActRegistry;
use pneuma_demo::{ParseError, parse_utterance};

// --- Successful parses ----------------------------------------------------

#[test]
fn rename_to_new_extracts_new_name() {
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("rename to bar.txt", &r).unwrap();
    assert_eq!(parsed.act_id.as_str(), "file.rename");
    assert_eq!(
        parsed.payload_slots,
        vec![("new_name".to_owned(), "bar.txt".to_owned())]
    );
}

#[test]
fn rename_it_to_new_extracts_new_name() {
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("rename it to bar.txt", &r).unwrap();
    assert_eq!(parsed.act_id.as_str(), "file.rename");
    assert_eq!(
        parsed.payload_slots,
        vec![("new_name".to_owned(), "bar.txt".to_owned())]
    );
}

#[test]
fn rn_alias_resolves_to_rename() {
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("rn bar.txt", &r).unwrap();
    assert_eq!(parsed.act_id.as_str(), "file.rename");
    assert_eq!(
        parsed.payload_slots,
        vec![("new_name".to_owned(), "bar.txt".to_owned())]
    );
}

#[test]
fn copy_to_destination_extracts_destination() {
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("copy to /tmp/dest.txt", &r).unwrap();
    assert_eq!(parsed.act_id.as_str(), "file.copy");
    assert_eq!(
        parsed.payload_slots,
        vec![("destination".to_owned(), "/tmp/dest.txt".to_owned())]
    );
}

#[test]
fn delete_has_no_payload_slots() {
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("delete", &r).unwrap();
    assert_eq!(parsed.act_id.as_str(), "file.delete");
    assert!(parsed.payload_slots.is_empty());
}

#[test]
fn delete_alias_rm_resolves() {
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("rm", &r).unwrap();
    assert_eq!(parsed.act_id.as_str(), "file.delete");
}

#[test]
fn open_has_no_payload_slots() {
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("open", &r).unwrap();
    assert_eq!(parsed.act_id.as_str(), "file.open");
    assert!(parsed.payload_slots.is_empty());
}

#[test]
fn undo_resolves_to_workspace_undo() {
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("undo", &r).unwrap();
    assert_eq!(parsed.act_id.as_str(), "workspace.undo");
}

// --- Errors ----------------------------------------------------------------

#[test]
fn empty_utterance_errors() {
    let r = ActRegistry::canonical();
    assert!(matches!(parse_utterance("", &r), Err(ParseError::Empty)));
    assert!(matches!(
        parse_utterance("   \t\n", &r),
        Err(ParseError::Empty)
    ));
}

#[test]
fn unknown_verb_errors() {
    let r = ActRegistry::canonical();
    let err = parse_utterance("teleport to mars", &r).unwrap_err();
    assert!(matches!(err, ParseError::UnknownVerb { ref verb } if verb == "teleport"));
}

#[test]
fn rename_without_new_name_errors() {
    let r = ActRegistry::canonical();
    let err = parse_utterance("rename to", &r).unwrap_err();
    assert!(matches!(
        err,
        ParseError::MissingSlot { ref slot, .. } if slot == "new_name"
    ));
}

#[test]
fn rename_filters_filler_words_in_terse_form() {
    // The parser strips deictic / filler words ("it", "the") so a
    // single content word becomes the new_name.
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("rename it the file", &r).unwrap();
    assert_eq!(parsed.act_id.as_str(), "file.rename");
    assert_eq!(parsed.payload_slots[0].1, "file");
}

#[test]
fn rename_with_no_args_at_all_errors() {
    let r = ActRegistry::canonical();
    let err = parse_utterance("rename", &r).unwrap_err();
    assert!(matches!(err, ParseError::MissingSlot { .. }));
}

#[test]
fn rename_with_only_filler_errors() {
    let r = ActRegistry::canonical();
    let err = parse_utterance("rename it the", &r).unwrap_err();
    assert!(matches!(err, ParseError::MissingSlot { .. }));
}

#[test]
fn copy_without_destination_errors() {
    let r = ActRegistry::canonical();
    let err = parse_utterance("copy", &r).unwrap_err();
    assert!(matches!(
        err,
        ParseError::MissingSlot { ref slot, .. } if slot == "destination"
    ));
}

// --- Case insensitivity (verb lookup is case-insensitive) ----------------

#[test]
fn rename_uppercase_works() {
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("RENAME to BAR.txt", &r).unwrap();
    assert_eq!(parsed.act_id.as_str(), "file.rename");
    assert_eq!(parsed.payload_slots[0].1, "BAR.txt");
}

// --- Multi-word destinations preserved ------------------------------------

#[test]
fn rename_to_multiword_preserves_spaces() {
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("rename to my file.txt", &r).unwrap();
    assert_eq!(parsed.payload_slots[0].1, "my file.txt");
}

// --- Utterance is echoed --------------------------------------------------

#[test]
fn parsed_utterance_echoes_input() {
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("  rename to bar.txt  ", &r).unwrap();
    assert_eq!(parsed.utterance, "rename to bar.txt");
}

// --- browser.navigate (step #13) ------------------------------------------

#[test]
fn navigate_to_url_extracts_url() {
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("navigate to https://example.com", &r).unwrap();
    assert_eq!(parsed.act_id.as_str(), "browser.navigate");
    assert_eq!(
        parsed.payload_slots,
        vec![("url".to_owned(), "https://example.com".to_owned())]
    );
}

#[test]
fn go_to_url_extracts_url() {
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("go to https://example.com", &r).unwrap();
    assert_eq!(parsed.act_id.as_str(), "browser.navigate");
    assert_eq!(
        parsed.payload_slots,
        vec![("url".to_owned(), "https://example.com".to_owned())]
    );
}

#[test]
fn browse_url_without_to_extracts_url() {
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("browse https://example.com", &r).unwrap();
    assert_eq!(parsed.act_id.as_str(), "browser.navigate");
    assert_eq!(
        parsed.payload_slots,
        vec![("url".to_owned(), "https://example.com".to_owned())]
    );
}

#[test]
fn go_url_without_to_extracts_url() {
    // "go example.com" should work too — "to" is optional for navigate.
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("go example.com", &r).unwrap();
    assert_eq!(parsed.act_id.as_str(), "browser.navigate");
    assert_eq!(
        parsed.payload_slots,
        vec![("url".to_owned(), "example.com".to_owned())]
    );
}

#[test]
fn navigate_with_filler_words_strips_them() {
    // "go to it" with no URL = missing slot. Filler-only utterance.
    let r = ActRegistry::canonical();
    let err = parse_utterance("go to", &r).unwrap_err();
    assert!(matches!(err, ParseError::MissingSlot { ref slot, .. } if slot == "url"));
}

#[test]
fn navigate_without_url_after_to_errors() {
    let r = ActRegistry::canonical();
    let err = parse_utterance("navigate to ", &r).unwrap_err();
    assert!(matches!(err, ParseError::MissingSlot { ref slot, .. } if slot == "url"));
}

#[test]
fn navigate_alone_errors() {
    // Bare "navigate" is recognized but URL is required.
    let r = ActRegistry::canonical();
    let err = parse_utterance("navigate", &r).unwrap_err();
    assert!(matches!(err, ParseError::MissingSlot { ref slot, .. } if slot == "url"));
}

// --- workspace.switch_app (step #14) -------------------------------------

#[test]
fn switch_to_app_extracts_target() {
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("switch to Safari", &r).unwrap();
    assert_eq!(parsed.act_id.as_str(), "workspace.switch_app");
    assert_eq!(
        parsed.payload_slots,
        vec![("target".to_owned(), "Safari".to_owned())]
    );
}

#[test]
fn switch_app_without_to_extracts_target() {
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("switch Safari", &r).unwrap();
    assert_eq!(parsed.act_id.as_str(), "workspace.switch_app");
    assert_eq!(
        parsed.payload_slots,
        vec![("target".to_owned(), "Safari".to_owned())]
    );
}

#[test]
fn switch_to_multiword_app_preserves_spaces() {
    let r = ActRegistry::canonical();
    let parsed = parse_utterance("switch to Visual Studio Code", &r).unwrap();
    assert_eq!(parsed.act_id.as_str(), "workspace.switch_app");
    assert_eq!(
        parsed.payload_slots,
        vec![("target".to_owned(), "Visual Studio Code".to_owned())]
    );
}

#[test]
fn switch_alone_errors() {
    let r = ActRegistry::canonical();
    let err = parse_utterance("switch", &r).unwrap_err();
    assert!(matches!(err, ParseError::MissingSlot { ref slot, .. } if slot == "target"));
}

#[test]
fn switch_to_with_no_app_errors() {
    let r = ActRegistry::canonical();
    let err = parse_utterance("switch to ", &r).unwrap_err();
    assert!(matches!(err, ParseError::MissingSlot { ref slot, .. } if slot == "target"));
}
