//! Binary entrypoint for the Tier 2 demo.
//!
//! Four flows after step #16b of MIL §11.2:
//!
//! - **Rename flow** (default and any `file.rename` utterance) — sets up
//!   a tempdir with `old.txt`, runs the canonical rename loop, prompts
//!   the user via stdin.
//! - **Navigate flow** (any `browser.navigate` utterance) — runs the
//!   correction loop for navigating the frontmost browser tab. On macOS
//!   it actually opens Safari via AppleScript; on Linux/Windows the
//!   executor surfaces a typed `PlatformUnsupported`.
//! - **Switch-app flow** (any `workspace.switch_app` utterance) — runs
//!   the correction loop for activating an app.
//! - **Arcan flow** (any `agent.refactor`, `agent.explain`,
//!   `agent.review`, `agent.generate` utterance) — forwards the
//!   directive's instruction to a Claude Code subprocess via the
//!   `pneuma-arcan-bridge` `StdioCommandArcan::claude_code()` builder.
//!   Surfaces the agent's response in a HUD frame and journals it as
//!   `AgentExecuted`. Requires `claude` on PATH; use
//!   `MIL_AGENT_COMMAND=...` to override.
//!
//! Dispatch is purely on the parsed act id — the demo binary itself
//! does not know what an act *means*; it just routes to the right
//! `Demo::run_*` flow. See `lib.rs` for the library surface.
//!
//! ## Environment variables
//!
//! - `MIL_UTTERANCE` — natural-language utterance (parsed deterministically).
//! - `MIL_VOICE_INPUT` — when set, drive `sensorium-voice` to obtain
//!   the utterance from voice input.
//! - `MIL_VOICE_BACKEND` — pick the STT backend. `mock` (default,
//!   uses `MIL_VOICE_MOCK`) or `parakeet` (real on-device NVIDIA
//!   Parakeet TDT EOU streaming inference; requires this binary
//!   to be built with `--features parakeet`).
//! - `MIL_VOICE_MOCK` — canned response when `MIL_VOICE_BACKEND=mock`
//!   (default `"explain this"`).
//! - `MIL_AGENT_COMMAND` — override the agent CLI command (default `claude`).
//! - `MIL_AGENT_ARGS` — comma-separated args (default `--print`).
//!
//! When `MIL_UTTERANCE` is unset and `MIL_VOICE_INPUT` is unset, the
//! demo defaults to the canonical rename flow (rename `old.txt` to
//! `new.txt`).

