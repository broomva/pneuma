//! Directive — the typestate-parameterized lifecycle.
//!
//! From `MIL-PROJECT.md` §6.2:
//!
//! ```text
//! Directive<Composing>
//!     │  add tokens, modifiers, slot bindings as recognition proceeds
//!     ▼
//! Directive<Composing>::try_finalize(context, policy, confidence)
//!     │  required slots filled? types valid? confidence ≥ threshold?
//!     ▼
//! Directive<Ready>
//!     │  branch on policy.requires_ratify
//!     ├─ no  → .commit()  → Directive<Committed>
//!     └─ yes → .propose() → Directive<Proposed>
//!                               │
//!                               ├─ .ratify()             → Directive<Committed>
//!                               ├─ .reject_for_amendment → Directive<Composing>
//!                               └─ .reject_and_cancel    → cancelled
//! ```
//!
//! The phantom `S` is one of [`Composing`], [`Ready`], [`Proposed`],
//! [`Committed`]. The state machine is enforced at compile time —
//! `commit()` lives only on `Directive<Ready>` so calling it on a
//! `Directive<Composing>` is a compile error.
//!
//! Data-driven validation (slot completeness, type matching, confidence
//! threshold) happens at runtime in [`Directive::try_finalize`].

use std::marker::PhantomData;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::act::ResolvedAct;
use crate::confidence::Confidence;
use crate::error::ContractError;
use crate::modifier::Modifier;
use crate::policy::PolicyEnvelope;
use crate::provenance::{ContextRef, Generation, TokenRef};

// --- DirectiveId -------------------------------------------------------------

/// `UUIDv7` newtype identifying a directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DirectiveId(Uuid);

impl DirectiveId {
    /// Mint a fresh `UUIDv7`.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Wrap an existing `Uuid`.
    #[must_use]
    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    /// Inner UUID.
    #[must_use]
    pub fn into_inner(self) -> Uuid {
        self.0
    }
}

impl Default for DirectiveId {
    fn default() -> Self {
        Self::new()
    }
}

// --- Typestate markers -------------------------------------------------------

/// Phantom marker: directive is being assembled. Slots may be unbound;
/// confidence and policy unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Composing;

/// Phantom marker: directive has been finalized. Slots bound,
/// confidence cleared the threshold, policy envelope computed. Ready
/// to commit (if no ratify) or propose (if ratify).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Ready;

/// Phantom marker: directive has been proposed. Awaiting user
/// ratification. May ratify → Committed, reject for amendment →
/// Composing, or reject-and-cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Proposed;

/// Phantom marker: directive has committed. Immutable; eligible for
/// dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Committed;

// --- DirectiveState (runtime tag) --------------------------------------------

/// Runtime mirror of the typestate phantom — for diagnostics, journals,
/// and APIs that need to handle a directive without parameterizing on
/// its type-state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DirectiveState {
    /// `Composing` typestate.
    Composing,
    /// `Ready` typestate.
    Ready,
    /// `Proposed` typestate.
    Proposed,
    /// `Committed` typestate.
    Committed,
    /// User cancelled (terminal).
    Cancelled,
}

// --- SpeechAct ---------------------------------------------------------------

/// What kind of speech act the directive represents.
///
/// From the spec: `Directive` (do this), `Interrogative` (tell me),
/// `Commissive` (I will), `Expressive` (I feel), `Assertive` (this is
/// true). v0.2 dispatch is concentrated on `Directive` and
/// `Interrogative`; the others are present for completeness and v0.3
/// expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SpeechAct {
    /// "Do X" — request action.
    Directive,
    /// "What is X?" / "Show me X" — request information.
    Interrogative,
    /// "I will X" — commitment.
    Commissive,
    /// "I feel X" — expressive (feedback to the agent).
    Expressive,
    /// "X is true" — assertion (typically used in correction).
    Assertive,
}

// --- Directive<S> ------------------------------------------------------------

