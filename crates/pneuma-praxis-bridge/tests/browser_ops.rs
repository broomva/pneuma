//! End-to-end tests for `browser.navigate` execution.
//!
//! Properties under test:
//!
//! - On non-macOS, `browser.navigate` returns
//!   [`PraxisError::PlatformUnsupported`] and does not shell out.
//! - URL slot extraction enforces `Referent::Url` shape — wrong slot
//!   kinds produce typed errors, not panics.
//! - URL safety filter rejects double-quote, backslash, and newlines
//!   before any shell-out is attempted (testable on any platform).
//! - Reverse-action carries the captured prior URL on macOS; not
//!   applicable on other platforms (since execute fails first).
//! - Reversing an `ExecutionOutcome::None` for `browser.navigate` is
//!   prevented by typing — but missing slots still surface cleanly.
//!
//! ## Why most tests are cross-platform
//!
//! The core invariants — slot extraction, URL validation, error
//! surfacing — are platform-independent. The actual AppleScript
//! shell-out is exercised by an `#[ignore]`-gated macOS-only test
//! that opens a real Safari tab; CI does not run that test.

use pneuma_acts::registry;
use pneuma_core::act::ResolvedSlotValue;
use pneuma_core::{ActId, FileRef, ReferentValue};
use pneuma_praxis_bridge::{Executor, LocalPraxis, PraxisError, ReverseAction};
use pneuma_router::PraxisCall;

// --- Helpers ---------------------------------------------------------------

fn praxis_call(act_id: &str, slots: Vec<(&str, ResolvedSlotValue)>) -> PraxisCall {
    let act = registry()
        .into_iter()
        .find(|a| a.id.as_str() == act_id)
        .unwrap_or_else(|| panic!("registry must contain {act_id}"));
    PraxisCall {
        act_id: ActId::new(act_id).unwrap(),
        slots: slots.into_iter().map(|(n, v)| (n.to_owned(), v)).collect(),
        reverse_recipe: act.reverse_recipe.clone(),
    }
}

fn url_slot(url: &str) -> ResolvedSlotValue {
    ResolvedSlotValue::Referent(ReferentValue::Url(url.to_owned()))
}

// --- Slot validation (cross-platform) --------------------------------------

#[test]
fn missing_url_slot_errors_with_typed_missing_slot() {
    let call = praxis_call("browser.navigate", vec![]);
    let err = LocalPraxis.execute(&call).unwrap_err();
    assert!(
        matches!(err, PraxisError::MissingSlot { ref slot, .. } if slot == "url"),
        "expected MissingSlot {{ slot: \"url\", .. }}, got {err:?}"
    );
}

#[test]
fn wrong_slot_kind_for_url_errors() {
    // A File referent in the url slot — wrong kind, no shell-out.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not_a_url.txt");
    let bogus = ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new(path)));
    let call = praxis_call("browser.navigate", vec![("url", bogus)]);
    let err = LocalPraxis.execute(&call).unwrap_err();
    assert!(
        matches!(
            err,
            PraxisError::WrongSlotKind { ref slot, .. } if slot == "url"
        ),
        "expected WrongSlotKind {{ slot: \"url\", .. }}, got {err:?}"
    );
}

// --- URL safety filter (cross-platform) ------------------------------------

#[test]
fn url_with_double_quote_is_rejected() {
    let call = praxis_call(
        "browser.navigate",
        vec![("url", url_slot("https://example.com/\"injected"))],
    );
    let err = LocalPraxis.execute(&call).unwrap_err();
    assert!(
        matches!(err, PraxisError::UnsafeUrl { reason } if reason.contains("double-quote")),
        "expected UnsafeUrl(double-quote), got {err:?}"
    );
}

#[test]
fn url_with_backslash_is_rejected() {
    let call = praxis_call(
        "browser.navigate",
        vec![("url", url_slot("https://example.com/\\path"))],
    );
    let err = LocalPraxis.execute(&call).unwrap_err();
    assert!(
        matches!(err, PraxisError::UnsafeUrl { reason } if reason.contains("backslash")),
        "expected UnsafeUrl(backslash), got {err:?}"
    );
}

