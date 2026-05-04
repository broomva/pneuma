//! Tiny deterministic utterance parser.
//!
//! From `MIL-PROJECT.md` §11.3 (Phase 2):
//!
//! > `pneuma-act-registry`: deterministic act lookup for common acts.
//!
//! This module is the v0.2 minimum: a regex-free pattern matcher
//! that handles a few common phrasings for `file.rename`, `file.copy`,
//! `file.delete`, `file.open`, `file.read`. It does *not* call an
//! LLM; for unmatched utterances it returns `ParseError::NoMatch`,
//! which is the architectural answer (clarify rather than guess) per
//! spec §8.4.
//!
//! ## Patterns supported
//!
//! - `"rename to NEW"` / `"rename it to NEW"` / `"rn NEW"`
//! - `"copy to PATH"` / `"copy it to PATH"`
//! - `"delete"` / `"delete it"` / `"rm"` / `"remove"`
//! - `"open"` / `"open it"`
//! - `"read"` / `"show"`
//! - `"navigate to URL"` / `"go to URL"` / `"browse URL"` / `"go URL"`
//!
//! The verb is looked up via [`pneuma_acts::ActRegistry::lookup_by_verb`];
//! arguments are extracted by simple word splitting around `to`. The
//! parser is intentionally tiny — production parsing is a v0.3 LLM
//! concern.
//!
//! ## What this is NOT
//!
//! - Not a grammar. No multi-step parsing, no AST. Just enough to
//!   bind common slots from common phrasings.
//! - Not the production parser. v0.3 will use an LLM with constrained
//!   decoding into the directive schema.
//! - Not aware of the focused file. The caller (the demo) supplies
//!   the target slot from the workspace context; the parser only
//!   extracts payload slots like `new_name`.

use thiserror::Error;

use pneuma_acts::ActRegistry;
use pneuma_core::{Act, ActId};

/// Result of a successful utterance parse.
///
/// Carries the resolved act plus any payload-slot bindings the
/// parser was able to extract. The caller (typically `pneuma-demo`)
/// fills in the contextual slots (`target` from the focused file,
/// `destination` from somewhere else) before finalizing the
/// directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUtterance {
    /// The resolved act id.
    pub act_id: ActId,
    /// Extracted payload-slot bindings as `(name, value)` pairs.
    /// Empty if the act has no payload slots or the parser couldn't
    /// extract them.
    pub payload_slots: Vec<(String, String)>,
    /// Echo of the original utterance for diagnostics.
    pub utterance: String,
}

/// Parser errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ParseError {
    /// The first word didn't match any registered verb alias.
    #[error("no act registered for verb `{verb}`")]
    UnknownVerb {
        /// The token that failed lookup.
        verb: String,
    },

    /// The utterance was empty after trimming.
    #[error("empty utterance")]
    Empty,

    /// A required slot for the matched act could not be extracted
    /// from the utterance.
    #[error("missing payload slot `{slot}` for act {act_id} in utterance `{utterance}`")]
    MissingSlot {
        /// Which act we matched.
        act_id: String,
        /// Which slot we couldn't extract.
        slot: String,
        /// The original utterance.
        utterance: String,
    },

    /// The utterance was recognized but doesn't match any handled
    /// pattern. Caller should escalate to clarify or LLM.
    #[error("no parser pattern matched utterance `{0}`")]
    NoMatch(String),
}

/// Parse `utterance` into a [`ParsedUtterance`] using `registry`'s
/// verb aliases. Pure function.
pub fn parse_utterance(
    utterance: &str,
    registry: &ActRegistry,
) -> Result<ParsedUtterance, ParseError> {
    let trimmed = utterance.trim();
    if trimmed.is_empty() {
        return Err(ParseError::Empty);
    }

    let mut tokens = trimmed.split_whitespace();
    let verb_token = tokens.next().expect("trimmed non-empty has tokens");

    let act = registry
        .lookup_by_verb(verb_token)
        .ok_or_else(|| ParseError::UnknownVerb {
            verb: verb_token.to_owned(),
        })?;

    let rest: Vec<&str> = tokens.collect();
    let payload_slots = extract_payload_slots(act, &rest, trimmed)?;

    Ok(ParsedUtterance {
        act_id: act.id.clone(),
        payload_slots,
        utterance: trimmed.to_owned(),
    })
}

