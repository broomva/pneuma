//! Integration tests for [`pneuma_resolver`].
//!
//! Properties under test:
//!
//! 1. **Per-axis resolution** — File / Window / App slots resolve
//!    from the matching `WorkspaceContext` axis.
//! 2. **Hint override** — `AnaphorRef::with_hint("window")` forces
//!    the window axis even on an `Any` slot.
//! 3. **`Any`-slot preference** — file > window > app fallback.
//! 4. **Missing focus** — context with no focused entity surfaces
//!    `NoFocusedEntity` rather than panicking.
//! 5. **Unknown surface** — `AnaphorRef::new("teapot")` surfaces
//!    `UnknownAnaphor`.
//! 6. **Incompatible slot type** — anaphor against a `String` slot
//!    isn't reachable (parser produces Anaphor only for Referent
//!    slots), but a `Selection`/`Symbol`/`Url` slot surfaces
//!    `IncompatibleSlotType`.
//! 7. **Pass-through** — non-anaphor bindings unchanged after
//!    `resolve_directive`.
//! 8. **Directive-level** — the public `resolve_directive` correctly
//!    walks all bindings, even multiple anaphors on the same act.

use chrono::Utc;
use pneuma_core::act::{ResolvedAct, ResolvedSlot, ResolvedSlotValue};
use pneuma_core::{
    Act, ActId, ActPrimitive, AnaphorRef, AppId, Arity, BindingKind, BlastRadius, Directive,
    ExecutorHint, FileRef, Provenance, ReferentType, ReferentValue, Reversibility, SlotKind,
    SlotSignature, SpeechAct,
};
use pneuma_resolver::{ResolverError, resolve_anaphor, resolve_directive};
use sensorium_context::{ManualObserver, Observer};
use sensorium_core::{AppId as SAppId, FileRef as SFileRef, Timestamp, WindowId as SWindowId};
use std::path::PathBuf;

// --- Context builders -----------------------------------------------------

fn ctx_with_focused_file(path: &str) -> sensorium_core::WorkspaceContext {
    let obs = ManualObserver::new(Timestamp::now());
    obs.set_focused_file(SFileRef::new(PathBuf::from(path)), false);
    obs.current()
}

fn ctx_with_focused_app(app: &str) -> sensorium_core::WorkspaceContext {
    let obs = ManualObserver::new(Timestamp::now());
    obs.rebuild_with(|b| b.with_focused_app(Some(SAppId::new(app).unwrap())));
    obs.current()
}

fn ctx_with_focused_window(win: &str) -> sensorium_core::WorkspaceContext {
    let obs = ManualObserver::new(Timestamp::now());
    obs.rebuild_with(|b| b.with_focused_window(Some(SWindowId::new(win).unwrap())));
    obs.current()
}

fn ctx_with_all(file: &str, app: &str, win: &str) -> sensorium_core::WorkspaceContext {
    let obs = ManualObserver::new(Timestamp::now());
    obs.set_focused_file(SFileRef::new(PathBuf::from(file)), false);
    obs.rebuild_with(|b| {
        b.with_focused_app(Some(SAppId::new(app).unwrap()))
            .with_focused_window(Some(SWindowId::new(win).unwrap()))
    });
    obs.current()
}

fn empty_ctx() -> sensorium_core::WorkspaceContext {
    ManualObserver::new(Timestamp::now()).current()
}

// --- Directive builders ---------------------------------------------------

/// Build a synthetic act with a single required Referent slot of the
/// given type. Useful for testing per-slot-type behavior in isolation.
fn act_with_single_referent_slot(slot_name: &str, slot_type: ReferentType) -> Act {
    Act {
        id: ActId::new("test.resolve").unwrap(),
        primitive: ActPrimitive::Custom,
        slots: vec![
            SlotSignature::new(slot_name, SlotKind::Referent(slot_type), Arity::Required).unwrap(),
        ],
        reversibility: Reversibility::Free,
        blast_radius: BlastRadius::Local,
        executor_hint: ExecutorHint::Praxis,
        reverse_recipe: None,
        description: Some("synthetic resolver test act".to_owned()),
    }
}

fn directive_with_anaphor_slot(
    slot_name: &str,
    slot_type: ReferentType,
    anaphor: AnaphorRef,
) -> Directive<pneuma_core::Composing> {
    let act = act_with_single_referent_slot(slot_name, slot_type);
    let resolved = ResolvedAct::empty(act);
    let provenance = Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now());
    Directive::new(SpeechAct::Directive, resolved).bind_slot(
        ResolvedSlot::new(
            slot_name,
            ResolvedSlotValue::Referent(ReferentValue::Anaphor(anaphor)),
            provenance,
        )
        .unwrap(),
    )
}

// --- Property 1: Per-axis resolution --------------------------------------

