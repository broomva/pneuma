# pneuma-acts

The canonical MIL act registry — data only.

`pneuma-acts` defines ~30 acts spanning file operations, workspace
control, agent dispatch, and Spaces messaging. Each act carries:

- A stable `ActId` (`file.rename`, `agent.refactor`, etc.)
- Slot signatures (name, kind, arity)
- Intrinsic `(Reversibility, BlastRadius)` for the policy envelope
- Executor hint (`Praxis` / `Arcan` / `Spaces` / `Any`)
- Reverse-action recipe identifier (when reversible)

This crate is *data*: no dispatch logic, no parsing, no I/O. Routers
and parsers consume it via [`registry()`] which returns a `Vec<Act>` of
the canonical set. Downstream crates may extend with their own acts
through the same `Act` type — there is no global registry magic.

## Status

v0.2.0. The act set is intentionally small for the Tier 2 build (see
[MIL-PROJECT.md][spec] §11). v0.3 will expand the registry as user
testing surfaces missing verbs.

[spec]: ../../../../MIL-PROJECT.md
