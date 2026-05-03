//! Agent-domain acts. Four acts: refactor, explain, review, generate.
//!
//! These dispatch through Arcan, not Praxis. Reverse actions are
//! agent-supplied at completion time; the registry records this with
//! a `Some(recipe)` placeholder that Arcan fills in.

use pneuma_core::{Act, BlastRadius, ExecutorHint, ReferentType, Reversibility};

use crate::{act, req_referent, req_string};

/// Four agent-domain acts.
#[must_use]
pub fn acts() -> Vec<Act> {
    vec![
        // agent.refactor — apply a refactor to a symbol or selection.
        act(
            "agent.refactor",
            vec![
                req_referent(
                    "target",
                    ReferentType::Any,
                    "What to refactor (symbol or selection)",
                ),
                req_string("instruction", "Refactor instruction"),
            ],
            Reversibility::Costly,
            BlastRadius::Project,
            ExecutorHint::Arcan,
            Some("agent.reverse_diff"),
            "Apply a code refactor via the agent runtime.",
        ),
        // agent.explain — read-only, no reverse.
        act(
            "agent.explain",
            vec![req_referent(
                "target",
                ReferentType::Any,
                "What to explain",
            )],
            Reversibility::Free,
            BlastRadius::Local,
            ExecutorHint::Arcan,
            None, // read-only — no side effect
            "Ask the agent to explain a referent.",
        ),
        // agent.review — read-only.
        act(
            "agent.review",
            vec![req_referent(
                "target",
                ReferentType::Any,
                "What to review",
            )],
            Reversibility::Free,
            BlastRadius::Local,
            ExecutorHint::Arcan,
            None,
            "Ask the agent to review a referent.",
        ),
        // agent.generate — create new content. Reverse: delete.
        act(
            "agent.generate",
            vec![req_string("instruction", "Generation instruction")],
            Reversibility::Costly,
            BlastRadius::Project,
            ExecutorHint::Arcan,
            Some("agent.delete_generated"),
            "Generate new content via the agent runtime.",
        ),
    ]
}
