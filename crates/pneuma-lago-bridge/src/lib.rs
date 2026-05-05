//! # pneuma-lago-bridge
//!
//! Minimal journaling bridge for MIL — appends committed directives and
//! execution outcomes to a JSON-lines file. Stand-in for the planned
//! Lago subsystem of the Life Agent OS.
//!
//! ## Why this exists in Tier 2 Week 2
//!
//! From `docs/mil/tier-2-build-plan.md`: the Week-2 deliverable is "real
//! I/O — workspace observer + Praxis bridge + minimal Lago journaling
//! adapter." The journal is what makes undo work across process restarts
//! and what gives Nous something to score outcomes against.
//!
//! ## Architecture
//!
//! - [`JournalWriter`] — append-only NDJSON writer. Buffered; explicit
//!   `flush()` between commits to bound durability windows.
//! - [`JournalReader`] — iterates records in append order.
//! - [`JournalRecord`] — typed enum of record kinds.
//! - [`JournalError`] — closed error enum.
//!
//! ## What this crate is NOT
//!
//! - Not a database. SQLite-backed Lago is v0.3+.
//! - Not durable across host crash. `flush()` syncs to OS but not to
//!   disk; callers that need fsync should wrap.
//! - Not concurrent. One writer per file. The substrate is single-user
//!   in v0.2 so this is fine.
//! - Not a query engine. `JournalReader::iter` is sequential; `grep` is
//!   the v0.2 query plan.

#![doc = include_str!("../README.md")]

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use pneuma_core::{Committed, Directive, DirectiveId};
use pneuma_praxis_bridge::{ExecutionOutcome, ReverseAction};

// --- Error -----------------------------------------------------------------

/// Errors raised by the journal.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum JournalError {
    /// Filesystem error.
    #[error("journal I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error (writing).
    #[error("journal serialize error: {0}")]
    Serialize(serde_json::Error),

    /// Deserialization error (reading).
    #[error("journal deserialize error at line {line}: {error}")]
    Deserialize {
        /// Line number (1-based) where the error occurred.
        line: usize,
        /// The serde_json error message.
        error: String,
    },
}

// --- JournalRecord ---------------------------------------------------------

/// One record in the journal.
///
/// Tagged enum so a replayer can dispatch on `kind` without context.
/// Each variant carries a fresh `record_id` (UUIDv7) and a wall-clock
/// `at` timestamp.
///
/// `clippy::large_enum_variant` is allowed because `Committed` carries a
/// full `Directive<Committed>` which is intentionally large. Boxing it
/// would force every reader to dereference, hurting the common path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
#[allow(clippy::large_enum_variant)]
pub enum JournalRecord {
    /// A directive committed (transitioned to `Directive<Committed>`).
    Committed {
        /// Record identifier.
        record_id: Uuid,
        /// When written.
        at: DateTime<Utc>,
        /// The committed directive — full payload.
        directive: Directive<Committed>,
    },
    /// A Praxis execution succeeded.
    Executed {
        /// Record identifier.
        record_id: Uuid,
        /// When written.
        at: DateTime<Utc>,
        /// Which directive this execution was for.
        directive_id: DirectiveId,
        /// The execution outcome.
        outcome: ExecutionOutcome,
    },
    /// A reverse-action (undo) completed.
    Reversed {
        /// Record identifier.
        record_id: Uuid,
        /// When written.
        at: DateTime<Utc>,
        /// The directive being undone.
        directive_id: DirectiveId,
        /// Which reverse-action ran (echoed for cross-reference).
        reverse_action: ReverseAction,
    },
    /// The user cancelled or rejected before commit / dispatch.
    Cancelled {
        /// Record identifier.
        record_id: Uuid,
        /// When written.
        at: DateTime<Utc>,
        /// Which directive was cancelled.
        directive_id: DirectiveId,
        /// Free-form reason.
        reason: String,
    },
    /// An executor returned an error.
    Failed {
        /// Record identifier.
        record_id: Uuid,
        /// When written.
        at: DateTime<Utc>,
        /// Which directive failed.
        directive_id: DirectiveId,
        /// Error message.
        error: String,
    },
    /// An Arcan (agent-runtime) execution succeeded.
    ///
    /// Distinct variant from [`Self::Executed`] because the two paths
    /// produce structurally different outcomes:
    /// - `Executed` carries an [`ExecutionOutcome`] (Praxis path —
    ///   typed result + reverse action).
    /// - `AgentExecuted` carries the agent's response text + the
    ///   executor label so different harnesses are distinguishable
    ///   when the same intent is replayed across them.
    AgentExecuted {
        /// Record identifier.
        record_id: Uuid,
        /// When written.
        at: DateTime<Utc>,
        /// Which directive this execution was for.
        directive_id: DirectiveId,
        /// Which executor ran the prompt (e.g. `"claude-code"`,
        /// `"codex"`, `"mock"`).
        executor: String,
        /// The agent's response text, captured from subprocess stdout.
        response: String,
        /// Subprocess exit code (0 on success).
        exit_code: i32,
    },
}

