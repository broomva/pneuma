# pneuma-praxis-bridge

The first executor adapter for MIL — translates a `pneuma_router::PraxisCall`
into actual filesystem operations and produces a reverse-action recipe so
the directive can be undone.

## Status

v0.2.0 — Tier 2 Week 2. Supports file-domain acts: `file.read`, `file.rename`,
`file.copy`, `file.write`. Sufficient for the "rename the focused file"
end-to-end demo.

`file.delete` is intentionally not implemented at the bridge level: it is
[`Reversibility::Irreversible`][rev], and v0.2 punts irreversible-real
actions to a future hardened bridge with soft-delete/trash semantics.

## Why a thin executor first

From [`docs/mil/research-synthesis-2026-05-03.md`][synth] (Risk #2 —
reverse-action timing): we need to surface *when* the reverse-action lands
in the journal. This bridge captures the reverse at execution time so the
journal records what was actually done, not what was predicted. A future
Arcan bridge (agent runtime) will capture reverse-actions at *completion*
time instead — same trait, different timing.

[rev]: https://docs.rs/pneuma-core/latest/pneuma_core/policy/enum.Reversibility.html
[synth]: ../../../../docs/mil/research-synthesis-2026-05-03.md

## Surface

- [`Executor`] — the trait. Synchronous; `execute` and `reverse`.
- [`LocalPraxis`] — file-system implementation.
- [`ExecutionOutcome`] — result payload + captured reverse action.
- [`ReverseAction`] — typed reverse recipes.
- [`PraxisError`] — local error type, distinct from `ContractError`.