#[test]
fn url_with_newline_is_rejected() {
    let call = praxis_call(
        "browser.navigate",
        vec![("url", url_slot("https://example.com/\nattack"))],
    );
    let err = LocalPraxis.execute(&call).unwrap_err();
    assert!(
        matches!(err, PraxisError::UnsafeUrl { reason } if reason.contains("newline")),
        "expected UnsafeUrl(newline), got {err:?}"
    );
}

// --- Platform gate ---------------------------------------------------------

#[cfg(not(target_os = "macos"))]
#[test]
fn navigate_on_non_macos_returns_platform_unsupported() {
    let call = praxis_call(
        "browser.navigate",
        vec![("url", url_slot("https://example.com"))],
    );
    let err = LocalPraxis.execute(&call).unwrap_err();
    assert!(
        matches!(err, PraxisError::PlatformUnsupported { .. }),
        "expected PlatformUnsupported on non-macOS, got {err:?}"
    );
}

// --- ReverseAction shape (cross-platform) ----------------------------------

#[test]
fn restore_url_variant_is_constructible_and_serializable() {
    // Sanity-check the new variant compiles and round-trips JSON.
    let action = ReverseAction::RestoreUrl {
        browser: "Safari".to_owned(),
        prior_url: "https://prior.example.com".to_owned(),
    };
    let json = serde_json::to_string(&action).unwrap();
    assert!(json.contains("RestoreUrl"));
    assert!(json.contains("Safari"));
    assert!(json.contains("prior.example.com"));
    let round_trip: ReverseAction = serde_json::from_str(&json).unwrap();
    assert_eq!(round_trip, action);
    // Also: the new variant is not `None`.
    assert!(!action.is_none());
}

// --- Reverse-path URL safety (cross-platform via reverse) ------------------

#[test]
fn reverse_with_unsafe_prior_url_is_rejected() {
    // Hand-construct an outcome carrying a malicious prior_url. This
    // models the case where capture_browser_front_url could in theory
    // return something unexpected — we still defend against it on
    // the reverse path.
    let call = praxis_call(
        "browser.navigate",
        vec![("url", url_slot("https://example.com"))],
    );
    let outcome = pneuma_praxis_bridge::ExecutionOutcome {
        act_id: ActId::new("browser.navigate").unwrap(),
        result: serde_json::json!({}),
        reverse_action: ReverseAction::RestoreUrl {
            browser: "Safari".to_owned(),
            prior_url: "https://example.com/\"injection".to_owned(),
        },
    };
    let err = LocalPraxis.reverse(&call, &outcome).unwrap_err();
    assert!(
        matches!(err, PraxisError::UnsafeUrl { .. }),
        "expected UnsafeUrl on reverse path, got {err:?}"
    );
}

// --- macOS integration test (gated, ignored by default) --------------------

/// Real end-to-end test against macOS Safari. Disabled by default
/// (would open a Safari window during `cargo test` and require the
/// user to grant Automation permissions). Run manually with:
///
/// ```bash
/// cargo test -p pneuma-praxis-bridge --test browser_ops -- --ignored
/// ```
#[cfg(target_os = "macos")]
#[test]
#[ignore = "opens a real Safari window — run manually"]
fn macos_navigate_safari_round_trip() {
    let call = praxis_call(
        "browser.navigate",
        vec![("url", url_slot("https://example.com/"))],
    );
    let outcome = LocalPraxis
        .execute(&call)
        .expect("Safari navigation should succeed when run interactively");
    assert_eq!(outcome.act_id.as_str(), "browser.navigate");
    let ReverseAction::RestoreUrl { browser, prior_url } = &outcome.reverse_action else {
        panic!("expected RestoreUrl, got {:?}", outcome.reverse_action);
    };
    assert_eq!(browser, "Safari");
    // prior_url is whatever the user had open — non-empty when Safari
    // had a tab; possibly empty if Safari was just launched.
    let _ = prior_url;
    // Reverse should restore the prior URL (or a sensible default).
    LocalPraxis.reverse(&call, &outcome).unwrap();
}
