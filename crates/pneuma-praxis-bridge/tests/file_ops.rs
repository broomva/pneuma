//! End-to-end file-op tests for `LocalPraxis`.
//!
//! Properties under test:
//!
//! - `file.read` returns content; reverse-action is `None`.
//! - `file.rename` moves the file; reverse-action restores the original
//!   path; reversing twice errors gracefully.
//! - `file.copy` duplicates; reverse-action deletes the copy.
//! - `file.write` overwrites; reverse-action restores prior content.
//! - Missing / mistyped slots error cleanly without touching disk.
//! - Renaming into an occupied path errors before touching disk.

use std::fs;
use std::path::{Path, PathBuf};

use pneuma_acts::registry;
use pneuma_core::act::ResolvedSlotValue;
use pneuma_core::{ActId, FileRef, ReferentValue};
use pneuma_praxis_bridge::{ExecutionOutcome, Executor, LocalPraxis, PraxisError, ReverseAction};
use pneuma_router::PraxisCall;

// --- Helpers ---------------------------------------------------------------

fn praxis_call(act_id: &str, slots: Vec<(&str, ResolvedSlotValue)>) -> PraxisCall {
    let act = registry()
        .into_iter()
        .find(|a| a.id.as_str() == act_id)
        .unwrap_or_else(|| panic!("registry must contain {act_id}"));
    PraxisCall {
        act_id: ActId::new(act_id).unwrap(),
        slots: slots.into_iter().map(|(n, v)| (n.to_owned(), v)).collect(),
        reverse_recipe: act.reverse_recipe.clone(),
    }
}

fn file_slot(path: &Path) -> ResolvedSlotValue {
    ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new(path.to_path_buf())))
}

// --- file.read -------------------------------------------------------------

#[test]
fn read_returns_content_and_no_reverse_action() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hello.txt");
    fs::write(&path, "hello world").unwrap();

    let call = praxis_call("file.read", vec![("target", file_slot(&path))]);
    let outcome = LocalPraxis.execute(&call).unwrap();

    assert_eq!(outcome.act_id.as_str(), "file.read");
    assert_eq!(outcome.result["bytes"], 11);
    assert_eq!(outcome.result["content_utf8_lossy"], "hello world");
    assert!(outcome.reverse_action.is_none());
}

#[test]
fn read_of_missing_file_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("never.txt");
    let call = praxis_call("file.read", vec![("target", file_slot(&path))]);
    let err = LocalPraxis.execute(&call).unwrap_err();
    assert!(matches!(err, PraxisError::Filesystem(_)));
}

#[test]
fn reversing_a_read_outcome_errors_with_no_reverse_action() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.txt");
    fs::write(&path, "x").unwrap();
    let call = praxis_call("file.read", vec![("target", file_slot(&path))]);
    let outcome = LocalPraxis.execute(&call).unwrap();
    let err = LocalPraxis.reverse(&call, &outcome).unwrap_err();
    assert!(matches!(err, PraxisError::NoReverseAction));
}

// --- file.rename -----------------------------------------------------------

#[test]
fn rename_moves_file_and_records_reverse() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("a.txt");
    fs::write(&from, "alpha").unwrap();

    let call = praxis_call(
        "file.rename",
        vec![
            ("target", file_slot(&from)),
            ("new_name", ResolvedSlotValue::String("b.txt".to_owned())),
        ],
    );
    let outcome = LocalPraxis.execute(&call).unwrap();

    let to = dir.path().join("b.txt");
    assert!(!from.exists(), "source removed");
    assert!(to.exists(), "destination created");
    assert_eq!(fs::read_to_string(&to).unwrap(), "alpha");

    match &outcome.reverse_action {
        ReverseAction::RenameBack { from: f, to: t } => {
            assert_eq!(f, &from);
            assert_eq!(t, &to);
        }
        other => panic!("expected RenameBack, got {other:?}"),
    }
}

#[test]
fn rename_reverse_restores_original_path() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("a.txt");
    fs::write(&from, "alpha").unwrap();

    let call = praxis_call(
        "file.rename",
        vec![
            ("target", file_slot(&from)),
            ("new_name", ResolvedSlotValue::String("b.txt".to_owned())),
        ],
    );
    let outcome = LocalPraxis.execute(&call).unwrap();
    LocalPraxis.reverse(&call, &outcome).unwrap();

    let to = dir.path().join("b.txt");
    assert!(from.exists(), "original path restored");
    assert!(!to.exists(), "renamed path empty");
    assert_eq!(fs::read_to_string(&from).unwrap(), "alpha");
}

#[test]
fn rename_into_occupied_path_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("a.txt");
    let blocker = dir.path().join("b.txt");
    fs::write(&from, "alpha").unwrap();
    fs::write(&blocker, "beta").unwrap();

    let call = praxis_call(
        "file.rename",
        vec![
            ("target", file_slot(&from)),
            ("new_name", ResolvedSlotValue::String("b.txt".to_owned())),
        ],
    );
    let err = LocalPraxis.execute(&call).unwrap_err();
    assert!(matches!(err, PraxisError::ReverseRefused(_)));
    // Neither file moved.
    assert_eq!(fs::read_to_string(&from).unwrap(), "alpha");
    assert_eq!(fs::read_to_string(&blocker).unwrap(), "beta");
}

