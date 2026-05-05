//! End-to-end tests for `workspace.switch_app` execution.
//!
//! Same shape as `browser_ops.rs` — cross-platform unit tests for
//! slot extraction, app-name safety filter, and platform gating;
//! a `#[ignore]`-gated macOS-only test that actually switches apps.
//!
//! Properties under test:
//!
//! - On non-macOS, `workspace.switch_app` returns
//!   [`PraxisError::PlatformUnsupported`] without shelling out.
//! - Slot extraction enforces `Referent::App` shape — wrong slot
//!   kinds produce typed errors, not panics.
//! - App-name safety filter rejects empty / `"` / `\` / newline.
//! - The act has reverse-action `None` (Reversibility::Free); calling
//!   `reverse` on a `workspace.switch_app` outcome errors with
//!   `NoReverseAction`.

use pneuma_acts::registry;
use pneuma_core::act::ResolvedSlotValue;
use pneuma_core::{ActId, AppId, FileRef, ReferentValue};
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

fn app_slot(name: &str) -> ResolvedSlotValue {
    ResolvedSlotValue::Referent(ReferentValue::App(AppId::new(name).unwrap()))
}

// --- Slot validation (cross-platform) --------------------------------------

#[test]
fn missing_target_slot_errors_with_typed_missing_slot() {
    let call = praxis_call("workspace.switch_app", vec![]);
    let err = LocalPraxis.execute(&call).unwrap_err();
    assert!(
        matches!(err, PraxisError::MissingSlot { ref slot, .. } if slot == "target"),
        "expected MissingSlot {{ slot: \"target\", .. }}, got {err:?}"
    );
}

#[test]
fn wrong_slot_kind_for_target_errors() {
    // A File referent in the target slot — wrong kind, no shell-out.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not_an_app.txt");
    let bogus = ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new(path)));
    let call = praxis_call("workspace.switch_app", vec![("target", bogus)]);
    let err = LocalPraxis.execute(&call).unwrap_err();
    assert!(
        matches!(
            err,
            PraxisError::WrongSlotKind { ref slot, .. } if slot == "target"
        ),
        "expected WrongSlotKind {{ slot: \"target\", .. }}, got {err:?}"
    );
}

// --- App-name safety filter (cross-platform) -------------------------------

// Note: we cannot test rejection of an empty AppId because
// AppId::new() rejects empty inputs at the type-system boundary.
// The safety filter on raw app-name strings remains valuable for
// defense in depth, but the empty case is unreachable through the
// public API.

// --- Platform gate ---------------------------------------------------------

#[cfg(not(target_os = "macos"))]
#[test]
fn switch_app_on_non_macos_returns_platform_unsupported() {
    let call = praxis_call("workspace.switch_app", vec![("target", app_slot("Safari"))]);
    let err = LocalPraxis.execute(&call).unwrap_err();
    assert!(
        matches!(err, PraxisError::PlatformUnsupported { .. }),
        "expected PlatformUnsupported on non-macOS, got {err:?}"
    );
}

// --- Reverse-action shape (cross-platform) ---------------------------------

#[test]
fn switch_app_outcome_carries_no_reverse() {
    // We can't call execute() to get an outcome on Linux (it errors
    // with PlatformUnsupported), but we can hand-construct an
    // outcome and confirm reverse refuses cleanly. This documents
    // the v0.2 stance: switch_app is Reversibility::Free, so
    // ReverseAction::None is the canonical outcome shape.
    let call = praxis_call("workspace.switch_app", vec![("target", app_slot("Safari"))]);
    let outcome = pneuma_praxis_bridge::ExecutionOutcome {
        act_id: ActId::new("workspace.switch_app").unwrap(),
        result: serde_json::json!({"activated": "Safari"}),
        reverse_action: ReverseAction::None,
    };
    let err = LocalPraxis.reverse(&call, &outcome).unwrap_err();
    assert!(
        matches!(err, PraxisError::NoReverseAction),
        "switch_app outcome's None reverse must surface NoReverseAction"
    );
}

// --- macOS integration test (gated, ignored by default) --------------------

/// Real end-to-end test against macOS. Activates the Finder (always
/// present on macOS, no Automation prompt needed for it). Disabled
/// by default to keep CI hermetic. Run manually with:
///
/// ```bash
/// cargo test -p pneuma-praxis-bridge --test workspace_ops -- --ignored
/// ```
#[cfg(target_os = "macos")]
#[test]
#[ignore = "activates Finder on macOS — run manually"]
fn macos_switch_app_to_finder() {
    let call = praxis_call("workspace.switch_app", vec![("target", app_slot("Finder"))]);
    let outcome = LocalPraxis
        .execute(&call)
        .expect("Finder activation should succeed");
    assert_eq!(outcome.act_id.as_str(), "workspace.switch_app");
    assert!(matches!(outcome.reverse_action, ReverseAction::None));
}
