//! Inspection-domain acts. Four acts: show_state, list_recent, search, what_is.
//!
//! All read-only, all Free, all Local. Inspection is the "interrogative"
//! speech-act class — request information, no side effects.

use pneuma_core::{Act, BlastRadius, ExecutorHint, ReferentType, Reversibility};

use crate::{act, opt_string, req_referent, req_string};

/// Four inspection-domain acts.
#[must_use]
pub fn acts() -> Vec<Act> {
    vec![
        // inspection.show_state — render workspace state in HUD.
        act(
            "inspection.show_state",
            vec![opt_string(
                "scope",
                "Scope: workspace | agent | session (default workspace)",
            )],
            Reversibility::Free,
            BlastRadius::Local,
            ExecutorHint::Praxis,
            None,
            "Render current workspace state in the HUD.",
        ),
        // inspection.list_recent — list recent activity.
        act(
            "inspection.list_recent",
            vec![opt_string(
                "kind",
                "Activity kind: files | windows | directives | all (default all)",
            )],
            Reversibility::Free,
            BlastRadius::Local,
            ExecutorHint::Praxis,
            None,
            "List recent activity from the substrate's ring.",
        ),
        // inspection.search — query the personal corpus.
        act(
            "inspection.search",
            vec![req_string("query", "Search query string")],
            Reversibility::Free,
            BlastRadius::Local,
            ExecutorHint::Any,
            None,
            "Search the personal corpus.",
        ),
        // inspection.what_is — describe a referent.
        act(
            "inspection.what_is",
            vec![req_referent(
                "target",
                ReferentType::Any,
                "Referent to describe",
            )],
            Reversibility::Free,
            BlastRadius::Local,
            ExecutorHint::Any,
            None,
            "Describe a referent.",
        ),
    ]
}