fn extract_payload_slots(
    act: &Act,
    tokens: &[&str],
    utterance: &str,
) -> Result<Vec<(String, String)>, ParseError> {
    let act_id = act.id.as_str();
    // `clippy::match_same_arms` is allowed because the no-payload-slot
    // arm and the catch-all arm differ in *intent* — the named acts
    // are deliberately silent (no payload to extract), the wildcard
    // covers acts the v0.2 parser hasn't been taught yet (v0.3 LLM
    // will fill it in). Merging them would lose that distinction.
    #[allow(clippy::match_same_arms)]
    match act_id {
        "file.rename" => extract_rename_slots(tokens, utterance),
        "file.copy" | "file.move" => extract_to_destination_slots(tokens, utterance, act_id),
        "browser.navigate" => extract_navigate_slots(tokens, utterance),
        // Acts with no payload slots in v0.2:
        "file.read"
        | "file.open"
        | "file.delete"
        | "file.save"
        | "workspace.undo"
        | "workspace.navigate_back"
        | "selection.copy"
        | "selection.paste"
        | "selection.select_all" => Ok(Vec::new()),
        // Catchall for acts the v0.2 parser doesn't recognize.
        _ => Ok(Vec::new()),
    }
}

/// Extract `new_name` from "rename [it] to NEW" / "rn NEW".
fn extract_rename_slots(
    tokens: &[&str],
    utterance: &str,
) -> Result<Vec<(String, String)>, ParseError> {
    let missing = || ParseError::MissingSlot {
        act_id: "file.rename".to_owned(),
        slot: "new_name".to_owned(),
        utterance: utterance.to_owned(),
    };
    let new_name = if let Some(idx) = position_lower(tokens, "to") {
        let after_to: Vec<String> = tokens
            .iter()
            .skip(idx + 1)
            .map(|t| (*t).to_string())
            .collect();
        if after_to.is_empty() {
            return Err(missing());
        }
        after_to.join(" ")
    } else {
        // No "to" — accept "rn NEW" / "rename NEW" — single arg case.
        // Skip over "it" (deictic) if present.
        let args: Vec<&str> = tokens
            .iter()
            .copied()
            .filter(|t| !is_filler_word(t))
            .collect();
        if args.len() == 1 {
            args[0].to_string()
        } else {
            return Err(missing());
        }
    };
    Ok(vec![("new_name".to_owned(), new_name)])
}

/// Extract `destination` from "copy/move [it] to PATH".
fn extract_to_destination_slots(
    tokens: &[&str],
    utterance: &str,
    act_id: &str,
) -> Result<Vec<(String, String)>, ParseError> {
    let missing = || ParseError::MissingSlot {
        act_id: act_id.to_owned(),
        slot: "destination".to_owned(),
        utterance: utterance.to_owned(),
    };
    if let Some(idx) = position_lower(tokens, "to") {
        let after_to: Vec<String> = tokens
            .iter()
            .skip(idx + 1)
            .map(|t| (*t).to_string())
            .collect();
        if after_to.is_empty() {
            return Err(missing());
        }
        Ok(vec![("destination".to_owned(), after_to.join(" "))])
    } else {
        Err(missing())
    }
}

/// Extract `url` from "navigate [to] URL" / "go [to] URL" / "browse [it] URL".
///
/// More permissive than the file-targeted extractors because URLs can
/// follow the verb directly (no `to` keyword) — `"go example.com"` is
/// fluent English. We strip filler words (`it`, `this`, `that`) and
/// take whatever non-filler tokens remain (or whatever follows `to`)
/// as the URL.
fn extract_navigate_slots(
    tokens: &[&str],
    utterance: &str,
) -> Result<Vec<(String, String)>, ParseError> {
    let missing = || ParseError::MissingSlot {
        act_id: "browser.navigate".to_owned(),
        slot: "url".to_owned(),
        utterance: utterance.to_owned(),
    };
    let url_words: Vec<String> = if let Some(idx) = position_lower(tokens, "to") {
        tokens
            .iter()
            .skip(idx + 1)
            .map(|t| (*t).to_string())
            .collect()
    } else {
        // No "to" — accept "go example.com" / "navigate example.com" / "browse example.com".
        tokens
            .iter()
            .copied()
            .filter(|t| !is_filler_word(t))
            .map(str::to_string)
            .collect()
    };
    if url_words.is_empty() {
        return Err(missing());
    }
    Ok(vec![("url".to_owned(), url_words.join(" "))])
}

fn position_lower(tokens: &[&str], needle: &str) -> Option<usize> {
    let needle_lower = needle.to_lowercase();
    tokens.iter().position(|t| t.to_lowercase() == needle_lower)
}

fn is_filler_word(t: &str) -> bool {
    matches!(
        t.to_lowercase().as_str(),
        "it" | "this" | "that" | "the" | "a" | "an" | "my" | "your"
    )
}