#[test]
fn rename_reverse_refuses_when_original_path_reoccupied() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("a.txt");
    fs::write(&from, "alpha").unwrap();

    let call = praxis_call(
        "file.rename",
        vec![
            ("target", file_slot(&from)),
            ("new_name", ResolvedSlotValue::String("b.txt".to_owned())),
        ],
    );
    let outcome = LocalPraxis.execute(&call).unwrap();

    // Something else creates a file at the original path before reverse.
    fs::write(&from, "interloper").unwrap();

    let err = LocalPraxis.reverse(&call, &outcome).unwrap_err();
    assert!(matches!(err, PraxisError::ReverseRefused(_)));
    // Renamed file untouched.
    let to = dir.path().join("b.txt");
    assert_eq!(fs::read_to_string(&to).unwrap(), "alpha");
    assert_eq!(fs::read_to_string(&from).unwrap(), "interloper");
}

// --- file.copy -------------------------------------------------------------

#[test]
fn copy_duplicates_file_and_reverse_deletes_copy() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("src.txt");
    fs::write(&source, "data").unwrap();
    let dest = dir.path().join("copy.txt");

    let call = praxis_call(
        "file.copy",
        vec![
            ("target", file_slot(&source)),
            (
                "destination",
                ResolvedSlotValue::String(dest.display().to_string()),
            ),
        ],
    );
    let outcome = LocalPraxis.execute(&call).unwrap();

    assert!(source.exists());
    assert!(dest.exists());
    assert_eq!(fs::read_to_string(&dest).unwrap(), "data");

    LocalPraxis.reverse(&call, &outcome).unwrap();
    assert!(source.exists(), "source preserved through reverse");
    assert!(!dest.exists(), "copy removed by reverse");
}

#[test]
fn copy_into_occupied_path_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("src.txt");
    fs::write(&source, "data").unwrap();
    let dest = dir.path().join("blocker.txt");
    fs::write(&dest, "blocker").unwrap();

    let call = praxis_call(
        "file.copy",
        vec![
            ("target", file_slot(&source)),
            (
                "destination",
                ResolvedSlotValue::String(dest.display().to_string()),
            ),
        ],
    );
    let err = LocalPraxis.execute(&call).unwrap_err();
    assert!(matches!(err, PraxisError::ReverseRefused(_)));
    assert_eq!(fs::read_to_string(&dest).unwrap(), "blocker");
}

// --- file.write ------------------------------------------------------------

#[test]
fn write_overwrites_and_reverse_restores_prior_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.txt");
    fs::write(&path, "original").unwrap();

    let call = praxis_call(
        "file.write",
        vec![
            ("target", file_slot(&path)),
            ("content", ResolvedSlotValue::String("replaced".to_owned())),
        ],
    );
    let outcome = LocalPraxis.execute(&call).unwrap();

    assert_eq!(fs::read_to_string(&path).unwrap(), "replaced");
    match &outcome.reverse_action {
        ReverseAction::RestoreContent { path: p, prior } => {
            assert_eq!(p, &path);
            assert_eq!(prior, b"original");
        }
        other => panic!("expected RestoreContent, got {other:?}"),
    }

    LocalPraxis.reverse(&call, &outcome).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "original");
}

#[test]
fn write_to_new_file_captures_empty_prior() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new.txt");

    let call = praxis_call(
        "file.write",
        vec![
            ("target", file_slot(&path)),
            ("content", ResolvedSlotValue::String("hello".to_owned())),
        ],
    );
    let outcome = LocalPraxis.execute(&call).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "hello");

    // Reverse: prior was empty so file becomes empty, not deleted. This
    // is the documented v0.2 behavior — see lib.rs comment.
    LocalPraxis.reverse(&call, &outcome).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "");
}

// --- Slot validation --------------------------------------------------------

#[test]
fn missing_slot_errors_cleanly() {
    let call = praxis_call("file.rename", vec![]);
    let err = LocalPraxis.execute(&call).unwrap_err();
    assert!(matches!(
        err,
        PraxisError::MissingSlot { ref slot, .. } if slot == "target"
    ));
}

#[test]
fn wrong_slot_kind_errors_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let _path = dir.path().join("x.txt");
    // file.rename expects target: Referent(File). Pass a String instead.
    let call = praxis_call(
        "file.rename",
        vec![
            ("target", ResolvedSlotValue::String("not a file".to_owned())),
            ("new_name", ResolvedSlotValue::String("y.txt".to_owned())),
        ],
    );
    let err = LocalPraxis.execute(&call).unwrap_err();
    assert!(matches!(
        err,
        PraxisError::WrongSlotKind { ref slot, .. } if slot == "target"
    ));
}

#[test]
fn unsupported_act_errors_cleanly() {
    let call = PraxisCall {
        act_id: ActId::new("synthetic.unimplemented").unwrap(),
        slots: Vec::new(),
        reverse_recipe: None,
    };
    let err = LocalPraxis.execute(&call).unwrap_err();
    assert!(matches!(err, PraxisError::UnsupportedAct(ref s) if s == "synthetic.unimplemented"));
}

// --- ExecutionOutcome serialization -----------------------------------------

#[test]
fn outcome_round_trips_through_serde_json() {
    let outcome = ExecutionOutcome {
        act_id: ActId::new("file.rename").unwrap(),
        result: serde_json::json!({"from": "/tmp/a", "to": "/tmp/b"}),
        reverse_action: ReverseAction::RenameBack {
            from: PathBuf::from("/tmp/a"),
            to: PathBuf::from("/tmp/b"),
        },
    };
    let json = serde_json::to_string(&outcome).unwrap();
    let de: ExecutionOutcome = serde_json::from_str(&json).unwrap();
    assert_eq!(de, outcome);
}