#![allow(clippy::print_stderr, clippy::print_stdout)] // demo binary

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pneuma_acts::ActRegistry;
use pneuma_arcan_bridge::StdioCommandArcan;
use pneuma_core::act::ResolvedSlotValue;
use pneuma_core::{AnaphorRef, FileRef, ReferentValue};
use pneuma_demo::{Demo, DemoConfig, ParsedUtterance, manual_observer_for, parse_utterance};
use pneuma_ratify::StdinRatifier;
use pneuma_resolver::is_deictic_surface;
use sensorium_context::{ManualObserver, Observer};
use sensorium_context_macos::MacOsWorkspaceObserver;
use sensorium_core::{PrimitiveToken, Timestamp};
use sensorium_voice::{Backend as VoiceBackend, VoiceConfig, VoiceSession};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("demo failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> std::io::Result<()> {
    let work_dir = tempdir_for_journal()?;
    let journal_path = work_dir.path.join("demo.journal.ndjson");

    // Source the utterance from one of three places, in priority order:
    // 1. MIL_UTTERANCE env var (typed text).
    // 2. MIL_VOICE_INPUT env var → drive sensorium-voice to obtain a
    //    transcript. The backend is selected by MIL_VOICE_BACKEND
    //    (`mock` default, or `parakeet` for real on-device inference
    //    when this binary is built with `--features parakeet`).
    // 3. Neither set → default rename flow.
    //
    // If the user explicitly asked for voice (MIL_VOICE_INPUT set)
    // but voice fails, surface the error rather than silently
    // falling through to the rename flow — they'd never know what
    // went wrong otherwise.
    let utterance_env = if let Ok(typed) = std::env::var("MIL_UTTERANCE") {
        Some(typed).filter(|s| !s.trim().is_empty())
    } else if std::env::var("MIL_VOICE_INPUT").is_ok() {
        Some(listen_via_voice()?)
    } else {
        None
    };
    let registry = ActRegistry::canonical();
    let parsed = utterance_env
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .and_then(|s| match parse_utterance(s, &registry) {
            Ok(p) => Some(p),
            Err(err) => {
                eprintln!("demo: could not parse utterance ({err}); using default flow");
                None
            }
        });

    // Branch on the parsed act id.
    match parsed.as_ref().map(|p| p.act_id.as_str()) {
        Some("browser.navigate") => {
            let result = run_navigate_flow(parsed.expect("matched Some"), &journal_path);
            std::mem::forget(work_dir.guard);
            result
        }
        Some("workspace.switch_app") => {
            let result = run_switch_app_flow(parsed.expect("matched Some"), &journal_path);
            std::mem::forget(work_dir.guard);
            result
        }
        Some(act) if act.starts_with("agent.") => {
            let result = run_arcan_flow(parsed.expect("matched Some"), &journal_path);
            std::mem::forget(work_dir.guard);
            result
        }
        Some("file.rename") | None => {
            let result = run_rename_flow(parsed, &work_dir.path, &journal_path);
            std::mem::forget(work_dir.guard);
            result
        }
        Some(other) => {
            eprintln!(
                "demo: utterance resolved to act `{other}` — v0.2 demo handles file.rename, browser.navigate, workspace.switch_app, and agent.*. Falling back to rename."
            );
            let result = run_rename_flow(parsed, &work_dir.path, &journal_path);
            std::mem::forget(work_dir.guard);
            result
        }
    }
}

// --- Rename flow -----------------------------------------------------------

fn run_rename_flow(
    parsed: Option<ParsedUtterance>,
    work_dir: &Path,
    journal_path: &Path,
) -> std::io::Result<()> {
    let source_path = work_dir.join("old.txt");
    fs::write(&source_path, "alpha")?;

    // Extract `new_name` from the parsed utterance if it was a rename;
    // otherwise default. (Other parsed acts that fell through to rename
    // — e.g. unsupported v0.2 acts — keep the default.)
    let (new_name, utterance_for_directive) = match parsed {
        Some(p) if p.act_id.as_str() == "file.rename" => {
            let nn = p
                .payload_slots
                .iter()
                .find(|(k, _)| k == "new_name")
                .map(|(_, v)| v.clone());
            if let Some(name) = nn {
                (name, Some(p.utterance))
            } else {
                eprintln!("demo: utterance parsed but no new_name extracted; using default");
                ("new.txt".to_owned(), Some(p.utterance))
            }
        }
        Some(p) => ("new.txt".to_owned(), Some(p.utterance)),
        None => ("new.txt".to_owned(), None),
    };

    println!("┌─ MIL Tier 2 demo · file.rename ─────────────────────────────────────────");
    println!("│ workdir:     {}", work_dir.display());
    println!("│ source:      {}", source_path.display());
    println!("│ journal:     {}", journal_path.display());
    if let Some(u) = utterance_for_directive.as_deref() {
        println!("│ utterance:   {u}");
    }
    println!("│ proposed →   rename old.txt to {new_name}");
    println!("└──────────────────────────────────────────────────────────────────────────");
    println!();
    let _ = std::io::stdout().flush();

    let config = DemoConfig {
        source_path: &source_path,
        new_name: &new_name,
        journal_path,
        hud_width: 80,
        utterance: utterance_for_directive.as_deref(),
    };
    let stdout = std::io::stdout();
    let handle = stdout.lock();
    let ratifier = StdinRatifier {
        prompt: String::new(),
    };
    let observer = Box::new(manual_observer_for(&source_path));
    let mut demo = Demo::new(config, handle, ratifier, observer)
        .map_err(|e| std::io::Error::other(format!("setup: {e}")))?;

    let result = demo.run_rename();
    drop(demo);
    print_summary(result, journal_path)
}

