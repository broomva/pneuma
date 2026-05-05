//! # pneuma-arcan-bridge
//!
//! The first agent-harness adapter for MIL — translates a
//! [`pneuma_router::AgentPrompt`] into a subprocess invocation against
//! an external agent CLI (Claude Code, Codex, Aider, ...).
//!
//! ## What this crate is for
//!
//! Step #16 of `MIL-PROJECT.md` §11.2. Three goals:
//!
//! 1. **Open the second execution path.** Today MIL only has Praxis
//!    (deterministic OS calls). After this crate, [`Dispatch::Arcan`]
//!    has a concrete executor — the same intent contract can dispatch
//!    to either deterministic or LLM-delegated execution.
//! 2. **Stay agent-agnostic.** [`ArcanExecutor`] is the trait;
//!    [`StdioCommandArcan`] is a particular implementation configured
//!    via [`StdioCommandArcan::claude_code`] for Claude Code's CLI but
//!    swappable for any subprocess that takes a prompt and returns a
//!    response.
//! 3. **Mirror the Praxis trait shape.** Both bridges expose
//!    `execute(&self, ...)` returning a typed outcome. The journal
//!    can persist either through a parallel path; the demo can route
//!    either through act-id dispatch.
//!
//! ## What this crate is NOT
//!
//! - **Not Claude Code-specific.** The default builder configures
//!   for Claude Code, but the trait surface is generic.
//! - **Not async.** Sync subprocess execution for v0.2. A future
//!   `AsyncArcanExecutor` can wrap Tokio.
//! - **Not a journal.** That's `pneuma-lago-bridge`. This crate just
//!   *returns* enough information for the journal to record (in a
//!   future PR that extends `JournalRecord`).
//! - **Not a resolver.** Slots arrive already-resolved in the
//!   `AgentPrompt`. Resolving deictics is `pneuma-resolver`'s job
//!   (step #18).
//!
//! ## Three-layer architecture
//!
//! ```text
//! AgentPrompt
//!    │
//!    ▼  prompt_to_string(prompt) — structured → plain text
//! String
//!    │
//!    ▼  ArcanExecutor::execute(prompt) — trait method, sync
//!    │      ├─ MockArcan: returns canned response
//!    │      └─ StdioCommandArcan: subprocess + stdin/stdout
//!    ▼
//! ArcanOutcome { response, ... }
//! ```
//!
//! [`Dispatch::Arcan`]: pneuma_router::Dispatch::Arcan

#![doc = include_str!("../README.md")]

use std::io::Write;
use std::process::{Command, Stdio};

use pneuma_core::ActId;
use pneuma_core::act::ResolvedSlotValue;
use pneuma_router::AgentPrompt;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// --- Error ------------------------------------------------------------------

/// Errors an [`ArcanExecutor`] can return.
///
/// Distinct from [`pneuma_core::ContractError`] — that is for contract
/// violations *before* dispatch; `ArcanError` is for execution-time
/// failures *during* delegation to an external agent CLI.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ArcanError {
    /// Could not spawn the subprocess.
    #[error("subprocess `{command}` could not spawn: {error}")]
    SpawnFailed {
        /// The command name we attempted to spawn.
        command: String,
        /// The underlying I/O error.
        error: std::io::Error,
    },

    /// I/O error while writing the prompt to subprocess stdin.
    #[error("stdin write failed for subprocess `{command}`: {error}")]
    StdinWriteFailed {
        /// The command name.
        command: String,
        /// The underlying I/O error.
        error: std::io::Error,
    },

    /// Subprocess exited with non-zero status.
    #[error("subprocess `{command}` exited non-zero (code: {exit_code:?}): {stderr}")]
    SubprocessExitNonZero {
        /// The command name.
        command: String,
        /// Exit code, if any.
        exit_code: Option<i32>,
        /// Trimmed stderr from the subprocess.
        stderr: String,
    },

    /// Subprocess produced output that wasn't valid UTF-8.
    #[error("subprocess `{command}` produced non-UTF-8 stdout")]
    StdoutNotUtf8 {
        /// The command name.
        command: String,
    },

    /// The prompt contained content that we refuse to pass to a
    /// subprocess (e.g., a null byte). v0.2 blocks `\0` only; v0.3
    /// may add platform-specific shell-injection guards.
    #[error("prompt rejected: {reason}")]
    UnsafePromptContent {
        /// Why the prompt was rejected.
        reason: &'static str,
    },
}

