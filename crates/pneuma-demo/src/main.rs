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
//! - `MIL_AGENT_COMMAND` — override the agent CLI command (default `claude`).
//! - `MIL_AGENT_ARGS` — comma-separated args (default `--print`).
//!
//! ## Environment
//!
//! - `MIL_UTTERANCE` — optional natural-language utterance. When unset
//!   or empty, defaults to the canonical rename flow (rename `old.txt`
//!   to `new.txt`).

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
use sensorium_core::Timestamp;

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

    // Parse `MIL_UTTERANCE` upfront so we know which flow to run.
    let utterance_env = std::env::var("MIL_UTTERANCE").ok();
    let registry = ActRegistry::canonical();
    let parsed = utterance_env
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .and_then(|s| match parse_utterance(s, &registry) {
            Ok(p) => Some(p),
            Err(err) => {
                eprintln!("demo: could not parse MIL_UTTERANCE ({err}); using default flow");
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
