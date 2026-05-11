//! Tests for the streaming-substrate primitives in `pneuma-core`:
//! `Generation` (the per-stream monotonic counter, byte-compatible with
//! `sensorium_core::Generation`) and the `Directive.generation` field
//! that propagates through the typestate lifecycle.
//!
//! The architectural rule is that `pneuma-core` does not depend on
//! `sensorium-core` (acyclicity). So we mirror `Generation` here — same
//! shape, same wire format. The cross-crate bridge lives in
//! `pneuma-resolver` (which depends on both).
//!
//! Properties under test:
//!
//! 1. **Generation basics** — `Copy`, `Eq`, `Ord`, `Hash`, transparent
//!    serde (encodes as a bare `u64`). `INITIAL = 0`. `new(n).into_inner()
//!    == n`.
//! 2. **Directive defaults to no generation** — a freshly-constructed
//!    `Directive<Composing>` has `generation() == None`.
//! 3. **`with_generation` attaches a generation** — and `generation()`
//!    returns it.
//! 4. **Generation propagates through `try_finalize`** — `Composing →
//!    Ready` preserves the field.
//! 5. **Generation propagates through `propose` → `ratify`** — `Ready →
//!    Proposed → Committed` preserves the field.
//! 6. **Generation propagates through `commit`** — `Ready → Committed`
//!    preserves the field.
//! 7. **Generation propagates through `reject_for_amendment`** —
//!    `Proposed → Composing` preserves the field.
//! 8. **Serde round-trip** — a directive serialized with a generation
//!    deserializes with the same generation.
//! 9. **Serde omits `generation` when `None`** — the wire format stays
//!    compact for non-streaming callers.

use chrono::Utc;
use pneuma_core::act::ResolvedSlotValue;
use pneuma_core::{
    Act, ActId, ActPrimitive, Arity, BindingKind, BlastRadius, Composing, Confidence,
    ConfidenceProducer, ConfidenceScore, ContextRef, ContextSnapshotId, Directive, ExecutorHint,
    FileRef, Generation, PolicyEnvelope, Provenance, ReferentType, ReferentValue, ResolvedAct,
    ResolvedSlot, Reversibility, SlotKind, SlotSignature, SpeechAct,
};

// --- Fixtures ---------------------------------------------------------------
//
// Mirror the pattern in `lifecycle.rs`: build a tiny `file.read` act
// (Free reversibility + Local blast → low threshold so confidence
// 0.95 cleared the policy without fiddling).

fn read_act() -> Act {
    Act {
        id: ActId::new("file.read").unwrap(),
        primitive: ActPrimitive::Custom,
        slots: vec![
            SlotSignature::new(
                "file",
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
    }
}

fn calibrated_score(value: f32) -> ConfidenceScore {
    ConfidenceScore::new(value, true, ConfidenceProducer::Deterministic).unwrap()
}

fn deterministic_provenance() -> Provenance {
    Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now())
}

fn bound_file_slot(name: &str, path: &str) -> ResolvedSlot {
    ResolvedSlot::new(
        name,
        ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new(path))),
        deterministic_provenance(),
    )
    .unwrap()
}

fn fresh_context_ref() -> ContextRef {
    ContextRef::new(ContextSnapshotId::new(), Utc::now())
}

fn confidence_for_file_slot() -> Confidence {
    Confidence::from_slots(vec![("file".to_owned(), calibrated_score(0.95))]).unwrap()
}

fn composing_with_generation(g: Generation) -> Directive<Composing> {
    let act = read_act();
    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    let _ = policy;
    let resolved = ResolvedAct::empty(act);
    Directive::new(SpeechAct::Directive, resolved)
        .bind_slot(bound_file_slot("file", "/tmp/x.txt"))
        .with_generation(g)
}

fn permissive_policy() -> PolicyEnvelope {
    PolicyEnvelope::intrinsic(Reversibility::Free, BlastRadius::Local)
}

// --- Property 1: Generation basics -----------------------------------------

#[test]
fn generation_initial_is_zero() {
    assert_eq!(Generation::INITIAL.into_inner(), 0);
}

#[test]
fn generation_into_inner_round_trips() {
    let g = Generation::new(12_345);
    assert_eq!(g.into_inner(), 12_345);
}

