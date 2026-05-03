//! # pneuma-acts
//!
//! The canonical MIL act registry — data only.
//!
//! From `MIL-PROJECT.md` §11.3 (Phase 2):
//!
//! > `pneuma-act-registry`: deterministic act lookup for common acts.
//!
//! And from the brief that opened the Tier 2 build:
//!
//! > Concrete act registry. Define ~30 acts with their slot signatures,
//! > reversibility, blast radius, and executor hints. Tedious but it's
//! > the data the entire system runs on.
//!
//! ## What this crate is
//!
//! Data. The canonical seed set of acts that the Tier 2 build needs.
//! Each act is a static `fn() -> Act`, called once at registry
//! assembly time. No globals, no `lazy_static` — the consumer holds the
//! `Vec<Act>` and decides what to do with it.
//!
//! ## What this crate is NOT
//!
//! - Not a parser. Verb-text → ActId mapping lives in `pneuma-parser`.
//! - Not a router. Dispatch decisions live in `pneuma-router`.
//! - Not an executor. Praxis / Arcan / Spaces own that.
//!
//! ## Coverage
//!
//! v0.2 ships ~30 acts grouped by domain:
//!
//! - **File** (8): open, read, rename, move, copy, delete, save, write
//! - **Workspace** (6): focus, split_pane, close_window, switch_app,
//!   navigate_back, undo
//! - **Selection** (5): select, select_all, copy, paste, cut
//! - **Agent** (4): refactor, explain, review, generate
//! - **Spaces** (3): message_send, message_react, broadcast
//! - **Inspection** (4): show_state, list_recent, search, what_is
//!
//! 30 total. Each has a corresponding test in `tests/registry.rs`
//! verifying slot signatures, reversibility, and executor hints.

#![doc = include_str!("../README.md")]

use pneuma_core::{
    Act, ActId, ActPrimitive, Arity, BlastRadius, ExecutorHint, ReferentType, Reversibility,
    SlotKind, SlotSignature,
};

mod agent;
mod file;
mod inspection;
mod selection;
mod spaces;
mod workspace;

// --- Public registry assembly ------------------------------------------------

/// The canonical seed set of acts. Returns a `Vec<Act>` containing
/// every act this crate registers.
///
/// Consumers build their own working registry from this seed plus any
/// downstream-specific acts.
#[must_use]
pub fn registry() -> Vec<Act> {
    let mut acts = Vec::new();
    acts.extend(file::acts());
    acts.extend(workspace::acts());
    acts.extend(selection::acts());
    acts.extend(agent::acts());
    acts.extend(spaces::acts());
    acts.extend(inspection::acts());
    acts
}

// --- Internal helpers --------------------------------------------------------

/// Build an [`Act`] with the boilerplate filled in. Used by the per-domain
/// modules to keep registrations terse.
pub(crate) fn act(
    id: &str,
    slots: Vec<SlotSignature>,
    reversibility: Reversibility,
    blast_radius: BlastRadius,
    executor_hint: ExecutorHint,
    reverse_recipe: Option<&str>,
    description: &str,
) -> Act {
    Act {
        id: ActId::new(id).expect("registry act ids are non-empty"),
        primitive: ActPrimitive::Custom,
        slots,
        reversibility,
        blast_radius,
        executor_hint,
        reverse_recipe: reverse_recipe.map(String::from),
        description: Some(description.to_owned()),
    }
}

/// Build a [`SlotSignature`] tersely. Required referent of the given
/// type, with description.
pub(crate) fn req_referent(name: &str, ty: ReferentType, description: &str) -> SlotSignature {
    SlotSignature::new(name, SlotKind::Referent(ty), Arity::Required)
        .expect("registry slot names are non-empty")
        .with_description(description)
}

/// Build a required string slot signature.
pub(crate) fn req_string(name: &str, description: &str) -> SlotSignature {
    SlotSignature::new(name, SlotKind::String, Arity::Required)
        .expect("registry slot names are non-empty")
        .with_description(description)
}

/// Build an optional string slot signature.
pub(crate) fn opt_string(name: &str, description: &str) -> SlotSignature {
    SlotSignature::new(name, SlotKind::String, Arity::Optional)
        .expect("registry slot names are non-empty")
        .with_description(description)
}
