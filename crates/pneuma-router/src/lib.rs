//! # pneuma-router
//!
//! The MIL router — a pure function from `(Directive<Committed>,
//! WorkspaceContext)` to `Dispatch`. No I/O, no async, no state.
//!
//! From the brief that opened the Tier 2 build:
//!
//! > Build the router as a pure function. The shape: a single function
//! > `pneuma_router::dispatch(directive, context) -> Dispatch`, where
//! > `Dispatch` is an enum of `{Praxis(PraxisCall), Arcan(AgentPrompt),
//! > Spaces(SpacesMessage), Refuse(Reason)}`. Pure. Tested. No I/O.
//!
//! ## What the router does
//!
//! Given a [`Directive`] in the `Committed` typestate and a
//! [`WorkspaceContext`] from `sensorium-core`, the router decides:
//!
//! 1. Has the policy envelope's `valid_until` passed? Refuse if so.
//! 2. Is the executor hint permitted by `permitted_executors`? If
//!    [`PolicyEnvelope::permitted_executors`] is empty, default to
//!    "any executor that matches the act's hint." Otherwise enforce.
//! 3. Build the payload for the chosen executor:
//!    - **Praxis**: `PraxisCall { act_id, slots, reverse_recipe }`
//!    - **Arcan**: `AgentPrompt { act_id, instruction, slots }`
//!    - **Spaces**: `SpacesMessage { act_id, channel, body }`
//!    - **Custom**: `CustomPayload { act_id, kind, data }`
//!
//! 4. Return [`Dispatch`].
//!
//! ## What the router does NOT do
//!
//! - **Drift detection.** This is an architectural decision surfaced
//!   by the Tier 2 build (see `docs/mil/research-synthesis-2026-05-03.md`
//!   risk #6). Snapshot IDs in `sensorium-core` are minted fresh per
//!   capture (sortable identity-of-record), so equality of IDs is
//!   *not* equality of observed state. Genuine drift detection
//!   requires `Arc::ptr_eq` on the underlying `Arc<WorkspaceState>`,
//!   which is a stateful concern that does not survive a pure
//!   function boundary. The router exposes [`drift_detected`] as an
//!   explicit caller-side helper that takes both the original
//!   committed snapshot and a fresh one — callers run it before
//!   handing the directive to [`dispatch`].
//! - Slot resolution. Slots must already be bound in the directive.
//! - Anaphora resolution. That's `pneuma-resolver`.
//! - Confidence computation. That's already enforced at finalize.
//! - Actually executing anything. The executor crates do that.
//!
//! ## Why pure
//!
//! Two reasons:
//!
//! 1. **Testability**. We can unit-test every dispatch path with
//!    hand-built directives and contexts. No mocks, no fixtures
//!    beyond what the contract types provide.
//! 2. **Future concurrency**. A pure router can be called from any
//!    thread, fanned out, retried. Adding state would force a
//!    locking story we don't need.

#![doc = include_str!("../README.md")]

use serde::{Deserialize, Serialize};

use pneuma_core::act::ResolvedSlotValue;
use pneuma_core::{
    ActId, BindingKind, Committed, Confidence, Directive, ExecutorHint, ExecutorKind,
    PolicyEnvelope,
};
use sensorium_core::WorkspaceContext;

// --- Dispatch payload types --------------------------------------------------

/// Praxis (deterministic) dispatch payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PraxisCall {
    /// Which act to invoke.
    pub act_id: ActId,
    /// Slot bindings as `(name, value)` pairs. Praxis decodes per-act.
    pub slots: Vec<(String, ResolvedSlotValue)>,
    /// Reverse-action recipe identifier, if the act is reversible.
    pub reverse_recipe: Option<String>,
}

/// Arcan (agent runtime) dispatch payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentPrompt {
    /// Which act the agent is fulfilling.
    pub act_id: ActId,
    /// The natural-language instruction to hand to the agent. Built
    /// from the directive's utterance + structured slot context.
    pub instruction: String,
    /// Slot bindings, for the agent to inspect.
    pub slots: Vec<(String, ResolvedSlotValue)>,
}

/// Spaces (multi-agent) dispatch payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpacesMessage {
    /// Which act this dispatches.
    pub act_id: ActId,
    /// Channel target identifier (free-form; Spaces protocol-defined).
    pub channel: Option<String>,
    /// Message body.
    pub body: String,
}

/// Custom (pluggable) dispatch payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomPayload {
    /// Which act this dispatches.
    pub act_id: ActId,
    /// Kind tag for the custom executor.
    pub kind: String,
    /// Slot bindings.
    pub slots: Vec<(String, ResolvedSlotValue)>,
}

