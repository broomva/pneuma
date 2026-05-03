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

use pneuma_demo::{Demo, DemoConfig};
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
    let mut demo = Demo::new(config, handle, StdinRatifier::default())
        .map_err(|e| std::io::Error::other(format!("setup: {e}")))?;

    match demo.run_rename() {
        Ok(summary) => {
            println!();
            println!("┌─ summary ─");
            println!("│ directive-id: {}", summary.directive_id.into_inner());
            println!("│ reversed:     {}", summary.reversed);
            println!("│ journal:      {}", summary.journal_path.display());
            println!("└──");
            // Keep tempdir around so the user can inspect the journal.
            std::mem::forget(work_dir.guard);
            Ok(())
        }
        Err(pneuma_demo::DemoError::Cancelled) => {
            eprintln!();
            eprintln!("demo: user cancelled.");
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
