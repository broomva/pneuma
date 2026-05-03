# pneuma-lago-bridge

Minimal journaling bridge for MIL — appends every committed directive and
every Praxis execution outcome to a JSON-lines file.

## Status

v0.2.0 — Tier 2 Week 2. Stand-in for the planned Lago subsystem of the
Life Agent OS. The wire format is structured and stable enough that a
future real-Lago bridge can replay these journals.

## Design

Append-only NDJSON. One line per [`JournalRecord`]. Records are
self-describing — they carry their kind tag so a replayer can dispatch
without external context.

Records:

- `committed` — a [`pneuma_core::Directive<Committed>`] just landed
- `executed` — a `LocalPraxis` execution succeeded
- `reverse` — an undo operation completed
- `cancelled` — the user explicitly cancelled / rejected
- `failed` — an executor returned an error

Read with [`JournalReader::iter`] — yields parsed records in append order.

## Why JSONL not a DB

For Tier 2 we want a journal we can `cat` to debug, `tail -f` to watch,
and `grep` to search. A SQLite-backed Lago is a v0.3+ concern.