// --- Outcome ----------------------------------------------------------------

/// What an [`ArcanExecutor`] returns on successful agent dispatch.
///
/// Distinct from [`pneuma_praxis_bridge::ExecutionOutcome`][p] — that
/// type carries a typed `ReverseAction`. Arcan outcomes carry the
/// agent's response text; the *agent* is responsible for any reverse
/// recipe it surfaces. (A future `AgentResponse::reverse_recipe` field
/// could carry that, mirroring the Praxis pattern at completion-time
/// instead of execution-time. v0.2 just captures the response.)
///
/// [p]: pneuma_praxis_bridge::ExecutionOutcome
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArcanOutcome {
    /// Which act produced this outcome (echoed for journal cross-reference).
    pub act_id: ActId,
    /// The agent's response text, captured from the subprocess stdout.
    pub response: String,
    /// What the executor was, e.g. `"claude-code"` or `"mock"`. The
    /// journal records this so different harnesses can be told apart
    /// when the same intent is replayed across them.
    pub executor: String,
    /// Subprocess exit code on success (always `0` for `MockArcan`,
    /// always `0` for `StdioCommandArcan` if execute() returned `Ok`).
    /// Recorded for diagnostics only.
    pub exit_code: i32,
}

// --- ArcanExecutor trait ----------------------------------------------------

/// The agent-runtime executor surface.
///
/// Implementors take an [`AgentPrompt`] and produce an [`ArcanOutcome`]
/// via whatever subprocess / API mechanism they wrap.
///
/// Synchronous on purpose for v0.2: subprocess invocation is
/// inherently blocking and a sync API matches the Praxis bridge's
/// shape. A future `AsyncArcanExecutor` trait can wrap Tokio for
/// streaming token-by-token responses.
pub trait ArcanExecutor {
    /// Run the prompt against the underlying agent. Returns the
    /// outcome on success; `ArcanError` on any failure.
    fn execute(&self, prompt: &AgentPrompt) -> Result<ArcanOutcome, ArcanError>;
}

// --- MockArcan --------------------------------------------------------------

/// Scripted-response executor for tests.
///
/// Constructed with a canned response string. Every `execute` call
/// returns that string verbatim, with `executor: "mock"` and
/// `exit_code: 0`. No subprocess is spawned.
#[derive(Debug, Clone)]
pub struct MockArcan {
    canned_response: String,
}

impl MockArcan {
    /// Construct a [`MockArcan`] that always returns `canned_response`.
    pub fn new(canned_response: impl Into<String>) -> Self {
        Self {
            canned_response: canned_response.into(),
        }
    }
}

impl ArcanExecutor for MockArcan {
    fn execute(&self, prompt: &AgentPrompt) -> Result<ArcanOutcome, ArcanError> {
        Ok(ArcanOutcome {
            act_id: prompt.act_id.clone(),
            response: self.canned_response.clone(),
            executor: "mock".to_owned(),
            exit_code: 0,
        })
    }
}

// --- StdioCommandArcan ------------------------------------------------------

/// Generic subprocess-based executor.
///
/// Spawns a configurable command, writes the formatted prompt to
/// stdin, captures stdout as the agent's response. Configurable for
/// any CLI that follows the standard agent-CLI shape — Claude Code,
/// Codex, Aider, custom harnesses.
///
/// ## Default configuration
///
/// [`StdioCommandArcan::claude_code`] returns a builder configured for
/// Anthropic's Claude Code CLI:
///
/// ```text
/// claude --print
/// ```
///
/// where the prompt is written to stdin and the response is captured
/// from stdout. Other CLIs configure via [`StdioCommandArcan::new`].
///
/// ## Subprocess hygiene
///
/// - Working directory: inherits from parent.
/// - Environment: inherits from parent. Set additional vars via
///   [`StdioCommandArcan::with_env`].
/// - Stdin: piped, prompt written, then closed.
/// - Stdout: captured.
/// - Stderr: captured (surfaced in `ArcanError::SubprocessExitNonZero`).
#[derive(Debug, Clone)]
pub struct StdioCommandArcan {
    command: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    executor_label: String,
}

