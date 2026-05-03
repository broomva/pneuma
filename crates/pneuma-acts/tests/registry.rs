//! Acts registry — coverage and structural properties.
//!
//! Properties under test:
//!
//! - The seed registry contains exactly 30 acts.
//! - Each act has a unique [`ActId`].
//! - File / workspace / selection / agent / spaces / inspection counts
//!   match the spec (8 / 6 / 5 / 4 / 3 / 4).
//! - Irreversible acts always have `requires_ratify` after running
//!   through `PolicyEnvelope::intrinsic`.
//! - Every reversible act has a `reverse_recipe` set.
//! - Read-only acts (Free + no reverse_recipe needed) are correctly
//!   classified.

use std::collections::HashSet;

use pneuma_acts::registry;
use pneuma_core::{Act, BlastRadius, PolicyEnvelope, Reversibility};

#[test]
fn seed_registry_has_thirty_acts() {
    let acts = registry();
    assert_eq!(acts.len(), 30, "seed registry should have exactly 30 acts");
}

#[test]
fn all_act_ids_are_unique() {
    let acts = registry();
    let ids: HashSet<_> = acts.iter().map(|a| a.id.as_str().to_owned()).collect();
    assert_eq!(ids.len(), acts.len(), "ActIds must be unique across the registry");
}

#[test]
fn domain_counts_match_spec() {
    let acts = registry();
    let count = |prefix: &str| -> usize {
        acts.iter().filter(|a| a.id.as_str().starts_with(prefix)).count()
    };
    assert_eq!(count("file."), 8, "file domain has 8 acts");
    assert_eq!(count("workspace."), 6, "workspace domain has 6 acts");
    assert_eq!(count("selection."), 5, "selection domain has 5 acts");
    assert_eq!(count("agent."), 4, "agent domain has 4 acts");
    assert_eq!(count("spaces."), 3, "spaces domain has 3 acts");
    assert_eq!(count("inspection."), 4, "inspection domain has 4 acts");
}

#[test]
fn irreversible_acts_force_ratify_via_intrinsic_policy() {
    let acts: Vec<Act> = registry()
        .into_iter()
        .filter(|a| a.reversibility == Reversibility::Irreversible)
        .collect();
    assert!(!acts.is_empty(), "spec includes irreversible acts");
    for a in acts {
        let policy = PolicyEnvelope::intrinsic(a.reversibility, a.blast_radius);
        assert!(
            policy.requires_ratify,
            "irreversible act {} must require ratify",
            a.id.as_str()
        );
    }
}

#[test]
fn reversible_costly_acts_have_reverse_recipe() {
    for a in registry() {
        if a.reversibility == Reversibility::Costly {
            assert!(
                a.reverse_recipe.is_some(),
                "Costly act {} must declare reverse_recipe",
                a.id.as_str()
            );
        }
    }
}

#[test]
fn irreversible_acts_have_no_reverse_recipe() {
    for a in registry() {
        if a.reversibility == Reversibility::Irreversible {
            assert!(
                a.reverse_recipe.is_none(),
                "Irreversible act {} should not declare a reverse_recipe",
                a.id.as_str()
            );
        }
    }
}

#[test]
fn external_blast_acts_force_ratify() {
    for a in registry() {
        if a.blast_radius == BlastRadius::External {
            let policy = PolicyEnvelope::intrinsic(a.reversibility, a.blast_radius);
            assert!(
                policy.requires_ratify,
                "External-blast act {} must require ratify",
                a.id.as_str()
            );
        }
    }
}

#[test]
fn agent_acts_route_to_arcan() {
    use pneuma_core::ExecutorHint;
    for a in registry() {
        if a.id.as_str().starts_with("agent.") {
            assert_eq!(
                a.executor_hint,
                ExecutorHint::Arcan,
                "agent.* acts route to Arcan; got {:?} for {}",
                a.executor_hint,
                a.id.as_str()
            );
        }
    }
}

#[test]
fn spaces_acts_route_to_spaces() {
    use pneuma_core::ExecutorHint;
    for a in registry() {
        if a.id.as_str().starts_with("spaces.") {
            assert_eq!(
                a.executor_hint,
                ExecutorHint::Spaces,
                "spaces.* acts route to Spaces; got {:?} for {}",
                a.executor_hint,
                a.id.as_str()
            );
        }
    }
}

#[test]
fn every_act_has_a_description() {
    for a in registry() {
        assert!(
            a.description.is_some(),
            "Act {} should have a description for HUD rendering",
            a.id.as_str()
        );
    }
}

#[test]
fn file_delete_is_irreversible_and_ratifies() {
    let acts = registry();
    let delete = acts
        .iter()
        .find(|a| a.id.as_str() == "file.delete")
        .expect("file.delete is canonical");
    assert_eq!(delete.reversibility, Reversibility::Irreversible);
    let policy = PolicyEnvelope::intrinsic(delete.reversibility, delete.blast_radius);
    assert!(policy.requires_ratify);
}

#[test]
fn file_rename_has_reverse_recipe_named_rename_back() {
    let acts = registry();
    let rename = acts
        .iter()
        .find(|a| a.id.as_str() == "file.rename")
        .expect("file.rename is canonical");
    assert_eq!(rename.reverse_recipe.as_deref(), Some("rename_back"));
}
