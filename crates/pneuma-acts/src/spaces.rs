//! Spaces-domain acts. Three acts: message_send, message_react, broadcast.
//!
//! These dispatch through the Spaces protocol. Send and broadcast are
//! Irreversible (External blast — message went out the wire); react
//! is Costly (can be removed but is observable in the meantime).

use pneuma_core::{Act, BlastRadius, ExecutorHint, ReferentType, Reversibility};

use crate::{act, req_referent, req_string};

/// Three Spaces-domain acts.
#[must_use]
pub fn acts() -> Vec<Act> {
    vec![
        // spaces.message_send — send a message. Irreversible.
        act(
            "spaces.message_send",
            vec![
                req_referent(
                    "channel",
                    ReferentType::Any,
                    "Channel or DM target identifier",
                ),
                req_string("body", "Message body"),
            ],
            Reversibility::Irreversible,
            BlastRadius::External,
            ExecutorHint::Spaces,
            None,
            "Send a message in a Spaces channel. Irreversible.",
        ),
        // spaces.message_react — emoji reaction.
        act(
            "spaces.message_react",
            vec![
                req_string("message_id", "Message identifier (Spaces protocol-defined)"),
                req_string("emoji", "Reaction emoji"),
            ],
            Reversibility::Costly,
            BlastRadius::External,
            ExecutorHint::Spaces,
            Some("spaces.unreact"),
            "Add a reaction to a Spaces message.",
        ),
        // spaces.broadcast — send to everyone. Irreversible.
        act(
            "spaces.broadcast",
            vec![req_string("body", "Broadcast body")],
            Reversibility::Irreversible,
            BlastRadius::External,
            ExecutorHint::Spaces,
            None,
            "Broadcast a message to all Spaces. Irreversible.",
        ),
    ]
}
