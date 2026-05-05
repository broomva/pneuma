# pneuma-arcan-bridge — agent-harness adapter

The first agent-runtime executor for MIL — translates a
[`pneuma_router::AgentPrompt`] into a subprocess invocation against an
external agent CLI.

Sibling of [`pneuma-praxis-bridge`](../pneuma-praxis-bridge); same trait
shape, different semantics:

- **Praxis** executes deterministically (filesystem, AppleScript).
- **Arcan** delegates to an LLM-in-the-loop subprocess
  (Claude Code, Codex, Aider, your own harness).

## What this crate is

Step #16 of `MIL-PROJECT.md` §11.2. Three pieces:

1. **The `ArcanExecutor` trait** — `execute(&self, prompt: &AgentPrompt)
   -> Result<ArcanOutcome, ArcanError>`. Same shape as
   `pneuma_praxis_bridge::Executor::execute` but distinct concrete
   types because the semantics differ (Praxis returns ReverseAction,
   Arcan returns the agent's response text).
2. **`MockArcan`** — scripted canned-response executor for tests.
3. **`StdioCommandArcan`** — generic subprocess wrapper. Configurable
   for any CLI that accepts a prompt on stdin (or as an argument) and
   returns a response on stdout. Defaults to a Claude Code-shaped
   invocation, but swappable.

## What this crate is NOT

- **Not Claude Code-specific.** The default `StdioCommandArcan::claude_code()`
  builder configures the subprocess for Claude Code's CLI, but the
  trait itself is agent-agnostic.
- **Not async.** v0.2 ships sync subprocess execution. A future
  `AsyncArcanExecutor` trait can wrap Tokio if streaming becomes
  required.
- **Not a journal.** That's `pneuma-lago-bridge`. This crate just
  *returns* enough information for the journal to record.
- **Not a context resolver.** Slots arrive already-resolved in the
  `AgentPrompt`. Resolving deictics ("this", "that") is
  `pneuma-resolver`'s job (step #18).

## Architecture

The bridge consists of three layers:

```text
AgentPrompt
   │
   ▼  prompt_to_string(prompt) — serialize structured prompt to plain text
String prompt
   │
   ▼  ArcanExecutor::execute(prompt) — trait method, sync
   │      ├─ MockArcan: returns canned text
   │      └─ StdioCommandArcan: spawns subprocess, captures stdout
ArcanOutcome { response, ... }
```

## Status

v0.2.0 — types, trait, mock executor, generic stdio subprocess
executor. Demo integration in a follow-up PR. See `MIL-PROJECT.md`
§11.2.