// --- Navigate flow ---------------------------------------------------------

fn run_navigate_flow(parsed: ParsedUtterance, journal_path: &Path) -> std::io::Result<()> {
    // The parser put a single `url` slot in the payload.
    let url = parsed
        .payload_slots
        .iter()
        .find(|(k, _)| k == "url")
        .map(|(_, v)| v.clone())
        .ok_or_else(|| {
            std::io::Error::other("parser produced no `url` slot for browser.navigate")
        })?;

    println!("┌─ MIL Tier 2 demo · browser.navigate ────────────────────────────────────");
    println!("│ journal:     {}", journal_path.display());
    println!("│ utterance:   {}", parsed.utterance);
    println!("│ proposed →   navigate Safari front tab to {url}");
    if !cfg!(target_os = "macos") {
        println!(
            "│ note:        non-macOS host detected — execution will surface PlatformUnsupported"
        );
    }
    println!("└──────────────────────────────────────────────────────────────────────────");
    println!();
    let _ = std::io::stdout().flush();

    // Navigate flow doesn't need a focused file in the workspace
    // observer — the URL is the slot binding. Use a fresh
    // ManualObserver as the substrate.
    let config = DemoConfig {
        // `source_path` is rename-specific; pass an empty PathBuf.
        // The navigate flow doesn't read it.
        source_path: Path::new(""),
        new_name: "",
        journal_path,
        hud_width: 80,
        utterance: Some(&parsed.utterance),
    };
    let stdout = std::io::stdout();
    let handle = stdout.lock();
    let ratifier = StdinRatifier {
        prompt: String::new(),
    };
    let observer = Box::new(ManualObserver::new(Timestamp::now()));
    let mut demo = Demo::new(config, handle, ratifier, observer)
        .map_err(|e| std::io::Error::other(format!("setup: {e}")))?;

    let result = demo.run_navigate(&url);
    drop(demo);
    print_summary(result, journal_path)
}

// --- Switch-app flow -------------------------------------------------------

fn run_switch_app_flow(parsed: ParsedUtterance, journal_path: &Path) -> std::io::Result<()> {
    let app_name = parsed
        .payload_slots
        .iter()
        .find(|(k, _)| k == "target")
        .map(|(_, v)| v.clone())
        .ok_or_else(|| {
            std::io::Error::other("parser produced no `target` slot for workspace.switch_app")
        })?;

    println!("┌─ MIL Tier 2 demo · workspace.switch_app ────────────────────────────────");
    println!("│ journal:     {}", journal_path.display());
    println!("│ utterance:   {}", parsed.utterance);
    println!("│ proposed →   activate macOS application: {app_name}");
    if !cfg!(target_os = "macos") {
        println!(
            "│ note:        non-macOS host detected — execution will surface PlatformUnsupported"
        );
    }
    println!("└──────────────────────────────────────────────────────────────────────────");
    println!();
    let _ = std::io::stdout().flush();

    let config = DemoConfig {
        source_path: Path::new(""),
        new_name: "",
        journal_path,
        hud_width: 80,
        utterance: Some(&parsed.utterance),
    };
    let stdout = std::io::stdout();
    let handle = stdout.lock();
    let ratifier = StdinRatifier {
        prompt: String::new(),
    };
    let observer = Box::new(ManualObserver::new(Timestamp::now()));
    let mut demo = Demo::new(config, handle, ratifier, observer)
        .map_err(|e| std::io::Error::other(format!("setup: {e}")))?;

    let result = demo.run_switch_app(&app_name);
    drop(demo);
    print_summary(result, journal_path)
}

