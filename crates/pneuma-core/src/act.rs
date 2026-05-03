//! Acts — the verb registry for MIL.
//!
//! From `MIL-PROJECT.md` §10.1:
//!
//! > `act.rs` — Act registry shape: `Act`, `ActId`, `ActPrimitive`,
//! > `Arity`, `SlotKind`, `ReferentType`, `ResolvedAct`, `ExecutorHint`.
//!
//! An [`Act`] is an immutable schema: name, slot signatures,
//! intrinsic policy. A [`ResolvedAct`] is what a directive carries —
//! the act schema plus the bindings the parser made.
//!
//! ## `ActPrimitive` is forward-compat (v0.3)
//!
//! From §10.2:
//!
//! > `ActPrimitive` enum is in the schema but not load-bearing in
//! > v0.2. Forward-compatible structure for the v0.3 gesture-only
//! > language.
//!
//! v0.3 will introduce a Wierzbicka-NSM-style act-primitive vocabulary
//! (~26 atoms) so gestures can compose acts without naming every verb.
//! For v0.2 the field is present but every act maps to
//! [`ActPrimitive::Custom`] — verbs are registered by string name.

use serde::{Deserialize, Serialize};

use crate::error::{ContractError, Result};
use crate::policy::{BlastRadius, Reversibility};
use crate::referent::{ReferentType, ReferentValue};

// --- ActId -------------------------------------------------------------------

/// A stable string identifier for an act. Conventionally
/// `dotted.namespace.verb` — `file.rename`, `agent.refactor`,
/// `workspace.split_pane`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActId(String);

impl ActId {
    /// Construct, rejecting empty input.
    pub fn new(raw: impl Into<String>) -> Result<Self> {
        let trimmed = raw.into().trim().to_owned();
        if trimmed.is_empty() {
            return Err(ContractError::EmptyIdentifier { field: "ActId" });
        }
        Ok(Self(trimmed))
    }

    /// Borrow the underlying string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// --- ActPrimitive (v0.3 forward-compat) --------------------------------------

/// Compositional act primitives — Wierzbicka-NSM-flavored atoms.
/// **Not load-bearing in v0.2.**
///
/// v0.3 will populate this enum with the canonical ~26 primitives
/// (DO, HAPPEN, MOVE, KNOW, THINK, FEEL, WANT, SAY, ...). For v0.2
/// every act is [`ActPrimitive::Custom`]; the field is here so the
/// wire format is forward-compatible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ActPrimitive {
    /// User-named verb. The default in v0.2.
    Custom,
    /// "DO" — agent performs an action. v0.3.
    Do,
    /// "MOVE" — relocate a referent. v0.3.
    Move,
    /// "SAY" — speech act / message. v0.3.
    Say,
    /// "KNOW" — query / inspection. v0.3.
    Know,
}

// --- Arity -------------------------------------------------------------------

/// Whether a slot is required, optional, or variadic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Arity {
    /// Slot must be bound before [`crate::Directive::try_finalize`].
    Required,
    /// Slot may be unbound; defaults can be supplied.
    Optional,
    /// Slot accepts a list of values. Length 0 allowed unless
    /// [`Arity::Variadic`] is paired with a separate "non-empty"
    /// wrapper at the act layer.
    Variadic,
}

// --- SlotKind ----------------------------------------------------------------

/// What kind of slot — a referent, a modifier, a free-form payload, etc.
///
/// In v0.2 the load-bearing kind is [`SlotKind::Referent`]; modifier
/// slots are typically attached at the directive level rather than the
/// act level. The richer kinds are here for v0.3.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SlotKind {
    /// Slot accepts a referent of the declared type.
    Referent(ReferentType),
    /// Slot accepts a string payload (e.g. `new_name` for `file.rename`).
    String,
    /// Slot accepts a numeric payload.
    Number,
    /// Slot accepts a boolean payload.
    Boolean,
}

// --- SlotSignature -----------------------------------------------------------

/// A single slot's signature in an act schema.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SlotSignature {
    /// Slot name. Used as the binding key.
    pub name: String,
    /// What kind of value the slot accepts.
    pub kind: SlotKind,
    /// Required / optional / variadic.
    pub arity: Arity,
    /// Free-form description for diagnostics and HUD rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl SlotSignature {
    /// Construct.
    pub fn new(
        name: impl Into<String>,
        kind: SlotKind,
        arity: Arity,
    ) -> Result<Self> {
        let n = name.into().trim().to_owned();
        if n.is_empty() {
            return Err(ContractError::EmptyIdentifier { field: "SlotSignature.name" });
        }
        Ok(Self { name: n, kind, arity, description: None })
    }

    /// Attach a description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

// --- ExecutorHint ------------------------------------------------------------

/// Hint to the router about which executor an act prefers. The router
/// honors this when the policy envelope's `permitted_executors` list
/// allows it; otherwise the policy wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutorHint {
    /// Prefer Praxis (deterministic).
    Praxis,
    /// Prefer Arcan (agent).
    Arcan,
    /// Prefer Spaces (multi-agent).
    Spaces,
    /// No preference — router decides on context.
    Any,
}