// --- Refusal reasons ---------------------------------------------------------

/// Why the router refused to dispatch a directive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum RefusalReason {
    /// `policy.valid_until` has passed.
    PolicyExpired,
    /// The act's preferred executor is not in
    /// [`PolicyEnvelope::permitted_executors`].
    ExecutorNotPermitted {
        /// What the act wanted.
        wanted: ExecutorHint,
        /// What the policy allows.
        permitted: Vec<ExecutorKind>,
    },
    /// `policy.permitted_executors` is non-empty but the act has no
    /// matching hint. Should be rare; usually a misconfigured policy.
    NoCompatibleExecutor,
    /// Act has no executor hint and policy allows none.
    NoExecutorAvailable,
}

// --- Dispatch ----------------------------------------------------------------

/// The router's decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Dispatch {
    /// Route to Praxis.
    Praxis(PraxisCall),
    /// Route to Arcan.
    Arcan(AgentPrompt),
    /// Route to Spaces.
    Spaces(SpacesMessage),
    /// Custom executor.
    Custom(CustomPayload),
    /// Refuse — explain why.
    Refuse(RefusalReason),
}

impl Dispatch {
    /// `true` if the dispatch was refused.
    #[must_use]
    pub fn is_refused(&self) -> bool {
        matches!(self, Self::Refuse(_))
    }
}

// --- The router function ----------------------------------------------------

/// Route a committed directive against the current workspace context.
///
/// **Pure.** No I/O, no allocation beyond the returned `Dispatch`.
///
/// ## Drift detection
///
/// Compares `directive.context().snapshot_id` to a fresh snapshot
/// derived from `context`. If they differ, returns
/// [`Dispatch::Refuse(RefusalReason::SnapshotDrift)`]. Drift means the
/// substrate observed a change between commit and dispatch — the
/// executor must not act on a stale view.
///
/// ## Validity window
///
/// If `policy.valid_until` is set and has passed (per `chrono::Utc::now`),
/// returns [`Dispatch::Refuse(RefusalReason::PolicyExpired)`]. The
/// `valid_until` is checked against current wall-clock; the `chrono`
/// call is the only impure-feeling thing in the router. We could pass
/// `now: DateTime<Utc>` as a parameter to make it fully pure; we do
/// that as `dispatch_at` below.
///
/// ## Executor selection
///
/// The act's [`Act::executor_hint`] is the preferred destination. If
/// [`PolicyEnvelope::permitted_executors`] is non-empty, the chosen
/// executor must be in the list. Otherwise the hint wins.
///
/// `ExecutorHint::Any` falls through to Praxis by default for v0.2;
/// downstream callers may override via the policy's permitted list.
#[must_use]
pub fn dispatch(directive: &Directive<Committed>, context: &WorkspaceContext) -> Dispatch {
    dispatch_at(directive, context, chrono::Utc::now())
}

/// Like [`dispatch`] but takes an explicit `now` timestamp, making
/// the router fully pure and easy to test under controlled time.
///
/// The `_context` parameter is unused in v0.2 but anticipates v0.3+
/// in-router resolution (anaphora, locus disambiguation). Callers
/// should still pass the current substrate so the API is stable
/// across versions.
#[must_use]
pub fn dispatch_at(
    directive: &Directive<Committed>,
    _context: &WorkspaceContext,
    now: chrono::DateTime<chrono::Utc>,
) -> Dispatch {
    let policy = directive.policy();

    // Step 1: validity window.
    if let Some(deadline) = policy.valid_until
        && now > deadline
    {
        return Dispatch::Refuse(RefusalReason::PolicyExpired);
    }

    // Step 2: executor selection.
    // (Drift detection is the caller's responsibility — see
    // `drift_detected` and the module docs.)
    let act = &directive.act.act;
    let chosen = choose_executor(act.executor_hint, policy);
    let chosen = match chosen {
        Ok(k) => k,
        Err(reason) => return Dispatch::Refuse(reason),
    };

    // Step 4: build the payload.
    let slots: Vec<(String, ResolvedSlotValue)> = directive
        .act
        .bindings
        .iter()
        .map(|b| (b.name.clone(), b.value.clone()))
        .collect();

    match chosen {
        ExecutorKind::Praxis => Dispatch::Praxis(PraxisCall {
            act_id: act.id.clone(),
            slots,
            reverse_recipe: act.reverse_recipe.clone(),
        }),
        ExecutorKind::Arcan => Dispatch::Arcan(AgentPrompt {
            act_id: act.id.clone(),
            instruction: build_arcan_instruction(directive),
            slots,
        }),
        ExecutorKind::Spaces => Dispatch::Spaces(build_spaces_message(directive)),
        ExecutorKind::Custom => Dispatch::Custom(CustomPayload {
            act_id: act.id.clone(),
            kind: act.id.as_str().split('.').next().unwrap_or("custom").to_owned(),
            slots,
        }),
        // ExecutorKind is #[non_exhaustive] in pneuma-core. Future-added
        // executor variants land here as Refuse(NoCompatibleExecutor)
        // until the router knows how to build their payload.
        _ => Dispatch::Refuse(RefusalReason::NoCompatibleExecutor),
    }
}

