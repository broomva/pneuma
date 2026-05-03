//! Binary entrypoint for the Tier 2 demo.
//!
//! Sets up a tempdir, writes `old.txt`, runs the rename demo, prints
//! HUD frames to stdout, prompts the user at the approval steps via
//! stdin.
//!
//! See `lib.rs` for the library surface — integration tests drive the
//! same flow with a [`pneuma_ratify::MockRatifier`].

#![allow(clippy::print_stderr, clippy::print_stdout)] // demo binary

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use pneuma_demo::{Demo, DemoConfig, manual_observer_for};
use pneuma_ratify::StdinRatifier;

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
    // Set up a tempdir with a file to rename.
    let work_dir = tempdir_with_fixture()?;
    let source_path = work_dir.path.join("old.txt");
    fs::write(&source_path, "alpha")?;
    let journal_path = work_dir.path.join("demo.journal.ndjson");

    println!("┌─ MIL Tier 2 demo ───────────────────────────────────────────────────────");
    println!("│ workdir:     {}", work_dir.path.display());
    println!("│ source:      {}", source_path.display());
    println!("│ journal:     {}", journal_path.display());
    println!("│ proposed →   rename old.txt to new.txt");
    println!("└──────────────────────────────────────────────────────────────────────────");
    println!();
    let _ = std::io::stdout().flush();

    let config = DemoConfig {
        source_path: &source_path,
        new_name: "new.txt",
        journal_path: &journal_path,
        hud_width: 80,
    };
    let stdout = std::io::stdout();
    let handle = stdout.lock();
    // Silent StdinRatifier: the demo prints its own context-specific
    // prompts so the StdinRatifier should not echo a competing one.
    let ratifier = StdinRatifier {
        prompt: String::new(),
    };
    // Build a ManualObserver pre-populated with the focused file.
    // The real producer model: a sensorium-context observer feeds
    // the substrate; pneuma-router queries when it needs context.
    let observer = Box::new(manual_observer_for(&source_path));
    let mut demo = Demo::new(config, handle, ratifier, observer)
        .map_err(|e| std::io::Error::other(format!("setup: {e}")))?;

    let result = demo.run_rename();
    // Drop the demo before printing summaries so the journal flushes.
    drop(demo);

    // Always preserve the tempdir so the user can inspect the journal,
    // even on cancel / failure. (UX finding #3 from the user review.)
    let journal_path_for_summary = journal_path.clone();
    std::mem::forget(work_dir.guard);

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
            println!("│ journal: {}", journal_path_for_summary.display());
            println!("└──");
            Ok(())
        }
        Err(e) => Err(std::io::Error::other(format!("{e}"))),
    }
}

struct DirAndGuard {
    path: PathBuf,
    guard: tempfile::TempDir,
}

fn tempdir_with_fixture() -> std::io::Result<DirAndGuard> {
    let guard = tempfile::tempdir()?;
    let path = guard.path().to_path_buf();
    Ok(DirAndGuard { path, guard })
}