#[test]
fn file_slot_resolves_focused_file() {
    let ctx = ctx_with_focused_file("/tmp/auth.rs");
    let anaphor = AnaphorRef::new("this").unwrap();
    let resolved = resolve_anaphor(&anaphor, ReferentType::File, &ctx).unwrap();
    match resolved {
        ReferentValue::File(f) => assert_eq!(f.path, PathBuf::from("/tmp/auth.rs")),
        other => panic!("expected File, got {other:?}"),
    }
}

#[test]
fn app_slot_resolves_focused_app() {
    let ctx = ctx_with_focused_app("com.apple.Safari");
    let anaphor = AnaphorRef::new("this app").unwrap();
    let resolved = resolve_anaphor(&anaphor, ReferentType::App, &ctx).unwrap();
    match resolved {
        ReferentValue::App(a) => assert_eq!(a.as_str(), "com.apple.Safari"),
        other => panic!("expected App, got {other:?}"),
    }
}

#[test]
fn window_slot_resolves_focused_window() {
    let ctx = ctx_with_focused_window("ax:42:Editor");
    let anaphor = AnaphorRef::new("this window").unwrap();
    let resolved = resolve_anaphor(&anaphor, ReferentType::Window, &ctx).unwrap();
    match resolved {
        ReferentValue::Window(w) => assert_eq!(w.as_str(), "ax:42:Editor"),
        other => panic!("expected Window, got {other:?}"),
    }
}

// --- Property 2: Hint override --------------------------------------------

#[test]
fn hint_override_routes_any_slot_to_window_axis() {
    let ctx = ctx_with_all("/tmp/x.rs", "com.apple.Safari", "ax:9:Tab");
    let anaphor = AnaphorRef::new("this").unwrap().with_hint("window");
    let resolved = resolve_anaphor(&anaphor, ReferentType::Any, &ctx).unwrap();
    assert!(
        matches!(resolved, ReferentValue::Window(_)),
        "hint=window must route to Window axis even on Any slot"
    );
}

#[test]
fn hint_override_routes_file_slot_to_app_axis() {
    // Edge case: hint can override even a typed slot. v0.2 honors it.
    let ctx = ctx_with_all("/tmp/x.rs", "com.apple.Safari", "ax:9:Tab");
    let anaphor = AnaphorRef::new("this").unwrap().with_hint("app");
    let resolved = resolve_anaphor(&anaphor, ReferentType::File, &ctx).unwrap();
    assert!(matches!(resolved, ReferentValue::App(_)));
}

// --- Property 3: Any-slot preference order --------------------------------

#[test]
fn any_slot_prefers_file_when_all_axes_populated() {
    let ctx = ctx_with_all("/tmp/x.rs", "com.apple.Safari", "ax:9:Tab");
    let anaphor = AnaphorRef::new("this").unwrap();
    let resolved = resolve_anaphor(&anaphor, ReferentType::Any, &ctx).unwrap();
    assert!(matches!(resolved, ReferentValue::File(_)));
}

#[test]
fn any_slot_falls_through_to_window_when_no_file() {
    let ctx = ctx_with_focused_window("ax:9:Tab");
    let anaphor = AnaphorRef::new("this").unwrap();
    let resolved = resolve_anaphor(&anaphor, ReferentType::Any, &ctx).unwrap();
    assert!(matches!(resolved, ReferentValue::Window(_)));
}

#[test]
fn any_slot_falls_through_to_app_when_no_file_or_window() {
    let ctx = ctx_with_focused_app("com.apple.Safari");
    let anaphor = AnaphorRef::new("this").unwrap();
    let resolved = resolve_anaphor(&anaphor, ReferentType::Any, &ctx).unwrap();
    assert!(matches!(resolved, ReferentValue::App(_)));
}

// --- Property 4: Missing focus --------------------------------------------

#[test]
fn no_focused_file_for_file_slot_errors() {
    let ctx = empty_ctx();
    let anaphor = AnaphorRef::new("this").unwrap();
    let err = resolve_anaphor(&anaphor, ReferentType::File, &ctx).unwrap_err();
    assert!(
        matches!(err, ResolverError::NoFocusedEntity { axis: "file", .. }),
        "expected NoFocusedEntity(file), got {err:?}"
    );
}

#[test]
fn empty_context_for_any_slot_errors() {
    let ctx = empty_ctx();
    let anaphor = AnaphorRef::new("this").unwrap();
    let err = resolve_anaphor(&anaphor, ReferentType::Any, &ctx).unwrap_err();
    assert!(matches!(err, ResolverError::NoFocusedEntity { .. }));
}

// --- Property 5: Unknown surface ------------------------------------------

#[test]
fn unrecognized_surface_form_errors() {
    let ctx = ctx_with_focused_file("/tmp/x.rs");
    let anaphor = AnaphorRef::new("teapot").unwrap();
    let err = resolve_anaphor(&anaphor, ReferentType::File, &ctx).unwrap_err();
    assert!(
        matches!(err, ResolverError::UnknownAnaphor { ref surface } if surface == "teapot"),
        "expected UnknownAnaphor(teapot), got {err:?}"
    );
}