// --- Helpers -----------------------------------------------------------------

fn choose_executor(
    hint: ExecutorHint,
    policy: &PolicyEnvelope,
) -> Result<ExecutorKind, RefusalReason> {
    // `clippy::match_same_arms` is allowed because `Praxis` and `Any`
    // are conceptually distinct hints — the tie that maps `Any` to
    // `Praxis` is a v0.2 *policy* default, not a coincidence to be
    // collapsed.
    #[allow(clippy::match_same_arms)]
    let candidate = match hint {
        ExecutorHint::Praxis => ExecutorKind::Praxis,
        ExecutorHint::Arcan => ExecutorKind::Arcan,
        ExecutorHint::Spaces => ExecutorKind::Spaces,
        ExecutorHint::Any => ExecutorKind::Praxis, // v0.2 default
    };
    if policy.permitted_executors.is_empty() {
        return Ok(candidate);
    }
    if policy.permitted_executors.contains(&candidate) {
        return Ok(candidate);
    }
    Err(RefusalReason::ExecutorNotPermitted {
        wanted: hint,
        permitted: policy.permitted_executors.clone(),
    })
}

fn build_arcan_instruction(directive: &Directive<Committed>) -> String {
    if let Some(utt) = &directive.utterance {
        utt.clone()
    } else {
        directive
            .act
            .act
            .description
            .clone()
            .unwrap_or_else(|| directive.act.act.id.as_str().to_owned())
    }
}

fn build_spaces_message(directive: &Directive<Committed>) -> SpacesMessage {
    let act = &directive.act.act;

    let body = directive
        .act
        .bindings
        .iter()
        .find(|b| b.name == "body")
        .and_then(|b| match &b.value {
            ResolvedSlotValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| directive.utterance.clone().unwrap_or_default());

    let channel = directive
        .act
        .bindings
        .iter()
        .find(|b| b.name == "channel")
        .and_then(|b| match &b.value {
            ResolvedSlotValue::String(s) => Some(s.clone()),
            ResolvedSlotValue::Referent(rv) => Some(format!("{rv:?}")),
            _ => None,
        });

    SpacesMessage { act_id: act.id.clone(), channel, body }
}

// --- Drift detection (caller-side) -----------------------------------------

/// Caller-side drift check.
///
/// **The router itself does not call this** because drift detection
/// requires `Arc::ptr_eq` (or content-addressed hashing) on the
/// underlying [`sensorium_core::WorkspaceState`], which a pure
/// function can't do given only IDs (snapshot IDs are minted fresh
/// per capture).
///
/// The caller is expected to:
/// 1. Stash the [`sensorium_core::WorkspaceSnapshot`] taken at
///    finalize-time.
/// 2. Take a fresh snapshot at dispatch-time.
/// 3. Call this function with both. If `true`, refuse to dispatch and
///    re-resolve / refuse / re-prompt the user.
///
/// Internally uses [`sensorium_core::WorkspaceSnapshot::observes_same_state`],
/// which is `Arc::ptr_eq` — `O(1)`, no allocation, no hashing.
#[must_use]
pub fn drift_detected(
    committed: &sensorium_core::WorkspaceSnapshot,
    current: &sensorium_core::WorkspaceSnapshot,
) -> bool {
    !committed.observes_same_state(current)
}

// --- Re-exports for convenience ----------------------------------------------

/// Trust scoring helper exposed for downstream callers that want to
/// pre-filter directives before submitting to the router.
#[must_use]
pub fn confidence_floor(confidence: &Confidence) -> f32 {
    confidence.weakest_slot()
}

/// Borrow the binding kinds appearing on this directive — for
/// trust-aware downstream policies.
pub fn binding_kinds(directive: &Directive<Committed>) -> Vec<BindingKind> {
    directive
        .act
        .bindings
        .iter()
        .map(|b| b.provenance.binding)
        .collect()
}

// Re-export ReferentValue path-friendly for tests / consumers.
pub use pneuma_core::{ReferentValue as Referent, ResolvedSlot};