// --- Arcan flow ------------------------------------------------------------

fn run_arcan_flow(parsed: ParsedUtterance, journal_path: &Path) -> std::io::Result<()> {
    let act_id = parsed.act_id.as_str().to_owned();

    // Promote parser-extracted slots into typed referents:
    //
    // - `target` that's a recognized deictic surface form ("this",
    //   "the focused window", etc.) → `Anaphor(AnaphorRef)`. The
    //   resolver in `pneuma-resolver` (step #18) replaces this with
    //   a concrete typed referent before finalize.
    // - `target` that resolves to an existing file path → `File(FileRef)`.
    // - `target` that doesn't resolve to a file (free-form noun phrase
    //   like "the auth module" or "MIL") → `Url(String)` as a
    //   free-form string-shaped Referent. v0.3 may route a smarter
    //   parser through resolver+context.
    // - Everything else → bare String slot.
    let mut payload_slots: Vec<(String, ResolvedSlotValue)> = Vec::new();
    for (name, value) in &parsed.payload_slots {
        if name == "target" {
            let typed = if is_deictic_surface(value) {
                let anaphor = AnaphorRef::new(value)
                    .expect("non-empty deictic surface must construct AnaphorRef");
                ResolvedSlotValue::Referent(ReferentValue::Anaphor(anaphor))
            } else if std::path::Path::new(value).exists() {
                ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new(value)))
            } else {
                ResolvedSlotValue::Referent(ReferentValue::Url(value.clone()))
            };
            payload_slots.push((name.clone(), typed));
        } else {
            payload_slots.push((name.clone(), ResolvedSlotValue::String(value.clone())));
        }
    }

    let arcan = build_arcan_executor_from_env();

    println!("┌─ MIL Tier 2 demo · {act_id} ──────────────────────────────────────────");
    println!("│ journal:     {}", journal_path.display());
    println!("│ utterance:   {}", parsed.utterance);
    println!("│ executor:    {}", arcan.executor_label());
    println!(
        "│ command:     {} {}",
        arcan.command(),
        arcan.args().join(" ")
    );
    println!("└──────────────────────────────────────────────────────────────────────────");
    println!();
    let _ = std::io::stdout().flush();

    let config = DemoConfig {
        source_path: Path::new(""),
        new_name: "",
        journal_path,
        hud_width: 80,
        utterance: Some(&parsed.utterance),
    };
    let stdout = std::io::stdout();
    let handle = stdout.lock();
    let ratifier = StdinRatifier {
        prompt: String::new(),
    };
    // Step #15 wiring: real macOS workspace observer for the arcan
    // flow. Polls NSWorkspace at 250ms; on non-macOS hosts the
    // observer is a stub (returns empty context). The resolver sees
    // a populated focused_app whenever a real GUI session is active,
    // letting "explain this app" / "refactor this" route to the
    // correct entity.
    //
    // We give the observer a beat to populate via an initial eager
    // poll before running the directive lifecycle, so the very first
    // turn has a fresh context.
    let observer: Box<dyn Observer> =
        match MacOsWorkspaceObserver::start(std::time::Duration::from_millis(250)) {
            Ok(obs) => {
                std::thread::sleep(std::time::Duration::from_millis(300));
                Box::new(obs)
            }
            Err(err) => {
                eprintln!(
                    "demo: macOS observer failed to start ({err}); falling back to empty observer"
                );
                Box::new(ManualObserver::new(Timestamp::now()))
            }
        };
    let mut demo = Demo::new(config, handle, ratifier, observer)
        .map_err(|e| std::io::Error::other(format!("setup: {e}")))?;

    let result = demo.run_arcan(&act_id, &parsed.utterance, payload_slots, &arcan);
    drop(demo);
    print_summary(result, journal_path)
}

