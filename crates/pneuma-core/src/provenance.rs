//! Provenance — audit-trail metadata attached to every typed value in
//! the contract.
//!
//! From `MIL-PROJECT.md` §10.2:
//!
//! > `Tagged<T>` is the universal currency. Every typed value carries
//! > its confidence, source tokens, and binding kind. Nothing in the
//! > contract is bare.
//!
//! `pneuma-core` is the downstream end of the substrate's provenance
//! chain. `sensorium-core` produces `Tagged<T>` at observation time;
//! `pneuma-core` re-wraps with per-slot bookkeeping and joins
//! confidence across slots.
//!
//! ## What's in a [`Provenance`]
//!
//! - [`Provenance::tokens`] — the substrate tokens this value was bound
//!   from. May span multiple modalities (gaze + speech + workspace).
//! - [`Provenance::binding`] — *how* the binding was made: deterministic
//!   hit-test, anaphora resolution, model interpretation, etc. Pneuma's
//!   confidence join is binding-kind-aware.
//! - [`Provenance::observed_at`] — when the underlying observation
//!   happened. Monotonically lags `Tagged::value`.
//!
//! ## Cross-crate compatibility
//!
//! [`ContextSnapshotId`] is byte-compatible with
//! `sensorium_core::WorkspaceSnapshotId`. Conversion via the inner
//! `Uuid` is straightforward; a future `pneuma-sensorium` glue crate
//! will expose `From`/`Into` impls.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// --- TokenRef ----------------------------------------------------------------

/// A reference to a substrate token (sensorium [`PrimitiveToken`]) the
/// directive was bound from.
///
/// We don't carry the full token here — just enough to look it up in
/// the journal. `id` is opaque (string for crate-isolation; in practice
/// a UUIDv7 from the substrate).
///
/// [`PrimitiveToken`]: https://github.com/broomva/sensorium
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TokenRef {
    /// Opaque token identifier. Resolves through the journal.
    pub id: String,
    /// Coarse modality tag for diagnostics — `"voice"`, `"gaze"`,
    /// `"workspace"`, `"gesture"`. Free-form; the binder may produce
    /// novel tags.
    pub modality: String,
    /// When the underlying observation happened.
    pub observed_at: DateTime<Utc>,
}

impl TokenRef {
    /// Construct a token reference.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        modality: impl Into<String>,
        observed_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id: id.into(),
            modality: modality.into(),
            observed_at,
        }
    }
}

// --- BindingKind -------------------------------------------------------------

/// How a value was bound to its slot.
///
/// Pneuma's confidence join is binding-kind-aware: a deterministic
/// hit-test produces higher joined confidence than a model
/// interpretation, even when the per-slot scores are equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BindingKind {
    /// Direct deterministic match — gaze hit-test, exact verb lookup,
    /// bound-by-typing. Highest trust.
    Deterministic,
    /// Anaphora resolution by recent-history lookup ("the file" → most
    /// recent file referent). Medium trust; depends on history scope.
    Anaphora,
    /// Workspace-context resolution ("here" → focused window).
    /// Medium-high trust.
    WorkspaceContext,
    /// Cross-modal binding by temporal coincidence (deictic +
    /// fixation). Medium trust; depends on the binding window.
    CrossModal,
    /// Predication-content interpretation by an LLM. Trust depends on
    /// calibration of the model's logprobs / structured-output validity.
    ModelInterpretation,
    /// Default value supplied by the act registry — neither observed
    /// nor inferred, just declared. Use sparingly.
    Default,
    /// User explicitly typed / dictated the value verbatim. High trust
    /// for the value, but the *intent* still needs ratification per
    /// policy envelope.
    UserExplicit,
}

impl BindingKind {
    /// Coarse trust multiplier for confidence join. The substrate
    /// honors this when computing weakest-slot confidence; downstream
    /// callers should treat these as advisory.
    #[must_use]
    pub fn trust_multiplier(self) -> f32 {
        match self {
            Self::Deterministic | Self::UserExplicit => 1.0,
            Self::WorkspaceContext => 0.95,
            Self::Anaphora => 0.9,
            Self::CrossModal => 0.85,
            Self::ModelInterpretation => 0.75,
            Self::Default => 0.6,
        }
    }
}

// --- Generation --------------------------------------------------------------

/// Monotonic stream-update generation, mirroring
/// `sensorium_core::Generation`.
///
/// Carried on [`crate::Directive`] when the directive originated from a
/// streaming substrate (voice STT, BCI, future gaze). Lets downstream
/// stages route on a stable per-utterance identity: speculative
/// directives derived from a `Partial` transcript share the source
/// generation, so a `Cancelled(g)` on the producer stream cleanly
/// drops every directive tagged with `g`.
///
/// **Wire-compatible with `sensorium_core::Generation`.** Both crates
/// wrap a `u64` and serialize transparently as a bare number. The
/// orphan rule prevents `From` impls in either crate; the bridge
/// function `bridge_generation` in `pneuma-resolver` (which depends on
/// both) does the conversion explicitly.
///
/// Not `#[non_exhaustive]` — this is a value newtype. Adding fields
/// would be a breaking change anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Generation(u64);

