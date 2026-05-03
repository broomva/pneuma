//! [`ActRegistry`] lookup tests — by id and by verb alias.

use pneuma_acts::{ActRegistry, AliasError};
use pneuma_core::ActId;

// --- Basic shape ----------------------------------------------------------

#[test]
fn canonical_registry_has_all_thirty_acts() {
    let r = ActRegistry::canonical();
    assert_eq!(r.act_count(), 30);
}

#[test]
fn empty_registry_starts_empty() {
    let r = ActRegistry::empty();
    assert_eq!(r.act_count(), 0);
    assert_eq!(r.alias_count(), 0);
}

// --- Lookup by id --------------------------------------------------------

#[test]
fn lookup_by_id_returns_known_act() {
    let r = ActRegistry::canonical();
    let id = ActId::new("file.rename").unwrap();
    let act = r.lookup_by_id(&id).expect("file.rename canonical");
    assert_eq!(act.id.as_str(), "file.rename");
}

#[test]
fn lookup_by_id_missing_returns_none() {
    let r = ActRegistry::canonical();
    let id = ActId::new("never.exists").unwrap();
    assert!(r.lookup_by_id(&id).is_none());
}

// --- Lookup by verb ------------------------------------------------------

#[test]
fn lookup_by_verb_resolves_canonical_verbs() {
    let r = ActRegistry::canonical();
    assert_eq!(
        r.lookup_by_verb("rename").map(|a| a.id.as_str()),
        Some("file.rename")
    );
    assert_eq!(
        r.lookup_by_verb("delete").map(|a| a.id.as_str()),
        Some("file.delete")
    );
    assert_eq!(
        r.lookup_by_verb("open").map(|a| a.id.as_str()),
        Some("file.open")
    );
    assert_eq!(
        r.lookup_by_verb("undo").map(|a| a.id.as_str()),
        Some("workspace.undo")
    );
}

#[test]
fn lookup_by_verb_resolves_synonyms_to_same_act() {
    let r = ActRegistry::canonical();
    let canonical = r.lookup_by_verb("delete").map(|a| a.id.as_str());
    assert_eq!(canonical, Some("file.delete"));
    assert_eq!(r.lookup_by_verb("rm").map(|a| a.id.as_str()), canonical);
    assert_eq!(r.lookup_by_verb("remove").map(|a| a.id.as_str()), canonical);
}

#[test]
fn lookup_by_verb_is_case_insensitive() {
    let r = ActRegistry::canonical();
    assert_eq!(
        r.lookup_by_verb("RENAME").map(|a| a.id.as_str()),
        Some("file.rename")
    );
    assert_eq!(
        r.lookup_by_verb("Rename").map(|a| a.id.as_str()),
        Some("file.rename")
    );
}

#[test]
fn lookup_by_verb_trims_whitespace() {
    let r = ActRegistry::canonical();
    assert_eq!(
        r.lookup_by_verb("  rename  ").map(|a| a.id.as_str()),
        Some("file.rename")
    );
}

#[test]
fn lookup_by_verb_unknown_returns_none() {
    let r = ActRegistry::canonical();
    assert!(r.lookup_by_verb("teleport").is_none());
}

// --- Alias registration ---------------------------------------------------

#[test]
fn try_register_alias_errors_on_unknown_act() {
    let mut r = ActRegistry::canonical();
    let result = r.try_register_alias("foo", "never.exists");
    assert!(matches!(result, Err(AliasError::UnknownActId(_))));
}

#[test]
fn try_register_alias_succeeds_for_known_act() {
    let mut r = ActRegistry::canonical();
    r.try_register_alias("rename2", "file.rename").unwrap();
    assert_eq!(
        r.lookup_by_verb("rename2").map(|a| a.id.as_str()),
        Some("file.rename")
    );
}

#[test]
fn alias_count_grows_with_registrations() {
    let mut r = ActRegistry::empty();
    let initial = r.alias_count();
    // Need to register an act first.
    let registry_acts = pneuma_acts::registry();
    let rename = registry_acts
        .into_iter()
        .find(|a| a.id.as_str() == "file.rename")
        .unwrap();
    r.register(rename);
    r.try_register_alias("foo", "file.rename").unwrap();
    r.try_register_alias("bar", "file.rename").unwrap();
    assert_eq!(r.alias_count(), initial + 2);
}

// --- Aliases iterator -----------------------------------------------------

#[test]
fn aliases_iter_yields_all_registered() {
    let r = ActRegistry::canonical();
    let aliases: Vec<_> = r.aliases().collect();
    // Plenty of canonical aliases — at least the size of the
    // canonical_verb_aliases table. (Lower bound only since the
    // table may grow.)
    assert!(aliases.len() >= 30);
}
