//! # pneuma-core
//!
//! The directive contract — the **language** of MIL.
//!
//! From `MIL-PROJECT.md` §2:
//!
//! > The contract is the language. The recognition layer can be as learned
//! > and opaque as needed; the directive types are typed, inspectable, and
//! > stable across model versions. The model is the *implementation* of
//! > the language; the contract is the language itself.
//!
//! This crate defines:
//!
//! - [`Directive<S>`] — a typestate-parameterized lifecycle. The phantom
//!   `S` is one of [`Composing`], [`Ready`], [`Proposed`], or
//!   [`Committed`]. The state machine is enforced at compile time;
//!   data-driven slot validation is enforced at runtime in
//!   [`Directive::try_finalize`].
//! - [`PolicyEnvelope`] — the safety contract: reversibility, blast
//!   radius, ratification, confidence threshold, validity window,
//!   permitted executors, redaction rules.
//! - [`Confidence`] — calibrated confidence with per-slot decomposition,
//!   honest `is_calibrated` flag, and producer attribution.
//! - [`Tagged<T>`] — the universal provenance wrapper. Every typed value
//!   in the contract carries its confidence, source tokens, and binding
//!   kind. Nothing is bare.
//! - [`Referent`], [`Modifier`], [`Act`], [`AgentResponse`] — the leaf
//!   types that flesh out the directive.
//!
//! ## The five categorical guarantees
//!
//! The contract enforces five properties (`MIL-PROJECT.md` §6.3):
//!
//! 1. No directive dispatches without all required slots bound.
//! 2. No directive dispatches with mismatched referent types.
//! 3. No directive dispatches below its policy envelope's confidence
//!    threshold.
//! 4. No irreversible-or-large-blast directive bypasses ratification.
//! 5. Every committed directive carries the workspace snapshot it was
//!    committed against.
//!
//! These five are what make MIL safe in a way pure-LLM systems aren't.
//!
//! ## Cross-crate compatibility with `sensorium-core`
//!
//! [`provenance::ContextSnapshotId`] is byte-compatible with
//! `sensorium_core::WorkspaceSnapshotId` (both are `UUIDv7`). The two
//! crates do not depend on each other; a future `pneuma-sensorium`
//! bridge crate will provide `From`/`Into` impls. The wire format is
//! byte-identical today.
//!
//! Similarly, [`referent::AppId`], [`referent::WindowId`],
//! [`referent::FileRef`], [`referent::SymbolRef`], and
//! [`referent::SelectionRef`] mirror the structurally-equivalent types
//! in `sensorium_core::entity`. The duplication is deliberate for v0.2;
//! a future migration will collapse to a shared definition.
//!
//! See [MIL-PROJECT.md][spec] §11 for the full build order.
//!
//! [spec]: https://github.com/broomva/pneuma

#![doc = include_str!("../README.md")]

pub mod act;
pub mod confidence;
pub mod directive;
pub mod error;
pub mod modifier;
pub mod policy;
pub mod provenance;
pub mod referent;
pub mod response;

// --- Public re-exports: the load-bearing surface. ----------------------------

pub use act::{
    Act, ActId, ActPrimitive, Arity, ExecutorHint, ResolvedAct, ResolvedSlot, SlotKind,
    SlotSignature,
};
pub use confidence::{Confidence, ConfidenceProducer, ConfidenceScore};
pub use directive::{
    Composing, Committed, Directive, DirectiveId, DirectiveState, Proposed, Ready, SpeechAct,
};
pub use error::{ContractError, Result};
pub use modifier::{Modifier, ModifierKind, TimeWindowSpec};
pub use policy::{
    BlastRadius, ExecutorKind, PolicyEnvelope, RedactionRule, Reversibility, Timestamp,
};
pub use provenance::{BindingKind, ContextRef, ContextSnapshotId, Provenance, Tagged, TokenRef};
pub use referent::{
    AnaphorRef, AppId, FileRef, ReferentType, ReferentValue, SelectionRef, SpatialAnchor,
    SymbolRef, TextSpan, WindowId,
};
pub use response::{
    AgentResponse, ClarifyOption, CostClass, DirectiveError, DirectiveResult, PlannedStep,
    ProgressUpdate, ProposalKind, StepStatus,
};

/// The grammar version this crate ships against.
///
/// Wire-compatible with `sensorium-core` `GRAMMAR_VERSION`. Kept in
/// lockstep — when one bumps, the other must bump.
pub const GRAMMAR_VERSION: &str = "0.2.0";
