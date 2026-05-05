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
| [`pneuma-demo`](crates/pneuma-demo) | Runnable binary + library — wires the entire stack; reads `MIL_UTTERANCE` env var; deterministic utterance parser; rename, navigate, and switch-app flows. | 35 + 3 ignored |

**Total tests:** 191 · 4 ignored (macOS interactive) · all green on `cargo test --workspace`. All clippy-clean under `-D warnings + pedantic`.

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
# Default: walks the rename demo on a tempdir file (new_name="new.txt")

$ MIL_UTTERANCE='rename to bar.txt' cargo run -p pneuma-demo
# Parses the utterance; new_name comes from "to bar.txt"

$ MIL_UTTERANCE='rn report.md' cargo run -p pneuma-demo
# Terse alias form; "rn" resolves to file.rename

$ MIL_UTTERANCE='navigate to https://example.com' cargo run -p pneuma-demo
# macOS only: opens the URL in Safari and prompts for undo
# Linux/Windows: surfaces typed PlatformUnsupported

$ MIL_UTTERANCE='go example.com' cargo run -p pneuma-demo
# Same; "go" / "browse" are aliases for browser.navigate

$ MIL_UTTERANCE='switch to Safari' cargo run -p pneuma-demo
# macOS only: activates Safari (or any registered app); no undo
# Linux/Windows: surfaces typed PlatformUnsupported

$ MIL_UTTERANCE='switch Visual Studio Code' cargo run -p pneuma-demo
# Multi-word app names work
```

The user presses Enter to commit, `u` to undo, `q` to cancel. Journal is
preserved at the printed tempdir path on every exit. Every run leaves a
reproducible trace.

The browser-navigate flow opens Safari real on macOS via AppleScript;
on Linux/Windows the executor surfaces a typed
`PraxisError::PlatformUnsupported` and the demo records a `Failed`
journal entry — the contract chain is identical, only the platform
gate differs. See
[`docs/mil/router-and-harness.md`](../../docs/mil/router-and-harness.md)
for the architectural argument.

## What you cannot do yet

- Use voice input (step #17, design references in `superwhisper-voice-ecosystem` entity)
- Reference deictics ("this", "that") (step #18, blocked on step #15)
- Address a specific Claude Code window (step #15 + #16 + #18)
- Run on real files outside a tempdir (no permission gates)
- Multi-browser navigate (v0.2 is Safari-only; `RestoreUrl.browser: String` is future-proofed)

See [`docs/mil/router-and-harness.md`](../../docs/mil/router-and-harness.md)
for *why the architecture supports all of this even though none of it is
wired yet* — the router is a pure function; agent harnesses (Claude Code,
Cursor, Arcan internal) are downstream of `Dispatch::Arcan(AgentPrompt)`.

## Status

v0.2.0 — Tier 2 (single-act demo) complete, Phase 1.1 (real Observer)
complete, Phase 2.1 (deterministic parser) complete, **step #13
(`browser.navigate`) and step #14 (`workspace.switch_app`) Praxis acts
+ demo flows complete**. Tier 3 — the empirical milestone where a human
user runs a full work session and we measure the bandwidth-reframe
metric — is now runnable on macOS for rename, navigate, and switch-app
flows. Next steps: #15 (`sensorium-context-macos`), #16 (Arcan bridge
stdio), #17 (sensorium-voice).

## Cross-references

- [`MIL-PROJECT.md`](../../MIL-PROJECT.md) §10.7–§10.13 — pneuma crate build notes
- [`MIL-PROJECT.md`](../../MIL-PROJECT.md) §11 — build order and current status
- [`MIL-PROJECT.md`](../../MIL-PROJECT.md) §13 — design decisions
- [`docs/mil/progress-snapshot-tier-2-complete.md`](../../docs/mil/progress-snapshot-tier-2-complete.md) — current state across both repos
- [`docs/mil/router-and-harness.md`](../../docs/mil/router-and-harness.md) — pure-function router vs. agent harness
