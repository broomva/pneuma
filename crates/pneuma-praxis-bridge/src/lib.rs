//! # pneuma-praxis-bridge
//!
//! The first executor adapter for MIL — translates a
//! [`pneuma_router::PraxisCall`] into actual filesystem operations and
//! produces a typed reverse-action recipe so the directive can be undone.
//!
//! ## What this crate is for
//!
//! Tier 2 Week 2 of the MIL build (see `docs/mil/tier-2-build-plan.md`).
//! Three goals:
//!
//! 1. **Prove the dispatch boundary works.** Given a `Dispatch::Praxis(call)`
//!    from the pure router, we can execute it on a real filesystem.
//! 2. **Surface reverse-action timing.** From the synthesis doc Risk #2: do
//!    we capture the reverse before, during, or after execution? This
//!    bridge captures it *during* — the reverse recipe is recorded only
//!    after the action successfully completed, with the actual paths /
//!    sizes / values that were observed at execution time. A future Arcan
//!    bridge will capture the reverse at *completion* (agent supplies
//!    it). Same trait, different timing.
//! 3. **Keep the executor pluggable.** [`Executor`] is the trait;
//!    [`LocalPraxis`] is a particular implementation. Tests use
//!    `LocalPraxis` against `tempfile::TempDir`. Future adapters
//!    (sandboxed Praxis, remote Praxis) plug in here.
//!
//! ## What this crate is NOT
//!
//! - **Not a full Praxis subsystem.** The Life Agent OS plans a `praxis`
//!   subsystem; this crate is a thin executor that lives inside Pneuma's
//!   dependency graph. When the real Praxis ships, the bridge becomes a
//!   wire-format adapter.
//! - **Not async.** Filesystem ops are fast on local disk; v0.2 ships
//!   sync. Adapters that need async (remote Praxis) can wrap.
//! - **Not a journal.** That's `pneuma-lago-bridge`. This crate just
//!   *returns* enough information for the journal to record.
//!
//! ## Reverse-action machinery
//!
//! Every executed call returns an [`ExecutionOutcome`]:
//!
//! - `result` — the success payload (file content, new path, etc.)
//! - `reverse_action` — a typed [`ReverseAction`] capturing what undo
//!   needs.
//!
//! Calling [`Executor::reverse`] with that outcome runs the inverse
//! operation. The substrate guarantee from `MIL-PROJECT.md` §6.3 #5 —
//! "every committed directive carries the workspace snapshot it was
//! committed against" — couples with this so executors can detect drift
//! before reverting.
//!
//! ## Acts supported in v0.2
//!
//! | Act           | Reversibility | Reverse                                |
//! |---------------|---------------|----------------------------------------|
//! | `file.read`   | Free          | None — read has no side effects        |
//! | `file.rename` | Costly        | `RenameBack { from, to }`              |
//! | `file.copy`   | Costly        | `DeleteCopy { path }`                  |
//! | `file.write`  | Costly        | `RestoreContent { path, prior }`       |
//!
//! `file.delete` is intentionally **not** wired here. It is
//! [`Reversibility::Irreversible`][rev]; v0.2 punts irreversible-real
//! operations until a hardened bridge with soft-delete / trash semantics
//! exists.
//!
//! [rev]: pneuma_core::Reversibility::Irreversible

#![doc = include_str!("../README.md")]

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use pneuma_core::act::ResolvedSlotValue;
use pneuma_core::{ActId, ReferentValue};
use pneuma_router::PraxisCall;

// --- Error -----------------------------------------------------------------

/// Errors a [`LocalPraxis`] executor can return.
///
/// Distinct from [`pneuma_core::ContractError`] — that is for contract
/// violations *before* dispatch; `PraxisError` is for execution-time
/// failures *during* dispatch.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PraxisError {
    /// The act is not implemented by this bridge.
    #[error("act `{0}` is not handled by LocalPraxis")]
    UnsupportedAct(String),

    /// A required slot was missing from the [`PraxisCall`]. This should
    /// have been caught by [`pneuma_core::Directive::try_finalize`] — if
    /// it surfaces here, it indicates a contract bug or a synthetic call.
    #[error("required slot `{slot}` missing from {act}")]
    MissingSlot {
        /// The act being executed.
        act: String,
        /// The slot whose binding was missing.
        slot: String,
    },

    /// A slot held the wrong shape of value (e.g. `String` where
    /// `Referent(File)` was expected).
    #[error("slot `{slot}` for {act} had wrong kind: {reason}")]
    WrongSlotKind {
        /// The act being executed.
        act: String,
        /// The slot whose binding was wrong.
        slot: String,
        /// Free-form reason.
        reason: String,
    },

    /// Underlying filesystem error.
    #[error("filesystem error: {0}")]
    Filesystem(#[from] std::io::Error),

    /// A reverse-action was requested for an outcome that has none
    /// (e.g. trying to reverse a `file.read`).
    #[error("outcome has no reverse action")]
    NoReverseAction,

    /// Reverse-action would clobber an existing file unsafely (e.g.
    /// reversing a rename when something is already at the original path).
    #[error("reverse-action refused: {0}")]
    ReverseRefused(String),
}