/// The typestate-parameterized directive.
///
/// Construct in `Composing` via [`Directive::new`]; transition through
/// the state machine via the methods on each typestate impl block
/// below. Compile-time enforcement of state invariants; runtime
/// enforcement of data invariants in [`Directive::try_finalize`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Directive<S> {
    /// Directive identity.
    pub id: DirectiveId,
    /// Grammar version this directive was constructed against. Lockstep
    /// with [`crate::GRAMMAR_VERSION`].
    pub grammar_version: String,
    /// Speech-act tag.
    pub speech_act: SpeechAct,
    /// Resolved act + bindings.
    pub act: ResolvedAct,
    /// Modifiers attached at the directive level.
    pub modifiers: Vec<Modifier>,
    /// Original utterance text, if any. `None` for gesture-only
    /// directives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utterance: Option<String>,
    /// Source tokens — substrate-emitted primitives this directive
    /// was assembled from.
    pub tokens: Vec<TokenRef>,
    /// Workspace snapshot the directive was committed against. `None`
    /// in `Composing` / `Ready` states; populated at finalization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextRef>,
    /// Policy envelope. `None` in `Composing`; computed at finalization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<PolicyEnvelope>,
    /// Joined confidence. `None` in `Composing`; populated at
    /// finalization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
    /// Runtime state mirror.
    pub state: DirectiveState,
    /// Wall-clock creation time.
    pub created_at: DateTime<Utc>,
    /// Wall-clock commit time. `None` until `Committed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed_at: Option<DateTime<Utc>>,
    /// Stream generation that produced this directive, when the
    /// directive originated from a streaming substrate (voice STT,
    /// BCI, gaze). `None` for non-streaming construction paths
    /// (typed utterance, gesture-only).
    ///
    /// Wire-compatible with `sensorium_core::Generation`. Carried
    /// through every state transition so downstream stages can
    /// route on a stable per-utterance identity — a `Cancelled(g)`
    /// on the source stream drops every directive tagged with `g`.
    ///
    /// See [`Directive::with_generation`] (Composing path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<Generation>,
    /// Phantom — the typestate.
    #[serde(skip)]
    _state: PhantomData<S>,
}

// --- Generic accessor --------------------------------------------------------

impl<S> Directive<S> {
    /// The stream generation that produced this directive, if any.
    ///
    /// Available in every typestate. Set on `Composing` via
    /// [`Directive::with_generation`]; propagated unchanged through
    /// every transition.
    #[must_use]
    pub fn generation(&self) -> Option<Generation> {
        self.generation
    }
}

// --- Composing impl ----------------------------------------------------------

impl Directive<Composing> {
    /// Construct a new directive in the `Composing` state.
    #[must_use]
    pub fn new(speech_act: SpeechAct, act: ResolvedAct) -> Self {
        Self {
            id: DirectiveId::new(),
            grammar_version: crate::GRAMMAR_VERSION.to_owned(),
            speech_act,
            act,
            modifiers: Vec::new(),
            utterance: None,
            tokens: Vec::new(),
            context: None,
            policy: None,
            confidence: None,
            state: DirectiveState::Composing,
            created_at: Utc::now(),
            committed_at: None,
            generation: None,
            _state: PhantomData,
        }
    }

    /// Attach the stream [`Generation`] that produced this directive.
    ///
    /// Used by streaming substrates (voice STT, BCI, gaze) to mark
    /// the directive's source utterance. Propagates unchanged through
    /// every state transition; downstream stages can drop derived
    /// work tagged with this generation when the producer emits a
    /// `Cancelled(g)`.
    ///
    /// For non-streaming construction (typed `MIL_UTTERANCE`,
    /// programmatic), leave it unset — the field stays `None`.
    pub fn with_generation(mut self, generation: Generation) -> Self {
        self.generation = Some(generation);
        self
    }

    /// Add a modifier in-place.
    pub fn with_modifier(mut self, modifier: Modifier) -> Self {
        self.modifiers.push(modifier);
        self
    }

    /// Add a source token in-place.
    pub fn with_token(mut self, token: TokenRef) -> Self {
        self.tokens.push(token);
        self
    }

    /// Attach an utterance.
    pub fn with_utterance(mut self, utterance: impl Into<String>) -> Self {
        self.utterance = Some(utterance.into());
        self
    }

