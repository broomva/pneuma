# pneuma-resolver — deictic / anaphor resolution for MIL

The boundary between **utterance surface** and **bound referent**.

`pneuma-resolver` walks a [`Directive`] before it finalizes, finds slot
bindings of `ReferentValue::Anaphor(...)` ("this", "that", "the focused
window"), and replaces them with concrete typed referents read from a
[`sensorium_core::WorkspaceContext`].

Step #18 of `MIL-PROJECT.md` §11.2. After this crate ships, utterances
like `"refactor this"` work on the actually-focused file rather than
requiring an explicit path.

## What this crate is

A pure function:

```rust,ignore
pub fn resolve_directive(
    directive: Directive<Composing>,
    context: &WorkspaceContext,
) -> Result<Directive<Composing>, ResolverError>;
```

Plus a per-anaphor helper:

```rust,ignore
pub fn resolve_anaphor(
    anaphor: &AnaphorRef,
    slot_type: ReferentType,
    context: &WorkspaceContext,
) -> Result<ReferentValue, ResolverError>;
```

Both are deterministic, sync, and free of I/O. The resolver inspects
the slot's declared `ReferentType` to decide which axis of the context
to read from:

| Slot type      | Axis                          | Surface forms that resolve     |
|----------------|-------------------------------|--------------------------------|
| `File`         | `context.focused_file`        | this, that, the file, current  |
| `Window`       | `context.focused_window`      | this window, the focused window|
| `App`          | `context.focused_app`         | this app, the active application|
| `Any`          | preference: file > window > app | any of the above             |

## What this crate is NOT

- **Not a parser.** Anaphor slots arrive already-tagged as
  `ReferentValue::Anaphor(AnaphorRef)`. Producing those is the parser
  / demo's job.
- **Not turn-history aware.** "It" referring to the previous turn's
  result is a v0.3 feature requiring conversational state.
- **Not a model.** No LLM. Pure deterministic resolution against a
  workspace snapshot.
- **Not a workspace observer.** That's `sensorium-context-macos`
  (step #15). The resolver consumes any `WorkspaceContext`; tests use
  hand-built snapshots.

## Status

v0.2.0 — pure deterministic resolver, single function, no async, no
deps on macOS frameworks. Tests run on every platform.