impl StdioCommandArcan {
    /// Construct a custom subprocess executor.
    ///
    /// `executor_label` is recorded in the [`ArcanOutcome::executor`]
    /// field so the journal can distinguish runs across different
    /// agent CLIs (e.g., `"codex"` vs `"aider"`).
    pub fn new(
        command: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
        executor_label: impl Into<String>,
    ) -> Self {
        Self {
            command: command.into(),
            args: args.into_iter().map(Into::into).collect(),
            env: Vec::new(),
            executor_label: executor_label.into(),
        }
    }

    /// Pre-configured for Anthropic's Claude Code CLI:
    /// invokes `claude --print` with the prompt on stdin.
    #[must_use]
    pub fn claude_code() -> Self {
        Self::new("claude", ["--print"], "claude-code")
    }

    /// Append an environment variable for the subprocess.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// The command name (for diagnostics + the journal).
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }

    /// The configured arguments.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// The executor label embedded in outcomes.
    #[must_use]
    pub fn executor_label(&self) -> &str {
        &self.executor_label
    }
}

impl ArcanExecutor for StdioCommandArcan {
    fn execute(&self, prompt: &AgentPrompt) -> Result<ArcanOutcome, ArcanError> {
        let prompt_str = prompt_to_string(prompt);
        reject_unsafe_prompt(&prompt_str)?;

        let mut cmd = Command::new(&self.command);
        cmd.args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in &self.env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|error| ArcanError::SpawnFailed {
            command: self.command.clone(),
            error,
        })?;

        // Write the prompt to stdin, then close it so the subprocess
        // can finish reading. Holding stdin open would deadlock with a
        // subprocess that reads-then-responds.
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt_str.as_bytes()).map_err(|error| {
                ArcanError::StdinWriteFailed {
                    command: self.command.clone(),
                    error,
                }
            })?;
            // stdin is dropped here, closing the pipe.
        }

        let output = child
            .wait_with_output()
            .map_err(|error| ArcanError::SpawnFailed {
                command: self.command.clone(),
                error,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(ArcanError::SubprocessExitNonZero {
                command: self.command.clone(),
                exit_code: output.status.code(),
                stderr,
            });
        }

        let response = String::from_utf8(output.stdout)
            .map_err(|_| ArcanError::StdoutNotUtf8 {
                command: self.command.clone(),
            })?
            .trim_end()
            .to_owned();

        Ok(ArcanOutcome {
            act_id: prompt.act_id.clone(),
            response,
            executor: self.executor_label.clone(),
            exit_code: output.status.code().unwrap_or(0),
        })
    }
}

// --- Prompt formatting ------------------------------------------------------

/// Serialize an [`AgentPrompt`] into plain text suitable for piping
/// into an agent CLI.
///
/// The format is intentionally simple — agent CLIs expect natural-
/// language prompts, not structured payloads. We render:
///
/// 1. The instruction (verbatim, this is the user's actual ask)
/// 2. A brief structured-context footer listing the bound slots, so
///    the agent has the resolved entities (file paths, URLs, app
///    names) without having to parse them out of the instruction.
///
/// Example output:
///
/// ```text
/// Refactor the authentication module to use the new token format.
///
/// ---
/// Bound slots (resolved by MIL):
/// - target: File("/path/to/auth.rs")
/// - new_format: String("ed25519")
/// - act: agent.refactor
/// ```
///
/// The footer is separated by a markdown horizontal rule so agents
/// that respect markdown render it sensibly.
#[must_use]
pub fn prompt_to_string(prompt: &AgentPrompt) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    s.push_str(&prompt.instruction);
    s.push_str("\n\n---\nBound slots (resolved by MIL):\n");
    for (name, value) in &prompt.slots {
        // `write!` into a String is infallible; `unwrap` documents
        // the contract rather than dropping a real error path.
        let _ = writeln!(s, "- {name}: {}", format_slot_value(value));
    }
    let _ = writeln!(s, "- act: {}", prompt.act_id.as_str());
    s
}