/// Build a `StdioCommandArcan` from environment variables, falling back
/// to the Claude Code default. `MIL_AGENT_COMMAND` overrides the binary
/// name; `MIL_AGENT_ARGS` provides comma-separated arguments.
fn build_arcan_executor_from_env() -> StdioCommandArcan {
    let cmd = std::env::var("MIL_AGENT_COMMAND").ok();
    let args = std::env::var("MIL_AGENT_ARGS").ok();
    match (cmd, args) {
        (Some(c), Some(a)) => StdioCommandArcan::new(c, a.split(',').map(str::trim), "custom"),
        (Some(c), None) => StdioCommandArcan::new(c, Vec::<String>::new(), "custom"),
        (None, Some(a)) => StdioCommandArcan::new(
            "claude",
            a.split(',').map(str::trim),
            "claude-code-customargs",
        ),
        (None, None) => StdioCommandArcan::claude_code(),
    }
}

// --- Shared output ---------------------------------------------------------

fn print_summary(
    result: Result<pneuma_demo::DemoSummary, pneuma_demo::DemoError>,
    journal_path: &Path,
) -> std::io::Result<()> {
    match result {
        Ok(summary) => {
            println!();
            println!("┌─ summary ─");
            println!("│ directive-id: {}", summary.directive_id.into_inner());
            println!("│ reversed:     {}", summary.reversed);
            println!("│ journal:      {}", summary.journal_path.display());
            println!("└──");
            Ok(())
        }
        Err(pneuma_demo::DemoError::Cancelled) => {
            println!();
            println!("┌─ summary ─");
            println!("│ status:  cancelled");
            println!("│ journal: {}", journal_path.display());
            println!("└──");
            Ok(())
        }
        Err(e) => Err(std::io::Error::other(format!("{e}"))),
    }
}

// --- Voice input ----------------------------------------------------------

/// Backend selector parsed from `MIL_VOICE_BACKEND`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VoiceBackendKind {
    /// Programmable mock — emits the canned `MIL_VOICE_MOCK` value
    /// on flush. No microphone access. Default when the env var is
    /// unset or empty.
    Mock,
    /// Real on-device NVIDIA Parakeet TDT EOU streaming inference.
    /// Requires `--features parakeet` at build time and a working
    /// microphone at runtime.
    Parakeet,
}

/// Parse `MIL_VOICE_BACKEND`. `None` and the empty string default
/// to `Mock`. Trailing/leading whitespace is trimmed; matching is
/// case-insensitive. Unknown values return an `io::Error` with a
/// helpful message.
fn parse_voice_backend(raw: Option<&str>) -> std::io::Result<VoiceBackendKind> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(VoiceBackendKind::Mock),
        Some(s) => match s.to_ascii_lowercase().as_str() {
            "mock" => Ok(VoiceBackendKind::Mock),
            "parakeet" => Ok(VoiceBackendKind::Parakeet),
            other => Err(std::io::Error::other(format!(
                "MIL_VOICE_BACKEND={other:?} not recognized; expected 'mock' or 'parakeet'"
            ))),
        },
    }
}

/// Drive a `sensorium_voice::VoiceSession` to obtain an utterance.
///
/// Dispatches on `MIL_VOICE_BACKEND`:
///
/// - `mock` (default) — synchronous: emits the `MIL_VOICE_MOCK` value
///   on `flush()`. No mic access. Suitable for CI / scripted demos.
/// - `parakeet` — opens the default microphone, runs `EnergyVad`
///   gated audio through Parakeet TDT EOU streaming inference, and
///   returns the first complete utterance. Requires the binary to
///   be built with `--features parakeet`.
fn listen_via_voice() -> std::io::Result<String> {
    let raw = std::env::var("MIL_VOICE_BACKEND").ok();
    match parse_voice_backend(raw.as_deref())? {
        VoiceBackendKind::Mock => listen_via_mock(),
        VoiceBackendKind::Parakeet => listen_via_parakeet(),
    }
}

