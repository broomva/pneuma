# Pneuma — in-flight intent for the Life Agent OS

Pneuma is the *living* intent stream for [MIL — the Multimodal Intent
Language](../../MIL-PROJECT.md). The directive contract, the parser,
the binder, the router, the bridges to executors. Distinct from Lago
(memory, persistent journal): **Pneuma owns intent's life; Lago owns
its afterlife.**

## Crates

| Crate | Role | Tests |
|---|---|---|
| [`pneuma-core`](crates/pneuma-core) | Directive contract — typestate `Composing → Ready → Proposed → Committed`, `PolicyEnvelope`, calibrated `Confidence`, `Tagged<T>`, `Modifier`, `Referent`, `AgentResponse`. | 48 |
| [`pneuma-acts`](crates/pneuma-acts) | 30-act seed registry + `ActRegistry` with case-insensitive verb-alias lookup (`"rn"` → `file.rename`). | 25 |
| [`pneuma-router`](crates/pneuma-router) | Pure dispatch function: `dispatch(d, ctx) -> Dispatch{Praxis, Arcan, Spaces, Custom, Refuse}`. ~150 LoC. | 9 |
| [`pneuma-praxis-bridge`](crates/pneuma-praxis-bridge) | First executor — `LocalPraxis` runs `file.read/rename/copy/write` with typed `ReverseAction`. | 15 |
| [`pneuma-lago-bridge`](crates/pneuma-lago-bridge) | Append-only NDJSON journal — `JournalRecord::{Committed, Executed, Reversed, Cancelled, Failed}`. | 8 |
| [`pneuma-hud`](crates/pneuma-hud) | Pure rendering — every directive state + outcomes + errors → ASCII frames. | 14 |
| [`pneuma-ratify`](crates/pneuma-ratify) | Approval-channel FSM — `ApprovalDecision`, `Ratifier` trait, `StdinRatifier`, `MockRatifier`. | 15 |
| [`pneuma-demo`](crates/pneuma-demo) | Runnable binary + library — wires the entire stack; reads `MIL_UTTERANCE` env var; deterministic utterance parser. | 26 |

**Total tests:** 160 · all green on `cargo test --workspace`. All clippy-clean
under `-D warnings + pedantic`.

Path-deps `sensorium-context` (sibling repo at `../sensorium`) so the same
CI matrix applies. See [`MIL-PROJECT.md`](../../MIL-PROJECT.md) §10 for the
full crate-by-crate notes.

Phase-2.2+ crates planned (not yet built): `pneuma-binder` (cross-modal
temporal binding, needed once Sensorium has multiple producers),
`pneuma-resolver` (anaphora + workspace resolution — "this", "that", "the
focused window"), `pneuma-arcan-bridge` (forward `AgentPrompt` to a
Claude Code / Cursor / Codex stdio interface), `pneuma-predication-model`
(LLM-bound predication, v0.3+, behind a feature flag).

## What you can do today

```sh
$ cargo run -p pneuma-demo
# Walks the demo on a tempdir file with default new_name="new.txt"

$ MIL_UTTERANCE='rename to bar.txt' cargo run -p pneuma-demo
# Parses the utterance; new_name comes from "to bar.txt"

$ MIL_UTTERANCE='rn report.md' cargo run -p pneuma-demo
# Terse alias form; "rn" resolves to file.rename
```

The user presses Enter to commit, `u` to undo, `q` to cancel. Journal is
preserved at the printed tempdir path on every exit. Every run leaves a
reproducible trace.

## What you cannot do yet

- Talk to a real OS app (browser, terminal, Claude Code window)
- Use voice input
- Reference deictics ("this", "that")
- Run on real files outside a tempdir (no permission gates)
- Address a specific Claude Code window

See [`docs/mil/router-and-harness.md`](../../docs/mil/router-and-harness.md)
for *why the architecture supports all of this even though none of it is
wired yet* — the router is a pure function; agent harnesses (Claude Code,
Cursor, Arcan internal) are downstream of `Dispatch::Arcan(AgentPrompt)`.

## Status

v0.2.0 — Tier 2 (single-act demo) complete, Phase 1.1 (real Observer)
complete, Phase 2.1 (deterministic parser) complete. Tier 3 — the
empirical milestone — requires step #13 (one real OS-control act, e.g.
`browser.navigate` via AppleScript, ~200 LoC) before it can start.

## Cross-references

- [`MIL-PROJECT.md`](../../MIL-PROJECT.md) §10.7–§10.13 — pneuma crate build notes
- [`MIL-PROJECT.md`](../../MIL-PROJECT.md) §11 — build order and current status
- [`MIL-PROJECT.md`](../../MIL-PROJECT.md) §13 — design decisions
- [`docs/mil/progress-snapshot-tier-2-complete.md`](../../docs/mil/progress-snapshot-tier-2-complete.md) — current state across both repos
- [`docs/mil/router-and-harness.md`](../../docs/mil/router-and-harness.md) — pure-function router vs. agent harness