// --- Act ---------------------------------------------------------------------

/// An act schema — the static description of a verb.
///
/// Acts are *registered*, not constructed per directive. The act
/// registry crate (`pneuma-acts`) provides the canonical set;
/// downstream crates may define additional acts.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Act {
    /// Act identifier.
    pub id: ActId,
    /// Forward-compat primitive tag (always `Custom` in v0.2).
    pub primitive: ActPrimitive,
    /// Slot signatures, in declaration order.
    pub slots: Vec<SlotSignature>,
    /// Intrinsic reversibility.
    pub reversibility: Reversibility,
    /// Intrinsic blast radius.
    pub blast_radius: BlastRadius,
    /// Preferred executor.
    pub executor_hint: ExecutorHint,
    /// Reverse-action recipe identifier — looked up by Praxis at
    /// dispatch time. `None` for irreversible acts; `Some` for
    /// reversible Praxis acts. Arcan acts may carry a recipe ID that
    /// the agent fills in at completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverse_recipe: Option<String>,
    /// Free-form description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Act {
    /// Find a slot by name.
    #[must_use]
    pub fn slot(&self, name: &str) -> Option<&SlotSignature> {
        self.slots.iter().find(|s| s.name == name)
    }
}

// --- ResolvedAct (act + bindings) --------------------------------------------

/// A bound slot value — kind-specific payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ResolvedSlotValue {
    /// Referent payload.
    Referent(ReferentValue),
    /// String payload.
    String(String),
    /// Numeric payload.
    Number(f64),
    /// Boolean payload.
    Boolean(bool),
}

impl ResolvedSlotValue {
    /// Type-check this value against a slot signature.
    ///
    /// `clippy::match_same_arms` is allowed — the scalar arms each
    /// match a *different* `(value, kind)` pair, even though they all
    /// short-circuit to `true`. Merging them with `|` would obscure
    /// the intent.
    #[allow(clippy::match_same_arms)]
    pub fn matches_kind(&self, kind: &SlotKind) -> bool {
        match (self, kind) {
            (Self::Referent(rv), SlotKind::Referent(rt)) => {
                *rt == crate::referent::ReferentType::Any || rv.type_of() == *rt
            }
            (Self::String(_), SlotKind::String) => true,
            (Self::Number(_), SlotKind::Number) => true,
            (Self::Boolean(_), SlotKind::Boolean) => true,
            _ => false,
        }
    }

    /// The referent-type of this value if it's a Referent slot;
    /// returns `None` for scalar slots.
    #[must_use]
    pub fn referent_type(&self) -> Option<crate::referent::ReferentType> {
        match self {
            Self::Referent(rv) => Some(rv.type_of()),
            _ => None,
        }
    }
}

/// A single bound slot in a [`ResolvedAct`] — name + value +
/// provenance. Unlike `Tagged<T>`, the slot tracks its name (since
/// the act schema indexes by name).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedSlot {
    /// Slot name (must match a `SlotSignature::name` on the parent act).
    pub name: String,
    /// Bound value.
    pub value: ResolvedSlotValue,
    /// Provenance for the binding.
    pub provenance: crate::provenance::Provenance,
}

impl ResolvedSlot {
    /// Construct.
    pub fn new(
        name: impl Into<String>,
        value: ResolvedSlotValue,
        provenance: crate::provenance::Provenance,
    ) -> Result<Self> {
        let n = name.into().trim().to_owned();
        if n.is_empty() {
            return Err(ContractError::EmptyIdentifier { field: "ResolvedSlot.name" });
        }
        Ok(Self { name: n, value, provenance })
    }
}

/// An act schema together with its bound slot values.
///
/// This is what a [`crate::Directive`] carries. Validation
/// (slot completeness, type matching) lives on the directive's
/// `try_finalize`, not on this struct — `ResolvedAct` is constructible
/// in any state, and the directive lifecycle gates dispatch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedAct {
    /// The act schema.
    pub act: Act,
    /// Bound slot values, indexed by name.
    pub bindings: Vec<ResolvedSlot>,
}

impl ResolvedAct {
    /// Construct an empty resolution (no slots bound yet).
    #[must_use]
    pub fn empty(act: Act) -> Self {
        Self { act, bindings: Vec::new() }
    }

    /// Bind a slot. Replaces any existing binding for the same slot
    /// name. Does *not* type-check — that happens in
    /// [`crate::Directive::try_finalize`].
    pub fn bind(&mut self, slot: ResolvedSlot) {
        if let Some(existing) = self.bindings.iter_mut().find(|s| s.name == slot.name) {
            *existing = slot;
        } else {
            self.bindings.push(slot);
        }
    }

    /// Look up a binding by slot name.
    #[must_use]
    pub fn binding(&self, name: &str) -> Option<&ResolvedSlot> {
        self.bindings.iter().find(|b| b.name == name)
    }

    /// Iterate the act's *required* slots that lack a binding.
    pub fn unbound_required(&self) -> impl Iterator<Item = &SlotSignature> {
        self.act.slots.iter().filter(|sig| {
            sig.arity == Arity::Required && self.binding(&sig.name).is_none()
        })
    }
}