/// Mock voice path — flushes a canned response immediately.
fn listen_via_mock() -> std::io::Result<String> {
    let canned = std::env::var("MIL_VOICE_MOCK").unwrap_or_else(|_| "explain this".to_owned());

    println!("┌─ MIL voice input · sensorium-voice ──────────────────────────────────────");
    println!("│ backend: mock");
    println!("│ utterance source: MIL_VOICE_MOCK env var (defaults to 'explain this')");
    println!("│ transcript: {canned}");
    println!("└──────────────────────────────────────────────────────────────────────────");
    println!();

    let mut session = VoiceSession::new(VoiceConfig {
        backend: VoiceBackend::mock(canned),
        ..VoiceConfig::default()
    })
    .map_err(|e| std::io::Error::other(format!("voice session: {e}")))?;
    let rx = session
        .tokens()
        .ok_or_else(|| std::io::Error::other("voice tokens already taken"))?;
    session
        .flush()
        .map_err(|e| std::io::Error::other(format!("voice flush: {e}")))?;
    let token = rx
        .recv_timeout(std::time::Duration::from_millis(500))
        .map_err(|_| std::io::Error::other("voice session produced no token"))?;
    match token {
        PrimitiveToken::Predication(t) => Ok(t.value),
        other => Err(std::io::Error::other(format!(
            "voice session produced non-predication token: {other:?}"
        ))),
    }
}

/// Parakeet voice path. Behind `feature = "parakeet"` so default
/// builds stay dep-light; without the feature this returns a
/// helpful error pointing the user at `cargo run --features parakeet`.
#[cfg(not(feature = "parakeet"))]
fn listen_via_parakeet() -> std::io::Result<String> {
    Err(std::io::Error::other(
        "MIL_VOICE_BACKEND=parakeet, but pneuma-demo was built without `--features parakeet`. \
         Rebuild with `cargo run -p pneuma-demo --features parakeet` (this links parakeet-rs + ort + \
         hf-hub and downloads ~150MB of model weights on first run).",
    ))
}

