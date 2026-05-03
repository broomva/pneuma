# pneuma-core

The directive contract — types-only foundation for the Pneuma subsystem.

`pneuma-core` defines what intents look like and the lifecycle they go
through:

- **`Directive<S>`** — typestate-parameterized lifecycle with phantom-typed
  states `Composing`, `Ready`, `Proposed`, `Committed`. Compile-time
  guarantees that a directive can't dispatch from the wrong state.
- **`PolicyEnvelope`** — the safety contract attached to every directive:
  reversibility, blast radius, ratification needs, validity window,
  permitted executors, redaction rules.
- **`Confidence`** — calibrated, per-slot decomposable. `is_calibrated`
  is honest; uncalibrated scores get a 20% effective penalty against
  thresholds.
- **`Tagged<T>`** — universal provenance wrapper. Every typed value
  carries its confidence, source tokens, and binding kind. Nothing in
  the contract is bare.

The contract enforces five categorical guarantees:

1. No directive dispatches without all required slots bound.
2. No directive dispatches with mismatched referent types.
3. No directive dispatches below its policy envelope's confidence
   threshold.
4. No irreversible-or-large-blast directive bypasses ratification.
5. Every committed directive carries the workspace snapshot it was
   committed against.

It performs no I/O, holds no mutable state, runs no parsers or routers.
Those live in downstream crates.

## Status

v0.2.0. Types-only. Used by `pneuma-router`, `pneuma-parser`,
`pneuma-binder`, `pneuma-resolver`, and the Arcan / Praxis / Lago bridges.

See [MIL-PROJECT.md](../../../../MIL-PROJECT.md) §6 for the directive
contract spec and §11 for the full build order.
