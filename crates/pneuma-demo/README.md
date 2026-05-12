# pneuma-demo

The runnable demo binary for **MIL Tier 2** — rename a file via a typed
directive, with live preview, ratification, journaling, and undo.

## What this demonstrates

Every load-bearing edge of the Tier 2 contract, end-to-end, with real
I/O:

1. A real `WorkspaceContext` (sensorium-core).
2. A real `Directive<Composing>` (pneuma-core typestate).
3. The pure router `dispatch` (pneuma-router).
4. A real `LocalPraxis` execution against a tempdir (pneuma-praxis-bridge).
5. A real NDJSON journal (pneuma-lago-bridge).
6. The HUD rendering each state transition (pneuma-hud).
7. Ratification via stdin keypresses (pneuma-ratify).
8. Undo, re-journaled.

## How to run

```sh
cargo run -p pneuma-demo
```

The demo creates a tempdir, writes `old.txt` with content `"alpha"`,
walks through the contract, prints every HUD frame, and prompts you
at the ratification step. Press Enter to commit, `u` to undo, `q` to
quit.

### Voice input

Set `MIL_VOICE_INPUT=1` to drive the directive from speech instead
of typed text. Pick the backend with `MIL_VOICE_BACKEND`:

| Value     | Behavior                                                                     |
|-----------|------------------------------------------------------------------------------|
| _unset_   | Falls back to `mock`.                                                        |
| `mock`    | Returns the value of `MIL_VOICE_MOCK` (default `"explain this"`). No mic.    |
| `parakeet`| Opens the default mic, runs `EnergyVad` + Parakeet TDT EOU streaming, and    |
|           | returns the first complete utterance. **Requires `--features parakeet`.**    |

Examples:

```sh
# Mock — fully scripted, no mic.
MIL_VOICE_INPUT=1 MIL_VOICE_MOCK="rename it to bar.txt" cargo run -p pneuma-demo

# Real Parakeet on M-series Apple Silicon. First run downloads
# ~150MB of weights into ~/.cache/sensorium-voice/parakeet-eou/.
MIL_VOICE_INPUT=1 MIL_VOICE_BACKEND=parakeet \
    cargo run -p pneuma-demo --features parakeet
```

`MIL_VOICE_TIMEOUT_SECS` (default 30) caps how long the parakeet path
waits for the first utterance before giving up.

#### Streaming output (parakeet)

The parakeet path drains `VoiceSession::streaming_tokens()` — the
generation-tagged substrate from `sensorium-core` — and renders each
update live on stderr:

```text
│ partial (gen=0): rename
│ partial (gen=0): rename it  →  file.rename
│ partial (gen=0): rename it to alpha dot txt  →  file.rename
│ FINAL (gen=0): rename it to alpha dot txt
```

Each `partial` line shows the running transcript at that generation;
the `→  file.rename` suffix appears when the partial is already
parseable, proving the parser layer reacts to partials, not just on
final. `FINAL` lands when the VAD closes the utterance — the demo
then dispatches the directive as usual. `Generation` increments per
utterance, so a multi-utterance session shows `gen=1`, `gen=2`, etc.

#### Recording-based validation (`MIL_VOICE_INPUT_FILE`)

For reproducible engine validation without a microphone, feed audio
from a WAV file:

```sh
# Generate a test recording with macOS `say` (any TTS works):
say -v Alex -r 140 "Rename it to alpha." -o /tmp/test.aiff
afconvert /tmp/test.aiff /tmp/test.wav -d LEI16@16000 -f WAVE -c 1

# Pipe it through the engine — same trace + streaming path as live mic:
MIL_VOICE_INPUT=1 MIL_VOICE_BACKEND=parakeet \
  MIL_VOICE_INPUT_FILE=/tmp/test.wav \
  cargo run -p pneuma-demo --features parakeet --release
```

The WAV must be PCM mono; sample rate is auto-resampled to 16 kHz.
Supports i16/i24/i32/f32 sample formats (hound-decoded). Bypasses
cpal entirely — same VAD → Parakeet → streaming partials → Final
chain, but the audio source is a file instead of a microphone.

Use this for:
- Regression-testing the engine against a fixed corpus
- Debugging a specific transcription without re-recording
- CI integration tests (deterministic input → deterministic output)
- Iterating on substrate changes without ambient noise

#### Diagnostics (`MIL_VOICE_TRACE`)

For per-chunk runtime diagnostics on either the mic or WAV path:

```sh
MIL_VOICE_TRACE=1 cargo run -p pneuma-demo --features parakeet --release
```

Surfaces audio RMS, VAD probabilities, gate transitions, and feed/flush
events on stderr. Invaluable for diagnosing "why didn't my voice
register" (usually a mic gain / VAD threshold mismatch) or "why is
Parakeet not transcribing" (usually too-short audio or audio quality).

## What it is NOT

- Not a production application. Everything is a single-process
  in-memory demo.
- Not interactive in a fancy way. Plain stdout printing; no raw
  terminal mode. Each render is a fresh frame appended to your
  scrollback.
- Not a test of the *bandwidth reframe*. That is a Tier 3 user trial
  where a human uses MIL for a day. This demo only proves the
  contract loads under real I/O.

## Library surface

The binary is the deliverable, but the crate also exposes:

- [`Demo`] — the assembled stack (substrate + journal + executor +
  HUD + ratifier).
- [`Demo::run_rename`] — the canonical demo flow as a function so
  integration tests can drive it with a `MockRatifier`.

## Status

v0.2.0 — Tier 2 Week 3 deliverable.
