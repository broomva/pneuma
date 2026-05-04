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
//! - Not a *full* parser. Pattern matching for slot extraction lives
//!   in downstream parsers; this crate ships **verb-text → ActId**
//!   lookup tables only (the Phase 2 "deterministic act lookup for
//!   common acts" deliverable).
//! - Not a router. Dispatch decisions live in `pneuma-router`.
//! - Not an executor. Praxis / Arcan / Spaces own that.
//!
//! ## Coverage
//!
//! v0.2 ships 31 acts grouped by domain:
//!
//! - **File** (8): open, read, rename, move, copy, delete, save, write
//! - **Workspace** (6): focus, split_pane, close_window, switch_app,
//!   navigate_back, undo
//! - **Selection** (5): select, select_all, copy, paste, cut
//! - **Agent** (4): refactor, explain, review, generate
//! - **Spaces** (3): message_send, message_react, broadcast
//! - **Inspection** (4): show_state, list_recent, search, what_is
//! - **Browser** (1): navigate
//!
//! 31 total. Each has a corresponding test in `tests/registry.rs`
//! verifying slot signatures, reversibility, and executor hints. The
//! browser namespace is the seed of step #13 of the MIL build order
//! (`MIL-PROJECT.md` §11.2) — first real OS-control act, executed via
//! AppleScript on macOS by `pneuma-praxis-bridge`.

#![doc = include_str!("../README.md")]

use std::collections::HashMap;

use pneuma_core::{
    Act, ActId, ActPrimitive, Arity, BlastRadius, ExecutorHint, ReferentType, Reversibility,
    SlotKind, SlotSignature,
};

mod agent;
mod browser;
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
    acts.extend(browser::acts());
    acts
}

// --- ActRegistry (with verb lookup) ------------------------------------------

/// Indexed wrapper over [`registry()`] — exposes lookups by ActId
/// and by **verb text** (e.g. `"rename"` → `Act { id: file.rename, ... }`).
///
/// This is the v0.2 answer to spec §11.3's "deterministic act lookup
/// for common acts." Verb aliases let downstream parsers map natural
/// utterances into act ids without an LLM.
///
/// ## Aliases
///
/// Each act gets one or more case-insensitive verb aliases. The
/// canonical aliases are seeded at construction; extend via
/// [`ActRegistry::register_alias`].
#[derive(Debug, Clone)]
pub struct ActRegistry {
    by_id: HashMap<String, Act>,
    by_verb: HashMap<String, ActId>,
}

impl Default for ActRegistry {
    fn default() -> Self {
        Self::canonical()
    }
}

impl ActRegistry {
    /// Build the canonical registry from the seed set + the standard
    /// verb aliases.
    #[must_use]
    pub fn canonical() -> Self {
        let mut r = Self::empty();
        for act in registry() {
            r.register(act);
        }
        for (verb, act_id) in canonical_verb_aliases() {
            // We just registered the canonical seed, so every alias
            // here resolves; ignore None returns rather than panicking
            // so future maintainers can drop aliases without breaking.
            let _ = r.try_register_alias(verb, act_id);
        }
        r
    }