// --- ReverseAction ---------------------------------------------------------

/// What information the executor needs to undo a successful execution.
///
/// Captured at execution time. Each variant is shaped to the act it
/// reverses, so the reversal handler can pattern-match without a
/// secondary lookup table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ReverseAction {
    /// No reverse needed (read-only acts).
    None,
    /// Rename `to` back to `from`.
    RenameBack {
        /// Original path (where the file used to live).
        from: PathBuf,
        /// New path (where the file lives now after the rename).
        to: PathBuf,
    },
    /// Delete the copy at `path`.
    DeleteCopy {
        /// Path to the copy that was created.
        path: PathBuf,
    },
    /// Restore prior content of `path`. We store the bytes inline; for
    /// large files a future variant could store a content-addressed
    /// blob ID instead.
    RestoreContent {
        /// Path whose content was overwritten.
        path: PathBuf,
        /// Bytes prior to overwrite.
        prior: Vec<u8>,
    },
}

impl ReverseAction {
    /// `true` if this reverse-action is a no-op (read-only acts).
    #[must_use]
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

// --- ExecutionOutcome -------------------------------------------------------

/// What a successful execution returns: a free-form result payload plus
/// the captured reverse action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionOutcome {
    /// Which act produced this outcome (echoed for journal cross-reference).
    pub act_id: ActId,
    /// JSON-shaped result. The shape depends on the act.
    pub result: serde_json::Value,
    /// Reverse-action recipe captured at execution time.
    pub reverse_action: ReverseAction,
}

// --- Executor trait ---------------------------------------------------------

/// The local-executor surface.
///
/// Implementors run a [`PraxisCall`] and produce an [`ExecutionOutcome`].
/// The reverse-action is captured in the outcome; `reverse` consumes
/// the outcome and runs the inverse.
///
/// Synchronous on purpose for v0.2: filesystem on local disk is fast
/// enough that an async API would be premature complexity. Adapters
/// that need async (remote Praxis, networked sandbox) can wrap.
pub trait Executor {
    /// Execute the call. Returns the outcome on success; `PraxisError`
    /// on any failure.
    fn execute(&self, call: &PraxisCall) -> Result<ExecutionOutcome, PraxisError>;

    /// Reverse a previously-executed outcome. The original `call` is
    /// passed for cross-reference (and for executors that need it for
    /// drift detection).
    fn reverse(
        &self,
        original_call: &PraxisCall,
        outcome: &ExecutionOutcome,
    ) -> Result<(), PraxisError>;
}

// --- LocalPraxis ------------------------------------------------------------

/// Filesystem-backed executor — runs file ops via `std::fs`.
///
/// Stateless. Construct one with [`LocalPraxis::new`] and reuse across
/// calls. (No mutex, no shared state, so it's `Send + Sync` for free.)
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalPraxis;

impl LocalPraxis {
    /// Construct a `LocalPraxis`.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Executor for LocalPraxis {
    fn execute(&self, call: &PraxisCall) -> Result<ExecutionOutcome, PraxisError> {
        match call.act_id.as_str() {
            "file.read" => execute_read(call),
            "file.rename" => execute_rename(call),
            "file.copy" => execute_copy(call),
            "file.write" => execute_write(call),
            other => Err(PraxisError::UnsupportedAct(other.to_owned())),
        }
    }

    fn reverse(
        &self,
        original_call: &PraxisCall,
        outcome: &ExecutionOutcome,
    ) -> Result<(), PraxisError> {
        match &outcome.reverse_action {
            ReverseAction::None => Err(PraxisError::NoReverseAction),
            ReverseAction::RenameBack { from, to } => reverse_rename(original_call, from, to),
            ReverseAction::DeleteCopy { path } => reverse_copy(path),
            ReverseAction::RestoreContent { path, prior } => reverse_write(path, prior),
        }
    }
}

// --- Per-act handlers -------------------------------------------------------

fn execute_read(call: &PraxisCall) -> Result<ExecutionOutcome, PraxisError> {
    let target = require_file_slot(call, "target")?;
    let bytes = std::fs::read(&target)?;
    let body = String::from_utf8_lossy(&bytes).into_owned();
    Ok(ExecutionOutcome {
        act_id: call.act_id.clone(),
        result: serde_json::json!({
            "path": target,
            "bytes": bytes.len(),
            "content_utf8_lossy": body,
        }),
        reverse_action: ReverseAction::None,
    })
}

