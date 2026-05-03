# Pneuma — in-flight intent for the Life Agent OS

Pneuma is the *living* intent stream for [MIL — the Multimodal Intent
Language](https://github.com/broomva). The directive contract, the parser,
the binder, the router. Distinct from Lago (memory, persistent journal) —
**Pneuma owns intent's life; Lago owns its afterlife.**

## Crates

- [`pneuma-core`](crates/pneuma-core) — the directive contract: typestate
  lifecycle, policy envelope, calibrated confidence, `Tagged<T>` provenance.
  Types-only foundation that every downstream crate depends on.

Phase-1+ crates planned: `pneuma-binder` (cross-modal temporal binding),
`pneuma-parser` (streaming FSM), `pneuma-resolver` (anaphora + workspace
resolution), `pneuma-router` (pure dispatch function), `pneuma-acts`
(act registry data), and bridges to Arcan / Praxis / Lago / Spaces.

## Status

v0.2.0 — `pneuma-core` types-only, phase 0 of the MIL build order. See
[MIL-PROJECT.md](../../MIL-PROJECT.md) for the full project spec.