    /// Build an empty registry. Use [`Self::canonical`] for the
    /// standard one.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            by_id: HashMap::new(),
            by_verb: HashMap::new(),
        }
    }

    /// Register an act. Replaces any existing act with the same id.
    pub fn register(&mut self, act: Act) {
        self.by_id.insert(act.id.as_str().to_owned(), act);
    }

    /// Register a verb alias. Errors if the act_id is unknown so the
    /// caller can surface bad seed data.
    pub fn try_register_alias(&mut self, verb: &str, act_id: &str) -> Result<(), AliasError> {
        if !self.by_id.contains_key(act_id) {
            return Err(AliasError::UnknownActId(act_id.to_owned()));
        }
        let id = ActId::new(act_id).map_err(|_| AliasError::UnknownActId(act_id.to_owned()))?;
        self.by_verb.insert(verb.to_lowercase(), id);
        Ok(())
    }

    /// Register a verb alias unconditionally; intended for downstream
    /// crates extending the registry. Panics if the act is unknown.
    pub fn register_alias(&mut self, verb: &str, act_id: &str) {
        self.try_register_alias(verb, act_id)
            .expect("act_id must be registered before adding aliases for it");
    }

    /// Look up an act by its identifier.
    #[must_use]
    pub fn lookup_by_id(&self, id: &ActId) -> Option<&Act> {
        self.by_id.get(id.as_str())
    }

    /// Look up an act by case-insensitive verb text.
    ///
    /// Returns `None` if the verb has no alias. The lookup uses
    /// `verb.to_lowercase().trim()` so callers don't have to
    /// normalize.
    #[must_use]
    pub fn lookup_by_verb(&self, verb: &str) -> Option<&Act> {
        let normalized = verb.trim().to_lowercase();
        let id = self.by_verb.get(&normalized)?;
        self.by_id.get(id.as_str())
    }

    /// Iterate every registered act.
    pub fn acts(&self) -> impl Iterator<Item = &Act> {
        self.by_id.values()
    }

    /// Iterate every registered (verb → act_id) alias.
    pub fn aliases(&self) -> impl Iterator<Item = (&str, &str)> {
        self.by_verb
            .iter()
            .map(|(verb, id)| (verb.as_str(), id.as_str()))
    }

    /// Number of acts registered.
    #[must_use]
    pub fn act_count(&self) -> usize {
        self.by_id.len()
    }

    /// Number of verb aliases registered.
    #[must_use]
    pub fn alias_count(&self) -> usize {
        self.by_verb.len()
    }
}

/// Errors raised by [`ActRegistry::try_register_alias`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AliasError {
    /// Cannot alias to an act that hasn't been registered yet.
    UnknownActId(String),
}

impl std::fmt::Display for AliasError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownActId(id) => write!(f, "alias points at unregistered act `{id}`"),
        }
    }
}

impl std::error::Error for AliasError {}

/// The canonical verb-alias seed table. Each entry is (verb,
/// canonical-act-id). All v0.2 acts get at least one alias; common
/// English synonyms map to the same act.
fn canonical_verb_aliases() -> &'static [(&'static str, &'static str)] {
    &[
        // file domain
        ("open", "file.open"),
        ("read", "file.read"),
        ("show", "file.read"),
        ("rename", "file.rename"),
        ("rn", "file.rename"),
        ("move", "file.move"),
        ("mv", "file.move"),
        ("copy", "file.copy"),
        ("cp", "file.copy"),
        ("duplicate", "file.copy"),
        ("delete", "file.delete"),
        ("rm", "file.delete"),
        ("remove", "file.delete"),
        ("save", "file.save"),
        ("write", "file.write"),
        // workspace
        ("focus", "workspace.focus"),
        ("split", "workspace.split_pane"),
        ("close", "workspace.close_window"),
        ("switch", "workspace.switch_app"),
        ("back", "workspace.navigate_back"),
        ("undo", "workspace.undo"),
        // selection
        ("select", "selection.select"),
        ("paste", "selection.paste"),
        ("cut", "selection.cut"),
        // agent
        ("refactor", "agent.refactor"),
        ("explain", "agent.explain"),
        ("review", "agent.review"),
        ("generate", "agent.generate"),
        // spaces
        ("send", "spaces.message_send"),
        ("react", "spaces.message_react"),
        ("broadcast", "spaces.broadcast"),
        // inspection
        ("state", "inspection.show_state"),
        ("recent", "inspection.list_recent"),
        ("search", "inspection.search"),
        ("describe", "inspection.what_is"),
        ("what", "inspection.what_is"),
        // browser
        ("navigate", "browser.navigate"),
        ("go", "browser.navigate"),
        ("browse", "browser.navigate"),
        // Note: "open" is already aliased to "file.open"; we deliberately
        // do not double-bind it here. Users who want to open a URL must
        // say "go to https://..." or "navigate to https://...". A future
        // `pneuma-resolver` will disambiguate "open https://..." once it
        // can recognize URL referents in the verb's first argument.
    ]
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