fn execute_rename(call: &PraxisCall) -> Result<ExecutionOutcome, PraxisError> {
    let from = require_file_slot(call, "target")?;
    let new_name = require_string_slot(call, "new_name")?;
    let to = from
        .parent()
        .map_or_else(|| PathBuf::from(&new_name), |p| p.join(&new_name));
    if to.exists() {
        return Err(PraxisError::ReverseRefused(format!(
            "rename target `{}` already exists",
            to.display()
        )));
    }
    std::fs::rename(&from, &to)?;
    Ok(ExecutionOutcome {
        act_id: call.act_id.clone(),
        result: serde_json::json!({
            "from": from,
            "to": to,
        }),
        reverse_action: ReverseAction::RenameBack { from, to },
    })
}

fn execute_copy(call: &PraxisCall) -> Result<ExecutionOutcome, PraxisError> {
    let source = require_file_slot(call, "target")?;
    let destination = require_string_slot(call, "destination")?;
    let dest_path = PathBuf::from(&destination);
    if dest_path.exists() {
        return Err(PraxisError::ReverseRefused(format!(
            "copy destination `{}` already exists",
            dest_path.display()
        )));
    }
    let bytes = std::fs::copy(&source, &dest_path)?;
    Ok(ExecutionOutcome {
        act_id: call.act_id.clone(),
        result: serde_json::json!({
            "source": source,
            "destination": dest_path,
            "bytes": bytes,
        }),
        reverse_action: ReverseAction::DeleteCopy { path: dest_path },
    })
}

fn execute_write(call: &PraxisCall) -> Result<ExecutionOutcome, PraxisError> {
    let target = require_file_slot(call, "target")?;
    let content = require_string_slot(call, "content")?;
    // Capture prior content for the reverse action. If the file didn't
    // exist, the reverse action is "delete".
    let prior = if target.exists() {
        std::fs::read(&target)?
    } else {
        // Use an empty Vec to mark "did not exist before". Reverse handler
        // detects this case via metadata if needed; for simplicity we
        // use an empty Vec to mean "restore as empty file" — the caller
        // who wanted a different behavior should issue a delete instead.
        Vec::new()
    };
    std::fs::write(&target, content.as_bytes())?;
    Ok(ExecutionOutcome {
        act_id: call.act_id.clone(),
        result: serde_json::json!({
            "path": target,
            "bytes": content.len(),
        }),
        reverse_action: ReverseAction::RestoreContent {
            path: target,
            prior,
        },
    })
}

// --- Reverse handlers -------------------------------------------------------

fn reverse_rename(_call: &PraxisCall, from: &PathBuf, to: &PathBuf) -> Result<(), PraxisError> {
    if from.exists() {
        return Err(PraxisError::ReverseRefused(format!(
            "cannot reverse rename: original path `{}` already occupied",
            from.display()
        )));
    }
    std::fs::rename(to, from)?;
    Ok(())
}

fn reverse_copy(path: &PathBuf) -> Result<(), PraxisError> {
    if !path.exists() {
        return Err(PraxisError::ReverseRefused(format!(
            "cannot reverse copy: copy at `{}` is gone",
            path.display()
        )));
    }
    std::fs::remove_file(path)?;
    Ok(())
}

fn reverse_write(path: &PathBuf, prior: &[u8]) -> Result<(), PraxisError> {
    std::fs::write(path, prior)?;
    Ok(())
}

// --- Slot extraction helpers ------------------------------------------------

fn slot<'a>(call: &'a PraxisCall, name: &str) -> Option<&'a ResolvedSlotValue> {
    call.slots
        .iter()
        .find_map(|(n, v)| if n == name { Some(v) } else { None })
}

fn require_file_slot(call: &PraxisCall, name: &str) -> Result<PathBuf, PraxisError> {
    let value = slot(call, name).ok_or_else(|| PraxisError::MissingSlot {
        act: call.act_id.as_str().to_owned(),
        slot: name.to_owned(),
    })?;
    match value {
        ResolvedSlotValue::Referent(ReferentValue::File(file_ref)) => Ok(file_ref.path.clone()),
        other => Err(PraxisError::WrongSlotKind {
            act: call.act_id.as_str().to_owned(),
            slot: name.to_owned(),
            reason: format!("expected Referent(File), got {other:?}"),
        }),
    }
}

fn require_string_slot(call: &PraxisCall, name: &str) -> Result<String, PraxisError> {
    let value = slot(call, name).ok_or_else(|| PraxisError::MissingSlot {
        act: call.act_id.as_str().to_owned(),
        slot: name.to_owned(),
    })?;
    match value {
        ResolvedSlotValue::String(s) => Ok(s.clone()),
        other => Err(PraxisError::WrongSlotKind {
            act: call.act_id.as_str().to_owned(),
            slot: name.to_owned(),
            reason: format!("expected String, got {other:?}"),
        }),
    }
}
