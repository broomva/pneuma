# pneuma-hud

Pure rendering for MIL's live-preview HUD.

## What this crate does

Takes a directive in any typestate (or a `PraxisCall` outcome / a
`ContractError` / a `PraxisError`) and produces an ASCII frame
suitable for a terminal pane. **No I/O, no terminal control, no
state.** A consumer that wants a live HUD calls a render function on
each state transition and prints the result; a fancier consumer
diffs frames and patches the screen.

## What this crate is NOT

- **Not a TUI library.** Frames are plain `String`. Add `crossterm` /
  `ratatui` upstream if you want raw input or partial redraws.
- **Not the production HUD.** Production wants 30 Hz repaint, GPU
  glass effect, Vision Pro overlay, etc. — all v0.3+. This is the
  Tier 2 Week 3 stand-in that proves the *contract* — what info the
  HUD needs to display.
- **Not stateful.** Each render is a pure function of its input.
  The caller decides cadence.

## Architectural finding (Risk #3 from synthesis)

The synthesis doc flagged "live-preview cadence" as a risk that only
surfaces at Tier 2. This crate's stance: **render-on-state-change**
for v0.2 demo. 30 Hz repaint is a v0.3 production concern.
Documented in `lib.rs` module docs.

## Surface

- [`HudFrame`] — owned `String` plus typed `kind` for journaling.
- [`HudRenderer`] — opaque renderer. Configurable width.
- Per-state methods: `render_composing`, `render_ready`,
  `render_proposed`, `render_committed`, `render_outcome`,
  `render_contract_error`, `render_praxis_error`.

See [MIL-PROJECT.md](../../../../MIL-PROJECT.md) §8 for the correction
loop the HUD is part of.