    /// Bind a slot value on the resolved act.
    pub fn bind_slot(mut self, slot: crate::act::ResolvedSlot) -> Self {
        self.act.bind(slot);
        self
    }

    /// Attempt to finalize. Validates:
    ///
    /// 1. All required slots are bound (Guarantee 1).
    /// 2. All bound values match their slot type signatures (Guarantee 2).
    /// 3. Effective confidence ≥ effective policy threshold (Guarantee 3).
    /// 4. `valid_until`, if set, has not passed.
    ///
    /// On success, returns a `Directive<Ready>`. On failure, returns
    /// `(self, ContractError)` so the caller can amend and retry — the
    /// directive value is preserved. The `Err` variant is intentionally
    /// large because it carries the whole directive; this is the
    /// amend-and-retry contract, and we silence
    /// `clippy::result_large_err` accordingly.
    #[allow(clippy::result_large_err)]
    pub fn try_finalize(
        mut self,
        context: ContextRef,
        policy: PolicyEnvelope,
        confidence: Confidence,
    ) -> std::result::Result<Directive<Ready>, (Self, ContractError)> {
        // Guarantee 1: required slots bound.
        // Clone the missing-slot name out of the borrow before
        // building the error so the borrow on `self.act` ends.
        let missing_slot: Option<(String, String)> = self
            .act
            .unbound_required()
            .next()
            .map(|sig| (self.act.act.id.as_str().to_owned(), sig.name.clone()));
        if let Some((act_id, slot)) = missing_slot {
            return Err((self, ContractError::UnboundRequiredSlot { act_id, slot }));
        }

        // Guarantee 2: type-check every binding.
        for binding in &self.act.bindings {
            let Some(sig) = self.act.act.slot(&binding.name) else {
                continue; // Extra bindings ignored; future-proof.
            };
            if !binding.value.matches_kind(&sig.kind) {
                let (expected, actual) = match (&sig.kind, binding.value.referent_type()) {
                    (crate::act::SlotKind::Referent(expected), Some(actual)) => (*expected, actual),
                    _ => (
                        crate::referent::ReferentType::Any,
                        binding
                            .value
                            .referent_type()
                            .unwrap_or(crate::referent::ReferentType::Any),
                    ),
                };
                let err = ContractError::TypeMismatch {
                    slot: binding.name.clone(),
                    expected,
                    actual,
                };
                return Err((self, err));
            }
        }

        // Guarantee 4 (validity): if valid_until set and past, expire.
        if let Some(deadline) = policy.valid_until
            && Utc::now() > deadline
        {
            let err = ContractError::Expired {
                expired_at: deadline,
            };
            return Err((self, err));
        }

        // Guarantee 3: confidence threshold.
        let effective_threshold = policy.effective_threshold(confidence.is_calibrated());
        let effective_confidence = confidence.effective_value();
        if effective_confidence < effective_threshold {
            let err = ContractError::ConfidenceBelowThreshold {
                confidence: effective_confidence,
                threshold: policy.min_confidence,
                effective_threshold,
            };
            return Err((self, err));
        }

        // Promote to Ready.
        self.context = Some(context);
        self.policy = Some(policy);
        self.confidence = Some(confidence);
        self.state = DirectiveState::Ready;

        Ok(Directive {
            id: self.id,
            grammar_version: self.grammar_version,
            speech_act: self.speech_act,
            act: self.act,
            modifiers: self.modifiers,
            utterance: self.utterance,
            tokens: self.tokens,
            context: self.context,
            policy: self.policy,
            confidence: self.confidence,
            state: self.state,
            created_at: self.created_at,
            committed_at: self.committed_at,
            generation: self.generation,
            _state: PhantomData,
        })
    }
}

// --- Ready impl --------------------------------------------------------------

impl Directive<Ready> {
    /// Commit directly. **Errors** with [`ContractError::RatifyRequired`]
    /// if the policy envelope requires ratification (Guarantee 4); use
    /// [`Self::propose`] in that case.
    ///
    /// On failure the directive is returned so the caller can re-route
    /// to `propose()`. Same `result_large_err` rationale as
    /// [`Composing::try_finalize`][Composing].
    #[allow(clippy::result_large_err)]
    pub fn commit(self) -> std::result::Result<Directive<Committed>, (Self, ContractError)> {
        if let Some(policy) = &self.policy
            && policy.requires_ratify
        {
            return Err((self, ContractError::RatifyRequired));
        }
        Ok(self.commit_unchecked())
    }