#[test]
fn deictic_surface_recognizer_accepts_compound_forms() {
    use pneuma_resolver::is_deictic_surface;
    assert!(is_deictic_surface("this"));
    assert!(is_deictic_surface("that"));
    assert!(is_deictic_surface("the focused window"));
    assert!(is_deictic_surface("this file"));
    assert!(is_deictic_surface("current"));
    assert!(is_deictic_surface("the active app"));
    // Not-deictic
    assert!(!is_deictic_surface("teapot"));
    assert!(!is_deictic_surface(""));
    assert!(!is_deictic_surface("this-not-deictic")); // no space → prefix check rejects
}

// --- Property 6: Incompatible slot type -----------------------------------

#[test]
fn anaphor_against_url_slot_errors_with_incompatible_type() {
    let ctx = ctx_with_focused_file("/tmp/x.rs");
    let anaphor = AnaphorRef::new("this").unwrap();
    let err = resolve_anaphor(&anaphor, ReferentType::Url, &ctx).unwrap_err();
    assert!(
        matches!(
            err,
            ResolverError::IncompatibleSlotType {
                slot_type: ReferentType::Url,
                ..
            }
        ),
        "expected IncompatibleSlotType(Url), got {err:?}"
    );
}

#[test]
fn anaphor_against_symbol_slot_errors_with_incompatible_type() {
    let ctx = ctx_with_focused_file("/tmp/x.rs");
    let anaphor = AnaphorRef::new("this").unwrap();
    let err = resolve_anaphor(&anaphor, ReferentType::Symbol, &ctx).unwrap_err();
    assert!(matches!(
        err,
        ResolverError::IncompatibleSlotType {
            slot_type: ReferentType::Symbol,
            ..
        }
    ));
}

// --- Property 7: Pass-through of non-anaphor bindings ---------------------

#[test]
fn non_anaphor_binding_passes_through_unchanged() {
    let ctx = ctx_with_focused_file("/tmp/x.rs");
    let act = act_with_single_referent_slot("target", ReferentType::File);
    let resolved_act = ResolvedAct::empty(act);
    let provenance = Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now());
    let original_path = PathBuf::from("/explicitly/bound/path.rs");
    let directive = Directive::new(SpeechAct::Directive, resolved_act).bind_slot(
        ResolvedSlot::new(
            "target",
            ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new(&original_path))),
            provenance,
        )
        .unwrap(),
    );
    let resolved = resolve_directive(directive, &ctx).unwrap();
    let target = resolved.act.binding("target").unwrap();
    match &target.value {
        ResolvedSlotValue::Referent(ReferentValue::File(f)) => {
            assert_eq!(
                f.path, original_path,
                "explicit binding must NOT be overwritten"
            );
        }
        other => panic!("expected File, got {other:?}"),
    }
}

// --- Property 8: Directive-level full walk --------------------------------

#[test]
fn resolve_directive_replaces_anaphor_binding_in_place() {
    let ctx = ctx_with_focused_file("/tmp/x.rs");
    let anaphor = AnaphorRef::new("this").unwrap();
    let directive = directive_with_anaphor_slot("target", ReferentType::File, anaphor);
    let resolved = resolve_directive(directive, &ctx).unwrap();
    let target = resolved.act.binding("target").unwrap();
    match &target.value {
        ResolvedSlotValue::Referent(ReferentValue::File(f)) => {
            assert_eq!(f.path, PathBuf::from("/tmp/x.rs"));
        }
        other => panic!("anaphor must be replaced by File; got {other:?}"),
    }
}

#[test]
fn resolve_directive_surfaces_first_resolution_error() {
    let ctx = empty_ctx();
    let anaphor = AnaphorRef::new("this").unwrap();
    let directive = directive_with_anaphor_slot("target", ReferentType::File, anaphor);
    let err = resolve_directive(directive, &ctx).unwrap_err();
    assert!(matches!(err, ResolverError::NoFocusedEntity { .. }));
}

#[test]
fn resolve_directive_with_no_anaphors_passes_through_clean() {
    let ctx = empty_ctx(); // even an empty context is fine — no anaphor to resolve
    let act = act_with_single_referent_slot("target", ReferentType::App);
    let resolved_act = ResolvedAct::empty(act);
    let provenance = Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now());
    let directive = Directive::new(SpeechAct::Directive, resolved_act).bind_slot(
        ResolvedSlot::new(
            "target",
            ResolvedSlotValue::Referent(ReferentValue::App(AppId::new("com.test").unwrap())),
            provenance,
        )
        .unwrap(),
    );
    let resolved = resolve_directive(directive, &ctx).unwrap();
    let target = resolved.act.binding("target").unwrap();
    assert!(matches!(
        &target.value,
        ResolvedSlotValue::Referent(ReferentValue::App(_))
    ));
}