impl Generation {
    /// The first generation a fresh producer mints. Matches
    /// `sensorium_core::Generation::INITIAL`.
    pub const INITIAL: Self = Self(0);

    /// Construct from a raw counter. Intended for tests, replay, and
    /// the cross-substrate bridge in `pneuma-resolver`.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Raw counter value.
    #[must_use]
    pub const fn into_inner(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for Generation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// --- ContextSnapshotId -------------------------------------------------------

/// `UUIDv7` newtype identifying a workspace snapshot the directive was
/// committed against.
///
/// **Wire-compatible with `sensorium_core::WorkspaceSnapshotId`.** Both
/// crates wrap a `UUIDv7`; a future `pneuma-sensorium` bridge crate
/// will provide `From`/`Into` impls. For now, callers can convert via
/// the inner `Uuid` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContextSnapshotId(Uuid);

impl ContextSnapshotId {
    /// Mint a fresh `UUIDv7`-backed snapshot ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Wrap an existing `Uuid` (replay machinery, tests, cross-crate
    /// bridge from `sensorium_core::WorkspaceSnapshotId`).
    #[must_use]
    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    /// Inner UUID, for callers needing to bridge to other ID systems.
    #[must_use]
    pub fn into_inner(self) -> Uuid {
        self.0
    }
}

impl Default for ContextSnapshotId {
    fn default() -> Self {
        Self::new()
    }
}

// --- ContextRef --------------------------------------------------------------

/// Reference to the workspace snapshot a directive was committed
/// against.
///
/// Carries the snapshot ID plus a `valid_until` extracted from the
/// policy envelope. Executors compare the snapshot ID at dispatch
/// time to detect drift; if the substrate has rebuilt, the directive
/// must re-resolve or be refused.
///
/// Guarantee 5 of the five categorical guarantees: every committed
/// directive carries this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContextRef {
    /// The snapshot ID. Resolves through `sensorium-core` /
    /// the workspace observer.
    pub snapshot_id: ContextSnapshotId,
    /// When the snapshot was captured.
    pub captured_at: DateTime<Utc>,
}

impl ContextRef {
    /// Construct a reference to a freshly-captured snapshot.
    #[must_use]
    pub fn new(snapshot_id: ContextSnapshotId, captured_at: DateTime<Utc>) -> Self {
        Self {
            snapshot_id,
            captured_at,
        }
    }
}

// --- Provenance --------------------------------------------------------------

/// Per-value provenance: the tokens, the binding kind, the observation
/// time. Carried by every [`Tagged<T>`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// The substrate tokens this value was bound from. May be empty
    /// for `Default`-bound slots.
    pub tokens: Vec<TokenRef>,
    /// How the binding was made.
    pub binding: BindingKind,
    /// When the underlying observation happened. For multi-token
    /// bindings, the *latest* observation time.
    pub observed_at: DateTime<Utc>,
}

impl Provenance {
    /// Construct a provenance record.
    #[must_use]
    pub fn new(tokens: Vec<TokenRef>, binding: BindingKind, observed_at: DateTime<Utc>) -> Self {
        Self {
            tokens,
            binding,
            observed_at,
        }
    }

    /// A "default value" provenance — no tokens, `BindingKind::Default`,
    /// observed-now. Used for act-registry-supplied defaults.
    #[must_use]
    pub fn default_value(observed_at: DateTime<Utc>) -> Self {
        Self {
            tokens: Vec::new(),
            binding: BindingKind::Default,
            observed_at,
        }
    }

    /// `true` if the binding kind is high-trust (Deterministic or
    /// UserExplicit). Convenience for downstream pattern-matches.
    #[must_use]
    pub fn is_high_trust(&self) -> bool {
        matches!(
            self.binding,
            BindingKind::Deterministic | BindingKind::UserExplicit
        )
    }
}

// --- Tagged<T> ---------------------------------------------------------------

/// Universal provenance wrapper — every typed value in the contract
/// flows as `Tagged<T>`.
///
/// `Tagged<T>` exists so consumers can pattern-match on the *value*
/// without losing the audit trail. Equality compares both axes; to
/// compare just the value, dereference: `tagged.value == other.value`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tagged<T> {
    /// The bound value.
    pub value: T,
    /// Audit-trail metadata.
    pub provenance: Provenance,
}

impl<T> Tagged<T> {
    /// Construct a tagged value.
    #[must_use]
    pub fn new(value: T, provenance: Provenance) -> Self {
        Self { value, provenance }
    }

    /// Map the inner value, preserving provenance.
    ///
    /// Used when a downstream component refines a value (e.g. resolves
    /// a partial referent to a fully-qualified one) without
    /// re-observing it. The provenance keeps pointing at the original
    /// tokens — the refinement does not add evidence.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Tagged<U> {
        Tagged {
            value: f(self.value),
            provenance: self.provenance,
        }
    }

    /// Borrow the value with provenance attached.
    #[must_use]
    pub fn as_ref(&self) -> Tagged<&T> {
        Tagged {
            value: &self.value,
            provenance: self.provenance.clone(),
        }
    }
}
