//! Integration tests proving the demo works against any `Observer`
//! impl, including the real filesystem-watching `FsObserver`.
//!
//! Properties:
//!
//! - The demo runs against a `ManualObserver` and uses its
//!   pre-populated focused file. (Already covered in `full_loop.rs`;
//!   re-tested here with explicit observer construction.)
//! - The demo runs against a `FsObserver` watching the source dir.
//!   Real filesystem events flow into the substrate; the demo
//!   queries the substrate at finalize time.
//! - The substrate is updated by the FsObserver as the rename
//!   happens — drift is observable across snapshots.

use std::fs;
use std::time::Duration;

use pneuma_demo::{Demo, DemoConfig};
use pneuma_lago_bridge::{JournalReader, JournalRecord};
use pneuma_ratify::{ApprovalDecision, MockRatifier};
use sensorium_context::{FsObserver, ManualObserver, Observer};
use sensorium_core::Timestamp;

#[test]
fn demo_runs_against_manual_observer_with_explicit_construction() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("old.txt");
    fs::write(&source_path, "alpha").unwrap();
    let journal_path = dir.path().join("journal.ndjson");

    let observer = ManualObserver::new(Timestamp::now());
    observer.set_focused_file(sensorium_core::FileRef::new(&source_path), true);
    // Snapshot before the demo runs to confirm the observer holds
    // the focused file.
    let snap_pre = observer.snapshot();
    assert_eq!(
        snap_pre.state.visible_files,
        vec![sensorium_core::FileRef::new(&source_path)]
    );

    let mut out = Vec::<u8>::new();
    let result = {
        let mut demo = Demo::new(
            DemoConfig {
                source_path: &source_path,
                new_name: "new.txt",
                journal_path: &journal_path,
                hud_width: 60,
            },
            &mut out,
            MockRatifier::from_decisions(vec![ApprovalDecision::Commit, ApprovalDecision::Undo]),
            Box::new(observer),
        )
        .unwrap();
        demo.run_rename()
    };
    let summary = result.unwrap();
    assert!(summary.reversed);

    let records: Vec<_> = JournalReader::open(&journal_path)
        .iter()
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(records.len(), 3);
    assert!(matches!(records[0], JournalRecord::Committed { .. }));
    assert!(matches!(records[1], JournalRecord::Executed { .. }));
    assert!(matches!(records[2], JournalRecord::Reversed { .. }));
}

#[test]
fn fs_observer_drift_is_visible_across_demo_runs() {
    // Two-stage check:
    //   1. Run a demo against the FsObserver.
    //   2. Verify that filesystem events the demo itself caused
    //      (the rename) are reflected in the observer's substrate
    //      after the demo finishes.
    //
    // This proves Tier 2 Risk #6 in the demo path: drift surfaces
    // when a real producer is wired in.
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("old.txt");
    fs::write(&source_path, "alpha").unwrap();
    let journal_path = dir.path().join("journal.ndjson");

    let observer = FsObserver::watch(dir.path(), false).unwrap();
    // Give notify a moment to set up its watcher.
    std::thread::sleep(Duration::from_millis(50));
    let snap_pre = observer.snapshot();
    let pre_activity_count = snap_pre.state.recent_activity.ring.len();

    // We need to read the observer's current state via the same
    // observer instance the demo uses, but Box::new takes ownership.
    // So clone the inner ManualObserver via the FsObserver's
    // accessor first, then move the FsObserver into the demo.
    let inner_for_check = observer.manual().clone();

    let mut out = Vec::<u8>::new();
    {
        let mut demo = Demo::new(
            DemoConfig {
                source_path: &source_path,
                new_name: "new.txt",
                journal_path: &journal_path,
                hud_width: 60,
            },
            &mut out,
            MockRatifier::from_decisions(vec![ApprovalDecision::Commit, ApprovalDecision::Cancel]),
            Box::new(observer),
        )
        .unwrap();
        let _summary = demo.run_rename().unwrap();
    }

    // Wait for filesystem events to propagate.
    std::thread::sleep(Duration::from_millis(400));

    // The clone shares state with the FsObserver, so we observe the
    // *post-demo* state including any events the demo's rename
    // caused.
    let snap_post = inner_for_check.snapshot();
    assert!(
        !snap_pre.observes_same_state(&snap_post),
        "demo's rename must register on the FsObserver substrate"
    );
    let post_activity_count = snap_post.state.recent_activity.ring.len();
    assert!(
        post_activity_count > pre_activity_count,
        "FsObserver should have recorded the rename event(s); pre={} post={}",
        pre_activity_count,
        post_activity_count
    );
}
