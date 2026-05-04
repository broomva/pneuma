//! End-to-end pipeline test for `browser.navigate` (step #13 of MIL §11.2).
//!
//! Drives the directive contract chain at the API level — no demo
//! binary, no stdin, no real Safari. Validates that the typed contract
//! holds end-to-end:
//!
//!   parse_utterance → ActRegistry → Directive<Composing>
//!     → bind URL slot (Referent::Url) → try_finalize → Directive<Ready>
//!     → commit → Directive<Committed> → pneuma_router::dispatch
//!     → Dispatch::Praxis(PraxisCall)
//!     → LocalPraxis::execute (cross-platform: returns PlatformUnsupported
//!       on non-macOS; would shell out on macOS, gated by --ignored)
//!
//! What this test does NOT cover:
//!
//! - Real Safari navigation (gated by `#[ignore]` in
//!   `pneuma-praxis-bridge/tests/browser_ops.rs`).
//! - The demo binary's main loop (still rename-only in v0.2; demo
//!   refactor for browser flow is a follow-up).

use chrono::Utc;

use pneuma_acts::{ActRegistry, registry};
use pneuma_core::act::ResolvedSlotValue;
use pneuma_core::{
    BindingKind, Confidence, ConfidenceProducer, ConfidenceScore, ContextRef, ContextSnapshotId,
    Directive, PolicyEnvelope, Provenance, ReferentValue, ResolvedAct, ResolvedSlot, SpeechAct,
};
use pneuma_demo::{ParsedUtterance, parse_utterance};
#[cfg(not(target_os = "macos"))]
use pneuma_praxis_bridge::{Executor, LocalPraxis, PraxisError};
use pneuma_router::{Dispatch, dispatch};
use sensorium_context::Observer;
use sensorium_core::{Timestamp, WorkspaceContext};

/// The URL we navigate to throughout the e2e tests. Stable so the
/// assertions stay readable.
const TEST_URL: &str = "https://example.com/mil-test";

// --- Helpers ---------------------------------------------------------------

fn fresh_context() -> WorkspaceContext {
    let observer = sensorium_context::ManualObserver::new(Timestamp::now());
    observer.current()
}

fn parsed_navigate_utterance() -> ParsedUtterance {
    let r = ActRegistry::canonical();
    parse_utterance(&format!("navigate to {TEST_URL}"), &r).expect("'navigate to <URL>' must parse")
}

fn build_navigate_directive(parsed: &ParsedUtterance) -> Directive<pneuma_core::Composing> {
    let act = registry()
        .into_iter()
        .find(|a| a.id == parsed.act_id)
        .expect("registry must contain browser.navigate");
    let provenance = Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now());
    // The parser produced a single payload binding: ("url", "<URL>").
    let url = parsed
        .payload_slots
        .iter()
        .find(|(name, _)| name == "url")
        .map(|(_, v)| v.clone())
        .expect("parser must extract a `url` slot");
    Directive::new(SpeechAct::Directive, ResolvedAct::empty(act))
        .bind_slot(
            ResolvedSlot::new(
                "url",
                ResolvedSlotValue::Referent(ReferentValue::Url(url)),
                provenance,
            )
            .expect("slot is non-empty"),
        )
        .with_utterance(parsed.utterance.clone())
}

fn build_confidence_for_navigate() -> Confidence {
    Confidence::from_slots(vec![(
        "url".to_owned(),
        ConfidenceScore::new(0.95, true, ConfidenceProducer::Deterministic).unwrap(),
    )])
    .expect("confidence is constructible")
}

fn finalize_and_commit(
    composing: Directive<pneuma_core::Composing>,
    context: &WorkspaceContext,
) -> Directive<pneuma_core::Committed> {
    let snapshot = context.snapshot();
    let context_ref = ContextRef::new(
        ContextSnapshotId::from_uuid(snapshot.id.into_inner()),
        snapshot.taken_at.into_inner(),
    );
    let policy = PolicyEnvelope::intrinsic(
        pneuma_core::Reversibility::Costly,
        pneuma_core::BlastRadius::Local,
    );
    let ready = composing
        .try_finalize(context_ref, policy, build_confidence_for_navigate())
        .map_err(|(_, err)| err)
        .expect("ready directive should finalize cleanly");
    ready
        .commit()
        .map_err(|(_, err)| err)
        .expect("commit should succeed (synthetic context)")
}

// --- Tests -----------------------------------------------------------------

#[test]
fn parse_navigate_to_url_and_dispatch_through_router() {
    let parsed = parsed_navigate_utterance();
    assert_eq!(parsed.act_id.as_str(), "browser.navigate");

    let composing = build_navigate_directive(&parsed);
    let context = fresh_context();
    let committed = finalize_and_commit(composing, &context);

    // Route. Browser acts have `ExecutorHint::Praxis`, so we expect
    // `Dispatch::Praxis(call)` carrying the original act_id and the
    // URL slot binding.
    let result = dispatch(&committed, &context);
    let call = match result {
        Dispatch::Praxis(c) => c,
        other => panic!("expected Dispatch::Praxis, got {other:?}"),
    };

    assert_eq!(call.act_id.as_str(), "browser.navigate");
    let url_value = call
        .slots
        .iter()
        .find(|(name, _)| name == "url")
        .map(|(_, v)| v)
        .expect("PraxisCall must carry the `url` slot");
    assert!(
        matches!(
            url_value,
            ResolvedSlotValue::Referent(ReferentValue::Url(u)) if u == TEST_URL
        ),
        "url slot must be a Referent::Url carrying the original URL, got {url_value:?}"
    );
    assert_eq!(
        call.reverse_recipe.as_deref(),
        Some("navigate_to_prior_url"),
        "the router must propagate the act's reverse_recipe hint"
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn full_chain_from_utterance_to_executor_returns_platform_unsupported() {
    // On Linux CI, the executor refuses without shelling out. This is
    // the production failure mode we want — non-macOS users get a
    // typed PlatformUnsupported error, not a panic and not a silent
    // success.
    let parsed = parsed_navigate_utterance();
    let composing = build_navigate_directive(&parsed);
    let context = fresh_context();
    let committed = finalize_and_commit(composing, &context);

    let call = match dispatch(&committed, &context) {
        Dispatch::Praxis(c) => c,
        other => panic!("expected Praxis dispatch, got {other:?}"),
    };

    let err = LocalPraxis.execute(&call).unwrap_err();
    assert!(
        matches!(err, PraxisError::PlatformUnsupported { .. }),
        "non-macOS execute must surface PlatformUnsupported, got {err:?}"
    );
}

#[test]
fn alternative_phrasings_all_dispatch_to_browser_navigate() {
    // Every supported phrasing should route to `browser.navigate`. This
    // is a property test asserting the parser → registry → directive
    // chain is stable across surface variants.
    let r = ActRegistry::canonical();
    for phrasing in [
        "navigate to https://example.com",
        "go to https://example.com",
        "browse https://example.com",
        "go https://example.com",
    ] {
        let parsed = parse_utterance(phrasing, &r)
            .unwrap_or_else(|e| panic!("'{phrasing}' must parse: {e}"));
        assert_eq!(
            parsed.act_id.as_str(),
            "browser.navigate",
            "'{phrasing}' should route to browser.navigate"
        );
        // Build the directive but stop short of dispatching — we only
        // care that the chain is constructible for every phrasing.
        let composing = build_navigate_directive(&parsed);
        let context = fresh_context();
        let _committed = finalize_and_commit(composing, &context);
    }
}