impl JournalRecord {
    /// Construct a `Committed` record stamped now.
    #[must_use]
    pub fn committed(directive: Directive<Committed>) -> Self {
        Self::Committed {
            record_id: Uuid::now_v7(),
            at: Utc::now(),
            directive,
        }
    }

    /// Construct an `Executed` record stamped now.
    #[must_use]
    pub fn executed(directive_id: DirectiveId, outcome: ExecutionOutcome) -> Self {
        Self::Executed {
            record_id: Uuid::now_v7(),
            at: Utc::now(),
            directive_id,
            outcome,
        }
    }

    /// Construct a `Reversed` record stamped now.
    #[must_use]
    pub fn reversed(directive_id: DirectiveId, reverse_action: ReverseAction) -> Self {
        Self::Reversed {
            record_id: Uuid::now_v7(),
            at: Utc::now(),
            directive_id,
            reverse_action,
        }
    }

    /// Construct a `Cancelled` record stamped now.
    #[must_use]
    pub fn cancelled(directive_id: DirectiveId, reason: impl Into<String>) -> Self {
        Self::Cancelled {
            record_id: Uuid::now_v7(),
            at: Utc::now(),
            directive_id,
            reason: reason.into(),
        }
    }

    /// Construct a `Failed` record stamped now.
    #[must_use]
    pub fn failed(directive_id: DirectiveId, error: impl Into<String>) -> Self {
        Self::Failed {
            record_id: Uuid::now_v7(),
            at: Utc::now(),
            directive_id,
            error: error.into(),
        }
    }

    /// Construct an `AgentExecuted` record stamped now.
    #[must_use]
    pub fn agent_executed(
        directive_id: DirectiveId,
        executor: impl Into<String>,
        response: impl Into<String>,
        exit_code: i32,
    ) -> Self {
        Self::AgentExecuted {
            record_id: Uuid::now_v7(),
            at: Utc::now(),
            directive_id,
            executor: executor.into(),
            response: response.into(),
            exit_code,
        }
    }

    /// The record's identifier, regardless of variant.
    #[must_use]
    pub fn record_id(&self) -> Uuid {
        match self {
            Self::Committed { record_id, .. }
            | Self::Executed { record_id, .. }
            | Self::Reversed { record_id, .. }
            | Self::Cancelled { record_id, .. }
            | Self::Failed { record_id, .. }
            | Self::AgentExecuted { record_id, .. } => *record_id,
        }
    }

    /// The directive ID this record is about, regardless of variant.
    #[must_use]
    pub fn directive_id(&self) -> DirectiveId {
        match self {
            Self::Committed { directive, .. } => directive.id,
            Self::Executed { directive_id, .. }
            | Self::Reversed { directive_id, .. }
            | Self::Cancelled { directive_id, .. }
            | Self::Failed { directive_id, .. }
            | Self::AgentExecuted { directive_id, .. } => *directive_id,
        }
    }
}

// --- JournalWriter ---------------------------------------------------------

/// Append-only writer for a JSON-lines journal.
///
/// One record per line. Buffered; call [`Self::flush`] to push the
/// buffer to the OS. Drop runs flush via the underlying `BufWriter`.
pub struct JournalWriter {
    path: PathBuf,
    inner: BufWriter<File>,
}

impl JournalWriter {
    /// Open or create the journal file at `path` for append.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, JournalError> {
        let path = path.into();
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            inner: BufWriter::new(file),
        })
    }

    /// Append one record.
    pub fn append(&mut self, record: &JournalRecord) -> Result<(), JournalError> {
        serde_json::to_writer(&mut self.inner, record).map_err(JournalError::Serialize)?;
        self.inner.write_all(b"\n")?;
        Ok(())
    }

    /// Flush the buffer to the OS.
    pub fn flush(&mut self) -> Result<(), JournalError> {
        self.inner.flush()?;
        Ok(())
    }

    /// The path the journal lives at.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

// --- JournalReader ---------------------------------------------------------

/// Read records from a JSON-lines journal.
pub struct JournalReader {
    path: PathBuf,
}

impl JournalReader {
    /// Open the journal at `path`. Does not read until [`Self::iter`].
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Iterate records in append order. Each item is a `Result` so a
    /// malformed line errors that line without aborting iteration.
    ///
    /// The outer `Result` reports the open / read failure, since iter
    /// can't itself express "could not start iterating".
    /// `clippy::iter_not_returning_iterator` is allowed for that reason.
    #[allow(clippy::iter_not_returning_iterator)]
    pub fn iter(
        &self,
    ) -> Result<impl Iterator<Item = Result<JournalRecord, JournalError>>, JournalError> {
        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        Ok(reader.lines().enumerate().filter_map(|(idx, line)| {
            let raw = match line {
                Ok(s) if s.trim().is_empty() => return None,
                Ok(s) => s,
                Err(e) => return Some(Err(JournalError::Io(e))),
            };
            Some(serde_json::from_str::<JournalRecord>(&raw).map_err(|e| {
                JournalError::Deserialize {
                    line: idx + 1,
                    error: e.to_string(),
                }
            }))
        }))
    }
}