#[test]
fn generation_is_copy_eq_hash() {
    use std::collections::HashSet;
    let g = Generation::new(7);
    let c = g;
    assert_eq!(g, c);
    let mut s = HashSet::new();
    s.insert(g);
    assert!(s.contains(&c));
}

#[test]
fn generation_orders_monotonically() {
    let g0 = Generation::new(0);
    let g1 = Generation::new(1);
    let g2 = Generation::new(2);
    assert!(g0 < g1);
    assert!(g1 < g2);
}

#[test]
fn generation_serializes_transparently_as_u64() {
    let g = Generation::new(7);
    let json = serde_json::to_string(&g).expect("serialize");
    assert_eq!(json, "7");
    let back: Generation = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, g);
}

// --- Property 2: Directive defaults to no generation -----------------------

#[test]
fn fresh_directive_has_no_generation() {
    let resolved = ResolvedAct::empty(read_act());
    let d: Directive<Composing> = Directive::new(SpeechAct::Directive, resolved);
    assert!(d.generation().is_none());
}

// --- Property 3: with_generation attaches a generation ---------------------

#[test]
fn with_generation_attaches_value() {
    let g = Generation::new(42);
    let d = composing_with_generation(g);
    assert_eq!(d.generation(), Some(g));
}

// --- Property 4: Generation survives Composing -> Ready --------------------

#[test]
fn try_finalize_preserves_generation() {
    let g = Generation::new(11);
    let composing = composing_with_generation(g);
    let ready = composing
        .try_finalize(
            fresh_context_ref(),
            permissive_policy(),
            confidence_for_file_slot(),
        )
        .expect("finalize");
    assert_eq!(ready.generation(), Some(g));
}

// --- Property 5: Generation survives Ready -> Proposed -> Committed --------

#[test]
fn propose_then_ratify_preserves_generation() {
    let g = Generation::new(13);
    let composing = composing_with_generation(g);
    let ready = composing
        .try_finalize(
            fresh_context_ref(),
            permissive_policy(),
            confidence_for_file_slot(),
        )
        .expect("finalize");
    let proposed = ready.propose();
    assert_eq!(proposed.generation(), Some(g));
    let committed = proposed.ratify();
    assert_eq!(committed.generation(), Some(g));
}

// --- Property 6: Generation survives Ready -> Committed (direct commit) ----

#[test]
fn direct_commit_preserves_generation() {
    let g = Generation::new(17);
    let composing = composing_with_generation(g);
    let ready = composing
        .try_finalize(
            fresh_context_ref(),
            permissive_policy(),
            confidence_for_file_slot(),
        )
        .expect("finalize");
    let committed = ready.commit().expect("commit");
    assert_eq!(committed.generation(), Some(g));
}

// --- Property 7: Generation survives Proposed -> Composing -----------------

#[test]
fn reject_for_amendment_preserves_generation() {
    let g = Generation::new(19);
    let composing = composing_with_generation(g);
    let ready = composing
        .try_finalize(
            fresh_context_ref(),
            permissive_policy(),
            confidence_for_file_slot(),
        )
        .expect("finalize");
    let proposed = ready.propose();
    let back_to_composing = proposed.reject_for_amendment();
    assert_eq!(back_to_composing.generation(), Some(g));
}

// --- Property 8: Serde round-trip preserves generation ---------------------

#[test]
fn serialized_directive_round_trips_with_generation() {
    let g = Generation::new(23);
    let composing = composing_with_generation(g);
    let json = serde_json::to_string(&composing).expect("serialize");
    assert!(
        json.contains("\"generation\":23"),
        "generation must appear in wire format: {json}"
    );
    let back: Directive<Composing> = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.generation(), Some(g));
}

// --- Property 9: Serde omits generation when None --------------------------

#[test]
fn serialized_directive_omits_generation_when_none() {
    let resolved = ResolvedAct::empty(read_act());
    let composing: Directive<Composing> = Directive::new(SpeechAct::Directive, resolved);
    let json = serde_json::to_string(&composing).expect("serialize");
    assert!(
        !json.contains("generation"),
        "wire format must omit generation when None: {json}"
    );
}
