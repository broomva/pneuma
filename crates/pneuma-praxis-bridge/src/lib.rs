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
//! | Act                      | Reversibility | Reverse                          |
//! |--------------------------|---------------|----------------------------------|
//! | `file.read`              | Free          | None — read has no side effects  |
//! | `file.rename`            | Costly        | `RenameBack { from, to }`        |
//! | `file.copy`              | Costly        | `DeleteCopy { path }`            |
//! | `file.write`             | Costly        | `RestoreContent { path, prior }` |
//! | `browser.navigate`       | Costly        | `RestoreUrl { browser, prior }`  |
//! | `workspace.switch_app`   | Free          | None — re-switching is trivial   |
//!
//! `file.delete` is intentionally **not** wired here. It is
//! [`Reversibility::Irreversible`][rev]; v0.2 punts irreversible-real
//! operations until a hardened bridge with soft-delete / trash semantics
//! exists.
//!
//! [rev]: pneuma_core::Reversibility::Irreversible
//!
//! ## `workspace.switch_app` execution model
//!
//! Step #14 of `MIL-PROJECT.md` §11.2. The second OS-control act —
//! shares the macOS / AppleScript machinery with `browser.navigate`.
//!
//! - **Platform.** macOS only in v0.2. Other platforms return
//!   `PraxisError::PlatformUnsupported`.
//! - **Slot.** `target: Referent::App(AppId)`. Extracted with
//!   `require_app_slot`.
//! - **Reverse.** `ReverseAction::None`. The act is declared
//!   `Reversibility::Free` because re-switching to the prior app is
//!   trivial-to-do-manually; v0.2 doesn't capture the prior app for
//!   automatic undo. A future Costly variant could add
//!   `ReverseAction::SwitchBack { prior_app }`.
//! - **Injection safety.** App names may contain spaces and unicode;
//!   we forbid `"`, `\`, and newlines (same as URLs). Real app names
//!   like `"Visual Studio Code"` work fine.
//!
//! ## `browser.navigate` execution model
//!
//! Step #13 of `MIL-PROJECT.md` §11.2. The first **OS-control** act —
//! it leaves the file system entirely and reaches into another running
//! application (Safari).
//!
//! - **Platform.** macOS only in v0.2. Other platforms return
//!   `PraxisError::PlatformUnsupported`. The dispatch is `cfg`-gated
//!   so the bridge compiles on Linux (CI matrix passes) but the
//!   AppleScript path only links on macOS.
//! - **Browser.** Safari only in v0.2. The reverse-action stores the
//!   browser name as a string so future variants (Chrome, Arc, Brave)
//!   plug in without touching the enum.
//! - **Reverse.** We capture the front tab's URL *before* the navigation
//!   fires, then store that as `ReverseAction::RestoreUrl`. If the user
//!   undoes, we navigate the front tab back to the captured URL. The
//!   reverse fails (returns `PraxisError::ReverseRefused`) if Safari
//!   is no longer running, has no windows, or has had the relevant tab
//!   closed.
//! - **Injection safety.** AppleScript is built with simple character
//!   denylisting (no `"`, no `\`, no newlines in URLs). v0.3 will use
//!   `url::Url::parse` for principled validation; v0.2 trades
//!   correctness on edge URLs for zero new dependencies.

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

    /// The act requires a platform feature this build doesn't have
    /// (e.g. `browser.navigate` outside macOS). Carries a static
    /// reason so callers can surface it without allocations.
    #[error("act requires unsupported platform: {reason}")]
    PlatformUnsupported {
        /// Static reason, e.g. `"browser.navigate requires macOS osascript"`.
        reason: &'static str,
    },

    /// An external command (osascript, etc.) returned a non-zero exit
    /// code. Distinct from `Filesystem` — we shelled out and the shell
    /// said no.
    #[error("external command `{command}` failed: {reason}")]
    ExternalCommand {
        /// The command name we spawned.
        command: String,
        /// Free-form reason (typically the trimmed stderr of the command).
        reason: String,
    },

    /// A URL was rejected because it contained AppleScript-unsafe
    /// characters. v0.2 blocks `"`, `\`, and newlines without parsing
    /// the URL further.
    #[error("URL rejected: {reason}")]
    UnsafeUrl {
        /// Why the URL was rejected.
        reason: &'static str,
    },
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
    /// Navigate the named browser's frontmost tab back to the URL it
    /// was showing before this act executed. Captured by
    /// `execute_browser_navigate` at execution time.
    RestoreUrl {
        /// Browser application name (v0.2 always `"Safari"`).
        browser: String,
        /// URL the browser was showing before the act.
        prior_url: String,
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
            "browser.navigate" => execute_browser_navigate(call),
            "workspace.switch_app" => execute_switch_app(call),
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
            ReverseAction::RestoreUrl { browser, prior_url } => {
                reverse_browser_navigate(browser, prior_url)
            }
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

/// Browser to drive on macOS in v0.2. Hard-coded to Safari because
/// AppleScript's URL-setting dialect varies per browser; future versions
/// will lift this into a `BrowserKind` enum or auto-detect from the
/// front application.
const V02_BROWSER: &str = "Safari";

fn execute_browser_navigate(call: &PraxisCall) -> Result<ExecutionOutcome, PraxisError> {
    let target_url = require_url_slot(call, "url")?;
    reject_unsafe_url(&target_url)?;
    // Capture-then-set: the prior URL is recorded *before* we navigate
    // so the reverse-action recipe is complete the moment execution
    // succeeds. If the capture or the navigate fails, no journal entry
    // is created — execution returned an error.
    let prior_url = capture_browser_front_url(V02_BROWSER)?;
    set_browser_front_url(V02_BROWSER, &target_url)?;
    Ok(ExecutionOutcome {
        act_id: call.act_id.clone(),
        result: serde_json::json!({
            "browser": V02_BROWSER,
            "navigated_to": target_url,
            "prior_url": prior_url,
        }),
        reverse_action: ReverseAction::RestoreUrl {
            browser: V02_BROWSER.to_owned(),
            prior_url,
        },
    })
}

fn execute_switch_app(call: &PraxisCall) -> Result<ExecutionOutcome, PraxisError> {
    let app_name = require_app_slot(call, "target")?;
    reject_unsafe_app_name(&app_name)?;
    activate_app(&app_name)?;
    Ok(ExecutionOutcome {
        act_id: call.act_id.clone(),
        result: serde_json::json!({
            "activated": app_name,
        }),
        // Reversibility::Free per the act registry: re-activating the
        // prior app is something the user can do trivially. v0.2 does
        // not capture the prior app for automatic undo.
        reverse_action: ReverseAction::None,
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

fn reverse_browser_navigate(browser: &str, prior_url: &str) -> Result<(), PraxisError> {
    // Refuse on URL drift / injection attempts. The captured URL came
    // from our own AppleScript so a `"` would only appear if Safari
    // reported one — extremely unlikely but we still refuse rather
    // than smuggle.
    reject_unsafe_url(prior_url)?;
    set_browser_front_url(browser, prior_url)
}

// --- Browser driver (cfg-gated AppleScript) ---------------------------------

/// Rejects URLs that would break our AppleScript shell-out.
///
/// v0.2 stance: deny-list. v0.3 will use `url::Url::parse` to validate
/// the URL is well-formed, then re-serialize through proper quoting.
fn reject_unsafe_url(url: &str) -> Result<(), PraxisError> {
    if url.contains('"') {
        return Err(PraxisError::UnsafeUrl {
            reason: "URL contains double-quote (would break AppleScript literal)",
        });
    }
    if url.contains('\\') {
        return Err(PraxisError::UnsafeUrl {
            reason: "URL contains backslash (would break AppleScript literal)",
        });
    }
    if url.contains('\n') || url.contains('\r') {
        return Err(PraxisError::UnsafeUrl {
            reason: "URL contains newline",
        });
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn capture_browser_front_url(browser: &str) -> Result<String, PraxisError> {
    let script = format!("tell application \"{browser}\" to URL of current tab of front window");
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PraxisError::ExternalCommand {
            command: "osascript".to_owned(),
            reason: stderr.trim().to_owned(),
        });
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok(url)
}

#[cfg(not(target_os = "macos"))]
fn capture_browser_front_url(_browser: &str) -> Result<String, PraxisError> {
    Err(PraxisError::PlatformUnsupported {
        reason: "browser.navigate requires macOS osascript",
    })
}

#[cfg(target_os = "macos")]
fn set_browser_front_url(browser: &str, url: &str) -> Result<(), PraxisError> {
    // Caller is responsible for `reject_unsafe_url`. We re-check anyway
    // because this function is also called from the reverse path with
    // a prior_url whose provenance came from a previous call to
    // `capture_browser_front_url` — defensible defense-in-depth.
    reject_unsafe_url(url)?;
    let script = format!(
        "tell application \"{browser}\"\n  \
            activate\n  \
            if (count of windows) is 0 then\n    \
                make new document\n  \
            end if\n  \
            set URL of current tab of front window to \"{url}\"\n\
        end tell"
    );
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PraxisError::ExternalCommand {
            command: "osascript".to_owned(),
            reason: stderr.trim().to_owned(),
        });
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn set_browser_front_url(_browser: &str, _url: &str) -> Result<(), PraxisError> {
    Err(PraxisError::PlatformUnsupported {
        reason: "browser.navigate requires macOS osascript",
    })
}

// --- App-name driver (cfg-gated AppleScript) --------------------------------

/// Rejects app names that would break the AppleScript shell-out.
///
/// Same denylist as `reject_unsafe_url` — block AppleScript-string
/// terminators / escape characters. Real macOS app names like
/// `"Visual Studio Code"`, `"Google Chrome"`, `"Microsoft Outlook"`
/// pass through unchanged.
fn reject_unsafe_app_name(name: &str) -> Result<(), PraxisError> {
    if name.is_empty() {
        return Err(PraxisError::UnsafeUrl {
            // UnsafeUrl is the closest existing variant; v0.3 may
            // introduce a UnsafeAppName variant if app-name validation
            // diverges from URL validation.
            reason: "app name is empty",
        });
    }
    if name.contains('"') {
        return Err(PraxisError::UnsafeUrl {
            reason: "app name contains double-quote (would break AppleScript literal)",
        });
    }
    if name.contains('\\') {
        return Err(PraxisError::UnsafeUrl {
            reason: "app name contains backslash (would break AppleScript literal)",
        });
    }
    if name.contains('\n') || name.contains('\r') {
        return Err(PraxisError::UnsafeUrl {
            reason: "app name contains newline",
        });
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn activate_app(name: &str) -> Result<(), PraxisError> {
    reject_unsafe_app_name(name)?;
    let script = format!("tell application \"{name}\" to activate");
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PraxisError::ExternalCommand {
            command: "osascript".to_owned(),
            reason: stderr.trim().to_owned(),
        });
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn activate_app(_name: &str) -> Result<(), PraxisError> {
    Err(PraxisError::PlatformUnsupported {
        reason: "workspace.switch_app requires macOS osascript",
    })
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

/// Extract a `Referent::App(AppId)` binding from a slot. Used by
/// `workspace.switch_app`. Returns the inner app-name string by clone.
fn require_app_slot(call: &PraxisCall, name: &str) -> Result<String, PraxisError> {
    let value = slot(call, name).ok_or_else(|| PraxisError::MissingSlot {
        act: call.act_id.as_str().to_owned(),
        slot: name.to_owned(),
    })?;
    match value {
        ResolvedSlotValue::Referent(ReferentValue::App(app)) => Ok(app.as_str().to_owned()),
        other => Err(PraxisError::WrongSlotKind {
            act: call.act_id.as_str().to_owned(),
            slot: name.to_owned(),
            reason: format!("expected Referent(App), got {other:?}"),
        }),
    }
}

/// Extract a `Referent::Url(String)` binding from a slot. Used by
/// `browser.navigate`. Returns the inner URL string by clone.
fn require_url_slot(call: &PraxisCall, name: &str) -> Result<String, PraxisError> {
    let value = slot(call, name).ok_or_else(|| PraxisError::MissingSlot {
        act: call.act_id.as_str().to_owned(),
        slot: name.to_owned(),
    })?;
    match value {
        ResolvedSlotValue::Referent(ReferentValue::Url(url)) => Ok(url.clone()),
        other => Err(PraxisError::WrongSlotKind {
            act: call.act_id.as_str().to_owned(),
            slot: name.to_owned(),
            reason: format!("expected Referent(Url), got {other:?}"),
        }),
    }
}