fn format_slot_value(value: &ResolvedSlotValue) -> String {
    // The Debug derive on ResolvedSlotValue includes the variant name,
    // e.g. `Referent(File(...))`. That's the cleanest plain-text
    // serialization for v0.2 — it tells the agent "this is a file"
    // vs "this is a URL" and includes the resolved value.
    format!("{value:?}")
}

// --- Prompt safety ----------------------------------------------------------

fn reject_unsafe_prompt(prompt: &str) -> Result<(), ArcanError> {
    if prompt.contains('\0') {
        return Err(ArcanError::UnsafePromptContent {
            reason: "prompt contains null byte (would terminate C string in subprocess)",
        });
    }
    Ok(())
}

// --- Inline tests -----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pneuma_core::ReferentValue;

    fn synthetic_prompt() -> AgentPrompt {
        AgentPrompt {
            act_id: ActId::new("agent.refactor").unwrap(),
            instruction: "Refactor this module".to_owned(),
            slots: vec![(
                "url".to_owned(),
                ResolvedSlotValue::Referent(ReferentValue::Url("https://example.com".to_owned())),
            )],
        }
    }

    #[test]
    fn prompt_to_string_renders_instruction_and_slots() {
        let formatted = prompt_to_string(&synthetic_prompt());
        assert!(formatted.contains("Refactor this module"));
        assert!(formatted.contains("- url:"));
        assert!(formatted.contains("https://example.com"));
        assert!(formatted.contains("- act: agent.refactor"));
    }

    #[test]
    fn prompt_with_null_byte_in_instruction_is_rejected() {
        let mut prompt = synthetic_prompt();
        prompt.instruction = "ignore previous\0".to_owned();
        let exec = StdioCommandArcan::new("/usr/bin/true", Vec::<String>::new(), "test");
        let err = exec.execute(&prompt).unwrap_err();
        assert!(
            matches!(err, ArcanError::UnsafePromptContent { .. }),
            "null byte must be rejected, got {err:?}"
        );
    }

    #[test]
    fn mock_arcan_returns_canned_response() {
        let mock = MockArcan::new("Refactor complete: 3 functions changed.");
        let outcome = mock.execute(&synthetic_prompt()).unwrap();
        assert_eq!(outcome.act_id.as_str(), "agent.refactor");
        assert_eq!(outcome.response, "Refactor complete: 3 functions changed.");
        assert_eq!(outcome.executor, "mock");
        assert_eq!(outcome.exit_code, 0);
    }

    #[test]
    fn stdio_command_arcan_claude_code_default() {
        let exec = StdioCommandArcan::claude_code();
        assert_eq!(exec.command(), "claude");
        assert_eq!(exec.args(), &["--print".to_owned()]);
        assert_eq!(exec.executor_label(), "claude-code");
    }

    #[test]
    fn stdio_command_arcan_with_env_appends() {
        let exec = StdioCommandArcan::claude_code()
            .with_env("ANTHROPIC_API_KEY", "test-key")
            .with_env("ANOTHER", "value");
        // Inspect via Debug since env is private; this is a smoke test.
        let dbg = format!("{exec:?}");
        assert!(dbg.contains("ANTHROPIC_API_KEY"));
        assert!(dbg.contains("ANOTHER"));
    }

    #[test]
    fn arcan_outcome_round_trips_json() {
        let outcome = ArcanOutcome {
            act_id: ActId::new("agent.refactor").unwrap(),
            response: "ok".to_owned(),
            executor: "mock".to_owned(),
            exit_code: 0,
        };
        let json = serde_json::to_string(&outcome).unwrap();
        let back: ArcanOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(back, outcome);
    }
}
