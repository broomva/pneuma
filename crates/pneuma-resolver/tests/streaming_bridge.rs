//! Tests for the cross-substrate Generation bridge + resolver
//! preservation of `Directive.generation` through anaphor resolution.
//!
//! The bridge exists because Rust's orphan rule forbids
//! `impl From<sensorium_core::Generation> for pneuma_core::Generation`
//! in either core crate. `pneuma-resolver` depends on both, so the
//! conversion lives here as the `bridge_generation` free function.
//!
//! Properties under test:
//!
//! 1. **`bridge_generation` preserves the raw u64** — sensorium and
//!    pneuma generations are byte-compatible across the bridge.
//! 2. **`bridge_generation` round-trips INITIAL** — the canonical
//!    "first generation" value crosses the bridge unchanged.
//! 3. **`resolve_directive` preserves `generation`** — the resolver
//!    mutates slot bindings in place but does not touch the
//!    directive's generation field.
//! 4. **`resolve_directive` preserves `None` generation** — the
//!    non-streaming construction path stays untouched.

use chrono::Utc;
use pneuma_core::act::{ResolvedAct, ResolvedSlot, ResolvedSlotValue};
use pneuma_core::{
    Act, ActId, ActPrimitive, AnaphorRef, Arity, BindingKind, BlastRadius, Composing, Directive,
    ExecutorHint, Provenance, ReferentType, ReferentValue, Reversibility, SlotKind, SlotSignature,
    SpeechAct,
};
use pneuma_resolver::{bridge_generation, resolve_directive};
use sensorium_context::{ManualObserver, Observer};
use sensorium_core::{FileRef as SFileRef, Timestamp};
use std::path::PathBuf;

// --- Fixtures ---------------------------------------------------------------

fn ctx_with_focused_file(path: &str) -> sensorium_core::WorkspaceContext {
    let obs = ManualObserver::new(Timestamp::now());
    obs.set_focused_file(SFileRef::new(PathBuf::from(path)), false);
    obs.current()
}

fn directive_with_file_anaphor(g: Option<pneuma_core::Generation>) -> Directive<Composing> {
    let act = Act {
        id: ActId::new("test.resolve.generation").unwrap(),
        primitive: ActPrimitive::Custom,
        slots: vec![
            SlotSignature::new(
                "target",
                SlotKind::Referent(ReferentType::File),
                Arity::Required,
            )
            .unwrap(),
        ],
        reversibility: Reversibility::Free,
        blast_radius: BlastRadius::Local,
        executor_hint: ExecutorHint::Praxis,
        reverse_recipe: None,
        description: None,
    };
    let resolved = ResolvedAct::empty(act);
    let provenance = Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now());
    let directive = Directive::new(SpeechAct::Directive, resolved).bind_slot(
        ResolvedSlot::new(
            "target",
            ResolvedSlotValue::Referent(ReferentValue::Anaphor(AnaphorRef::new("this").unwrap())),
            provenance,
        )
        .unwrap(),
    );
    match g {
        Some(g) => directive.with_generation(g),
        None => directive,
    }
}

// --- Property 1: bridge preserves u64 --------------------------------------

#[test]
fn bridge_generation_preserves_raw_u64() {
    let sg = sensorium_core::Generation::new(0xDEAD_BEEF);
    let pg = bridge_generation(sg);
    assert_eq!(pg.into_inner(), 0xDEAD_BEEF);
}

#[test]
fn bridge_generation_round_trips_arbitrary_values() {
    for v in [0_u64, 1, 7, 42, u64::MAX / 2, u64::MAX - 1, u64::MAX] {
        let sg = sensorium_core::Generation::new(v);
        let pg = bridge_generation(sg);
        assert_eq!(
            pg.into_inner(),
            sg.into_inner(),
            "bridge must preserve u64 value {v}"
        );
    }
}

// --- Property 2: INITIAL crosses cleanly -----------------------------------

#[test]
fn bridge_generation_round_trips_initial() {
    let pg = bridge_generation(sensorium_core::Generation::INITIAL);
    assert_eq!(pg, pneuma_core::Generation::INITIAL);
}

// --- Property 3: resolve_directive preserves a Some(Generation) ------------

#[test]
fn resolve_directive_preserves_generation() {
    let g = pneuma_core::Generation::new(0xBEEF);
    let directive = directive_with_file_anaphor(Some(g));
    assert_eq!(directive.generation(), Some(g));

    let ctx = ctx_with_focused_file("/tmp/resolved.txt");
    let resolved = resolve_directive(directive, &ctx).expect("resolve");

    assert_eq!(
        resolved.generation(),
        Some(g),
        "resolver must preserve directive.generation"
    );
    // Sanity: the anaphor actually got resolved (so we know resolve_directive
    // did its real work).
    let binding_value = &resolved.act.bindings[0].value;
    assert!(
        matches!(
            binding_value,
            ResolvedSlotValue::Referent(ReferentValue::File(_))
        ),
        "anaphor must be resolved to a File referent; got {binding_value:?}"
    );
}

// --- Property 4: resolve_directive preserves a None generation -------------

#[test]
fn resolve_directive_preserves_none_generation() {
    let directive = directive_with_file_anaphor(None);
    assert!(directive.generation().is_none());

    let ctx = ctx_with_focused_file("/tmp/none.txt");
    let resolved = resolve_directive(directive, &ctx).expect("resolve");

    assert!(
        resolved.generation().is_none(),
        "resolver must not synthesize a generation when the input had none"
    );
}

// --- Property 5: bridge composes with Directive::with_generation -----------

#[test]
fn end_to_end_voice_to_directive_generation_chain() {
    // Simulate the wiring: voice substrate emits a sensorium Generation,
    // we bridge it through to pneuma, attach it to a freshly-parsed
    // directive, run the resolver, confirm preservation.
    let voice_gen = sensorium_core::Generation::new(123_456);
    let pneuma_gen = bridge_generation(voice_gen);

    let directive = directive_with_file_anaphor(Some(pneuma_gen));
    let ctx = ctx_with_focused_file("/tmp/end_to_end.txt");
    let resolved = resolve_directive(directive, &ctx).expect("resolve");

    assert_eq!(resolved.generation().unwrap().into_inner(), 123_456);
}
