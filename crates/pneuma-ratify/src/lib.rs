//! # pneuma-ratify
//!
//! Approval-channel FSM for MIL. Maps user input (a single character
//! or hotkey) to typed [`ApprovalDecision`]. Pluggable via the
//! [`Ratifier`] trait so tests drive the loop without I/O.
//!
//! ## Hotkey vocabulary (v0.2)
//!
//! | Key       | Decision    |
//! |-----------|-------------|
//! | `Enter`   | `Commit`    |
//! | `Esc`     | `Cancel`    |
//! | `y` / `Y` | `Approve`   |
//! | `n` / `N` | `Reject`    |
//! | `u` / `U` | `Undo`      |
//! | `a` / `A` | `Amend` (returns `Cancel` for v0.2 — caller routes back to compose) |
//! | `e` / `E` | `Engage`    |
//! | `?`       | `Clarify`   |
//!
//! Anything else maps to [`ApprovalDecision::Continue`] (no decision yet).

#![doc = include_str!("../README.md")]

use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};

// --- ApprovalDecision ------------------------------------------------------

/// Typed approval-channel decision.
///
/// From `MIL-PROJECT.md` §14: "Engage / commit / cancel / approve /
/// reject / undo — the six approval-channel discourse moves;
/// safety-critical, binary."
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ApprovalDecision {
    /// "I am about to start a directive." (Held pinch / hotkey-press.)
    Engage,
    /// "Commit the directive currently composing." (Pinch-release / Enter.)
    Commit,
    /// "Cancel the directive currently composing." (Open palm / Esc.)
    Cancel,
    /// "Approve the proposed directive." (Pinch / Enter on a Proposal.)
    Approve,
    /// "Reject the proposed directive." (Open palm / Esc on a Proposal.)
    Reject,
    /// "Undo the most recent committed directive." (Flick / Cmd-Z.)
    Undo,
    /// "Help me — let me ask a question about the proposal."
    /// Carries the user's free-form clarification text.
    Clarify(String),
    /// No decision yet; keep listening.
    Continue,
}

impl ApprovalDecision {
    /// `true` if this decision terminates the current waiting loop
    /// (i.e. anything other than `Continue`).
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Continue)
    }

    /// Parse a single character to a decision. Used by stdin and test
    /// ratifiers.
    ///
    /// Mapping is documented at the crate root.
    ///
    /// `clippy::match_same_arms` is allowed: `'a'`/`'A'` (amend) and
    /// `'\x1b'`/`'q'`/`'Q'` (cancel) all collapse to `Cancel` *for now*
    /// (v0.2 hotkey FSM), but they're conceptually distinct and v0.3
    /// will distinguish (Amend → recompose, Cancel → drop).
    #[must_use]
    #[allow(clippy::match_same_arms)]
    pub fn from_char(c: char) -> Self {
        match c {
            '\n' | '\r' => Self::Commit,
            // Esc is `\x1b`; in line-mode stdin we usually never see it,
            // so we accept 'q' / 'Q' as an additional cancel hotkey.
            '\x1b' | 'q' | 'Q' => Self::Cancel,
            'y' | 'Y' => Self::Approve,
            'n' | 'N' => Self::Reject,
            'u' | 'U' => Self::Undo,
            'a' | 'A' => Self::Cancel, // amend → cancel + recompose
            'e' | 'E' => Self::Engage,
            '?' => Self::Clarify(String::new()),
            _ => Self::Continue,
        }
    }
}

// --- Ratifier trait --------------------------------------------------------

/// Read one approval decision from the user's input channel.
///
/// Implementors define how decisions arrive — stdin, test queue,
/// remote channel, gesture stream (v0.3).
pub trait Ratifier {
    /// Read the next decision. Blocks until one is available; returns
    /// [`ApprovalDecision::Cancel`] on EOF / channel closed so the
    /// caller doesn't hang.
    fn read_decision(&mut self) -> ApprovalDecision;
}

// --- StdinRatifier ---------------------------------------------------------

/// Stdin-line-based ratifier. Reads a line, takes the first non-empty
/// character, parses to a decision via [`ApprovalDecision::from_char`].
///
/// `?` followed by a space-separated clarification produces
/// `Clarify(rest_of_line)`.
pub struct StdinRatifier {
    /// Optional prompt printed before each `read_decision`. Set to
    /// empty string for silent.
    pub prompt: String,
}

impl Default for StdinRatifier {
    fn default() -> Self {
        Self {
            prompt: "[Enter=commit · Esc/q=cancel · y/n · u=undo · ?=clarify] > ".to_owned(),
        }
    }
}

impl Ratifier for StdinRatifier {
    fn read_decision(&mut self) -> ApprovalDecision {
        if !self.prompt.is_empty() {
            print!("{}", self.prompt);
            let _ = io::stdout().flush();
        }
        let mut line = String::new();
        let stdin = io::stdin();
        let mut handle = stdin.lock();
        match handle.read_line(&mut line) {
            // EOF and read errors both surface as Cancel so the caller's
            // loop terminates cleanly. (clippy::match_same_arms allowed
            // here for the same reason as `from_char`: distinct cases,
            // shared mapping in v0.2, v0.3 may diverge.)
            #[allow(clippy::match_same_arms)]
            Ok(0) => ApprovalDecision::Cancel,
            Ok(_) => parse_line(&line),
            Err(_) => ApprovalDecision::Cancel,
        }
    }
}

/// Parse a stdin line into a decision. Exposed for tests.
#[must_use]
pub fn parse_line(line: &str) -> ApprovalDecision {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        // Blank line == Enter == Commit.
        return ApprovalDecision::Commit;
    }
    let first = trimmed.chars().next().expect("trimmed non-empty");
    if first == '?' {
        let rest = trimmed[1..].trim().to_owned();
        return ApprovalDecision::Clarify(rest);
    }
    ApprovalDecision::from_char(first)
}

// --- MockRatifier (for tests) ----------------------------------------------

/// Test ratifier. Drives the FSM from a pre-built queue of decisions.
///
/// When the queue empties, returns [`ApprovalDecision::Cancel`] so
/// the caller doesn't loop forever in tests.
#[derive(Debug, Clone, Default)]
pub struct MockRatifier {
    queue: Vec<ApprovalDecision>,
}

impl MockRatifier {
    /// Construct from a sequence of decisions. The first item in the
    /// vector is returned first.
    #[must_use]
    pub fn from_decisions(mut decisions: Vec<ApprovalDecision>) -> Self {
        decisions.reverse();
        Self { queue: decisions }
    }

    /// Construct from a string of single-character keys, mapping each
    /// via [`ApprovalDecision::from_char`].
    #[must_use]
    pub fn from_keystream(keys: &str) -> Self {
        let decisions = keys.chars().map(ApprovalDecision::from_char).collect();
        Self::from_decisions(decisions)
    }

    /// Push a decision to the back of the queue.
    pub fn push(&mut self, d: ApprovalDecision) {
        self.queue.insert(0, d);
    }

    /// `true` if no decisions remain.
    #[must_use]
    pub fn is_drained(&self) -> bool {
        self.queue.is_empty()
    }
}

impl Ratifier for MockRatifier {
    fn read_decision(&mut self) -> ApprovalDecision {
        self.queue.pop().unwrap_or(ApprovalDecision::Cancel)
    }
}