/// Parakeet voice path — opens mic, runs `EnergyVad`-gated samples
/// through Parakeet TDT EOU, returns the first utterance's
/// transcript. Times out after `MIL_VOICE_TIMEOUT_SECS` seconds
/// (default 30) to prevent runaway sessions.
#[cfg(feature = "parakeet")]
fn listen_via_parakeet() -> std::io::Result<String> {
    use std::time::{Duration, Instant};

    use ringbuf::traits::Consumer;
    use sensorium_voice::{AudioCapture, AudioCaptureConfig, EnergyVad, VadGate};

    let timeout_secs: u64 = std::env::var("MIL_VOICE_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    let mut session = VoiceSession::new(VoiceConfig::parakeet_default())
        .map_err(|e| std::io::Error::other(format!("voice session: {e}")))?;
    let rx = session
        .tokens()
        .ok_or_else(|| std::io::Error::other("voice tokens already taken"))?;

    let mut capture = AudioCapture::start(&AudioCaptureConfig::default())
        .map_err(|e| std::io::Error::other(format!("audio capture: {e}")))?;
    let mut consumer = capture
        .consumer()
        .ok_or_else(|| std::io::Error::other("audio consumer already taken"))?;

    println!("┌─ MIL voice input · sensorium-voice ──────────────────────────────────────");
    println!("│ backend: parakeet (NVIDIA Parakeet TDT EOU, on-device)");
    println!("│ device:  {}", capture.device_name());
    println!("│ source:  {} Hz → 16 kHz", capture.source_sample_rate());
    println!("│ timeout: {timeout_secs}s (override with MIL_VOICE_TIMEOUT_SECS)");
    println!("│ speak now…");
    println!("└──────────────────────────────────────────────────────────────────────────");
    println!();

    let mut vad = EnergyVad::new();
    let mut gate = VadGate::new();
    let mut scratch = [0.0_f32; 1024];
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut first_utterance_seen = false;

    while !first_utterance_seen && Instant::now() < deadline {
        let n = consumer.pop_slice(&mut scratch);
        if n == 0 {
            // No new samples yet; yield to the audio thread instead
            // of busy-spinning.
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        let utterances = session
            .run_vad_driven(scratch[..n].iter().copied(), &mut vad, &mut gate)
            .map_err(|e| std::io::Error::other(format!("vad-driven: {e}")))?;
        if utterances > 0 {
            first_utterance_seen = true;
        }
    }

    if !first_utterance_seen {
        // Hit the deadline without VAD closing an utterance — flush
        // anyway so any in-flight audio surfaces as a Final delta.
        session
            .flush()
            .map_err(|e| std::io::Error::other(format!("voice flush: {e}")))?;
    }

    capture.stop();

    // Drain all tokens; keep the most recent Predication's text. Both
    // Partial and Final deltas surface as Predications today (the
    // session emits both); the last one written is the most complete.
    let mut transcript = String::new();
    while let Ok(token) = rx.try_recv() {
        if let PrimitiveToken::Predication(t) = token {
            transcript = t.value;
        }
    }
    let transcript = transcript.trim().to_owned();
    if transcript.is_empty() {
        return Err(std::io::Error::other(
            "voice session produced an empty transcript (timeout or silence?)",
        ));
    }
    println!("│ transcript: {transcript}");
    println!();
    Ok(transcript)
}

// --- Tempdir helper ---------------------------------------------------------

struct DirAndGuard {
    path: PathBuf,
    guard: tempfile::TempDir,
}

fn tempdir_for_journal() -> std::io::Result<DirAndGuard> {
    let guard = tempfile::tempdir()?;
    let path = guard.path().to_path_buf();
    Ok(DirAndGuard { path, guard })
}

// --- Tests -----------------------------------------------------------------
//
// Kept at the bottom of the file so `clippy::items_after_test_module`
// stays happy.

#[cfg(test)]
mod tests {
    use super::{VoiceBackendKind, parse_voice_backend};

    #[test]
    fn unset_defaults_to_mock() {
        assert_eq!(parse_voice_backend(None).unwrap(), VoiceBackendKind::Mock);
    }

    #[test]
    fn empty_string_defaults_to_mock() {
        assert_eq!(
            parse_voice_backend(Some("")).unwrap(),
            VoiceBackendKind::Mock
        );
        assert_eq!(
            parse_voice_backend(Some("   ")).unwrap(),
            VoiceBackendKind::Mock
        );
    }

    #[test]
    fn mock_value_returns_mock() {
        assert_eq!(
            parse_voice_backend(Some("mock")).unwrap(),
            VoiceBackendKind::Mock
        );
    }

    #[test]
    fn parakeet_value_returns_parakeet() {
        assert_eq!(
            parse_voice_backend(Some("parakeet")).unwrap(),
            VoiceBackendKind::Parakeet
        );
    }

    #[test]
    fn case_insensitive_matching() {
        assert_eq!(
            parse_voice_backend(Some("PARAKEET")).unwrap(),
            VoiceBackendKind::Parakeet
        );
        assert_eq!(
            parse_voice_backend(Some("Mock")).unwrap(),
            VoiceBackendKind::Mock
        );
    }

    #[test]
    fn whitespace_is_trimmed() {
        assert_eq!(
            parse_voice_backend(Some("  parakeet  ")).unwrap(),
            VoiceBackendKind::Parakeet
        );
    }

    #[test]
    fn unknown_value_errors_with_guidance() {
        let err = parse_voice_backend(Some("whisper")).expect_err("must error");
        let msg = err.to_string();
        assert!(msg.contains("whisper"), "must mention bad value: {msg}");
        assert!(
            msg.contains("mock") && msg.contains("parakeet"),
            "must list valid choices: {msg}"
        );
    }
}