    /// Propose for ratification — required for Irreversible /
    /// large-blast directives.
    #[must_use]
    pub fn propose(mut self) -> Directive<Proposed> {
        self.state = DirectiveState::Proposed;
        Directive {
            id: self.id,
            grammar_version: self.grammar_version,
            speech_act: self.speech_act,
            act: self.act,
            modifiers: self.modifiers,
            utterance: self.utterance,
            tokens: self.tokens,
            context: self.context,
            policy: self.policy,
            confidence: self.confidence,
            state: self.state,
            created_at: self.created_at,
            committed_at: self.committed_at,
            generation: self.generation,
            _state: PhantomData,
        }
    }

    /// Internal: commit without the ratify gate. Shared between
    /// `commit()` and `Proposed::ratify()`.
    fn commit_unchecked(mut self) -> Directive<Committed> {
        self.state = DirectiveState::Committed;
        self.committed_at = Some(Utc::now());
        Directive {
            id: self.id,
            grammar_version: self.grammar_version,
            speech_act: self.speech_act,
            act: self.act,
            modifiers: self.modifiers,
            utterance: self.utterance,
            tokens: self.tokens,
            context: self.context,
            policy: self.policy,
            confidence: self.confidence,
            state: self.state,
            created_at: self.created_at,
            committed_at: self.committed_at,
            generation: self.generation,
            _state: PhantomData,
        }
    }
}

// --- Proposed impl -----------------------------------------------------------

impl Directive<Proposed> {
    /// Ratify the proposal — promotes to `Committed`.
    #[must_use]
    pub fn ratify(self) -> Directive<Committed> {
        let ready = Directive::<Ready> {
            id: self.id,
            grammar_version: self.grammar_version,
            speech_act: self.speech_act,
            act: self.act,
            modifiers: self.modifiers,
            utterance: self.utterance,
            tokens: self.tokens,
            context: self.context,
            policy: self.policy,
            confidence: self.confidence,
            state: DirectiveState::Ready,
            created_at: self.created_at,
            committed_at: self.committed_at,
            generation: self.generation,
            _state: PhantomData,
        };
        ready.commit_unchecked()
    }

    /// Reject for amendment — return to Composing so the user can
    /// add / fix slots.
    #[must_use]
    pub fn reject_for_amendment(mut self) -> Directive<Composing> {
        self.state = DirectiveState::Composing;
        self.policy = None;
        self.confidence = None;
        Directive {
            id: self.id,
            grammar_version: self.grammar_version,
            speech_act: self.speech_act,
            act: self.act,
            modifiers: self.modifiers,
            utterance: self.utterance,
            tokens: self.tokens,
            context: self.context,
            policy: self.policy,
            confidence: self.confidence,
            state: self.state,
            created_at: self.created_at,
            committed_at: self.committed_at,
            generation: self.generation,
            _state: PhantomData,
        }
    }

    /// Reject and cancel — terminal. The directive is dropped (caller
    /// loses the value).
    pub fn reject_and_cancel(self) {
        // Drop. The Cancelled state is recorded in the caller's
        // journal via the directive's id, not in the typestate.
        let _ = self;
    }
}

// --- Committed impl ----------------------------------------------------------

impl Directive<Committed> {
    /// Borrow the policy envelope (always present for `Committed`).
    #[must_use]
    pub fn policy(&self) -> &PolicyEnvelope {
        self.policy
            .as_ref()
            .expect("Committed directive must have policy")
    }

    /// Borrow the confidence (always present for `Committed`).
    #[must_use]
    pub fn confidence(&self) -> &Confidence {
        self.confidence
            .as_ref()
            .expect("Committed directive must have confidence")
    }

    /// Borrow the context ref (always present for `Committed`).
    #[must_use]
    pub fn context(&self) -> &ContextRef {
        self.context
            .as_ref()
            .expect("Committed directive must have context")
    }
}
