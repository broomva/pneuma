# pneuma-ratify

Approval-channel FSM for MIL — maps user input to typed approval
decisions.

## Design

- [`ApprovalDecision`] — typed enum of the six discourse moves
  (Engage / Commit / Cancel / Approve / Reject / Undo) plus a
  free-form `Clarify(String)`.
- [`Ratifier`] — trait. `read_decision(&mut self) -> ApprovalDecision`.
- [`StdinRatifier`] — production impl that reads a line from stdin
  and parses the first character.
- [`MockRatifier`] — test impl that pulls decisions from a queue,
  for driving the FSM in tests without I/O.

## Status

v0.2.0 — Tier 2 Week 3. Hotkey-only ratification per spec §11.4. v0.3
will add gesture-based ratification via `sensorium-gesture`.
