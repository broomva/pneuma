//! # pneuma-resolver
//!
//! Replaces [`ReferentValue::Anaphor`] slot bindings with concrete
//! typed referents read from a [`WorkspaceContext`].
//!
//! ## What this crate is for
//!
//! Step #18 of `MIL-PROJECT.md` §11.2. The boundary between **utterance
//! surface** and **bound referent**.
//!
//! Without a resolver, `MIL_UTTERANCE='refactor this'` requires the
//! caller to supply an explicit file path. With a resolver, the
//! parser produces `ReferentValue::Anaphor(AnaphorRef::new("this"))`,
//! the demo composes a directive with that anaphor, and **before
//! finalize** the resolver swaps the anaphor for `ReferentValue::File(focused_file)`
//! — the file the workspace observer says is currently focused.
//!
//! ## What this crate is NOT
//!
//! - **Not a parser.** Anaphors arrive already-tagged.
//! - **Not turn-history aware.** "It" → previous turn's result is
//!   v0.3, requires conversational state.
//! - **Not a model.** Pure deterministic mapping from
//!   `(AnaphorRef, ReferentType, WorkspaceContext)` → `ReferentValue`.
//! - **Not a workspace observer.** That's `sensorium-context-macos`
//!   (step #15). The resolver consumes any `WorkspaceContext`.
//!
//! ## Resolution rules (v0.2)
//!
//! | Slot type    | Axis                          | Surface forms                     |
//! |--------------|-------------------------------|-----------------------------------|
//! | `File`       | `context.focused_file`        | this, that, the file, current     |
//! | `Window`     | `context.focused_window`      | this window, the focused window   |
//! | `App`        | `context.focused_app`         | this app, the active application  |
//! | `Any`        | preference: file > window > app | any of the above                |
//!
//! Slots that aren't `Anaphor`-typed pass through unchanged. Slots
//! whose anaphor surface form isn't recognized produce
//! [`ResolverError::UnknownAnaphor`].

#![doc = include_str!("../README.md")]

use pneuma_core::act::{ResolvedSlotValue, SlotKind};
use pneuma_core::{
    AnaphorRef, AppId, Composing, Directive, FileRef, ReferentType, ReferentValue, WindowId,
};
use sensorium_core::WorkspaceContext;
use thiserror::Error;

// --- Error -----------------------------------------------------------------

/// Errors the resolver can return.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum ResolverError {
    /// The anaphor's surface form isn't in our v0.2 vocabulary.
    #[error("unknown anaphor surface: `{surface}`")]
    UnknownAnaphor {
        /// The unrecognized surface form.
        surface: String,
    },

    /// The anaphor was recognized but the slot's `ReferentType`
    /// is incompatible with what the workspace can supply.
    #[error("anaphor `{surface}` cannot resolve to slot of type {slot_type:?}")]
    IncompatibleSlotType {
        /// The recognized surface form.
        surface: String,
        /// The slot's declared type.
        slot_type: ReferentType,
    },

    /// The workspace context is missing the focused entity needed
    /// to resolve this anaphor.
    #[error("workspace has no focused {axis} to resolve anaphor `{surface}`")]
    NoFocusedEntity {
        /// The recognized surface form.
        surface: String,
        /// Which axis was missing — `"file"`, `"window"`, or `"app"`.
        axis: &'static str,
    },

    /// Internal contract violation — we somehow constructed an invalid
    /// resolved value. Should never surface; kept as a typed escape
    /// hatch.
    #[error("internal contract error: {0}")]
    Contract(pneuma_core::ContractError),
}

// --- Public API ------------------------------------------------------------

/// Walk a [`Directive<Composing>`]'s slot bindings and replace any
/// [`ReferentValue::Anaphor`] values with concrete typed referents
/// resolved from `context`.
///
/// Returns the directive with all anaphors resolved, or the first
/// resolution error encountered. Non-anaphor bindings pass through
/// unchanged.
pub fn resolve_directive(
    mut directive: Directive<Composing>,
    context: &WorkspaceContext,
) -> Result<Directive<Composing>, ResolverError> {
    // Snapshot slot signatures so we can look up declared types
    // without holding a borrow on directive.act.act.
    let signatures: Vec<(String, ReferentType)> = directive
        .act
        .act
        .slots
        .iter()
        .filter_map(|sig| match &sig.kind {
            SlotKind::Referent(rt) => Some((sig.name.clone(), *rt)),
            _ => None,
        })
        .collect();

    for binding in &mut directive.act.bindings {
        if let ResolvedSlotValue::Referent(ReferentValue::Anaphor(anaphor)) = &binding.value {
            // Look up the slot's declared ReferentType. If the slot
            // isn't a referent slot at all, that's a contract bug
            // upstream — pass through.
            let slot_type = signatures
                .iter()
                .find(|(name, _)| name == &binding.name)
                .map_or(ReferentType::Any, |(_, ty)| *ty);

            let resolved = resolve_anaphor(anaphor, slot_type, context)?;
            binding.value = ResolvedSlotValue::Referent(resolved);
        }
    }

    Ok(directive)
}

/// Resolve a single anaphor against a workspace context, given the
/// slot's declared type.
///
/// This is the single decision point. v0.2 implements the rule table
/// from this crate's docs:
///
/// - File slot → `context.focused_file`
/// - Window slot → `context.focused_window`
/// - App slot → `context.focused_app`
/// - Any slot → file > window > app preference order
///
/// Other slot types (Selection, Symbol, Url, Set, Range, Locus,
/// Anaphor) return [`ResolverError::IncompatibleSlotType`] in v0.2.
pub fn resolve_anaphor(
    anaphor: &AnaphorRef,
    slot_type: ReferentType,
    context: &WorkspaceContext,
) -> Result<ReferentValue, ResolverError> {
    let surface = anaphor.surface.trim().to_lowercase();
    let recognized = is_deictic_surface(&surface);
    if !recognized {
        return Err(ResolverError::UnknownAnaphor {
            surface: anaphor.surface.clone(),
        });
    }

    // Hint can override the slot-type axis. e.g.,
    // AnaphorRef::new("this").with_hint("window") forces window axis
    // even if the slot is Any. v0.2 honors hint when present.
    let axis = anaphor.hint.as_deref().map(str::to_lowercase);

    // Hint dominates slot type when present. Otherwise route by slot type.
    match axis.as_deref() {
        Some("window") => return resolve_window(anaphor, context),
        Some("app" | "application") => return resolve_app(anaphor, context),
        Some("file") => return resolve_file(anaphor, context),
        _ => {}
    }
    match slot_type {
        ReferentType::Window => resolve_window(anaphor, context),
        ReferentType::App => resolve_app(anaphor, context),
        ReferentType::File => resolve_file(anaphor, context),
        ReferentType::Any => resolve_any(anaphor, context),
        // Other slot types are not resolvable from anaphors in v0.2.
        other => Err(ResolverError::IncompatibleSlotType {
            surface: anaphor.surface.clone(),
            slot_type: other,
        }),
    }
}

// --- Per-axis resolution ---------------------------------------------------

fn resolve_file(
    anaphor: &AnaphorRef,
    context: &WorkspaceContext,
) -> Result<ReferentValue, ResolverError> {
    // Convention from sensorium-context: the first entry in
    // `visible_files` is treated as the focused file. `set_focused_file`
    // sets `visible_files = [file]` so this round-trips cleanly.
    context
        .state()
        .visible_files
        .first()
        .map(|f| ReferentValue::File(file_ref_from_sensorium(f)))
        .ok_or(ResolverError::NoFocusedEntity {
            surface: anaphor.surface.clone(),
            axis: "file",
        })
}

fn resolve_window(
    anaphor: &AnaphorRef,
    context: &WorkspaceContext,
) -> Result<ReferentValue, ResolverError> {
    context
        .focused_window()
        .map(|w| {
            // sensorium_core::WindowId mirrors pneuma_core::WindowId — same
            // string-typed identifier shape, but they're different types
            // that need conversion.
            WindowId::new(w.as_str())
                .map(ReferentValue::Window)
                .map_err(ResolverError::Contract)
        })
        .transpose()?
        .ok_or(ResolverError::NoFocusedEntity {
            surface: anaphor.surface.clone(),
            axis: "window",
        })
}

fn resolve_app(
    anaphor: &AnaphorRef,
    context: &WorkspaceContext,
) -> Result<ReferentValue, ResolverError> {
    context
        .focused_app()
        .map(|a| {
            AppId::new(a.as_str())
                .map(ReferentValue::App)
                .map_err(ResolverError::Contract)
        })
        .transpose()?
        .ok_or(ResolverError::NoFocusedEntity {
            surface: anaphor.surface.clone(),
            axis: "app",
        })
}

fn resolve_any(
    anaphor: &AnaphorRef,
    context: &WorkspaceContext,
) -> Result<ReferentValue, ResolverError> {
    // Preference order: file > window > app. The first axis with a
    // populated focused entity wins. If all three are None, we error.
    if let Ok(v) = resolve_file(anaphor, context) {
        return Ok(v);
    }
    if let Ok(v) = resolve_window(anaphor, context) {
        return Ok(v);
    }
    if let Ok(v) = resolve_app(anaphor, context) {
        return Ok(v);
    }
    Err(ResolverError::NoFocusedEntity {
        surface: anaphor.surface.clone(),
        axis: "any (no file/window/app focused)",
    })
}

// --- Surface-form recognizer ------------------------------------------------

/// Returns `true` if the (lowercased, trimmed) surface form is a v0.2
/// recognized deictic.
///
/// v0.2 vocabulary: `this`, `that`, `it`, `the`, `current`, `focused`,
/// `the file`, `this file`, `the focused file`, `the window`,
/// `this window`, `the focused window`, `the app`, `this app`,
/// `the active application`, `the active app`.
///
/// We accept partial matches via prefix detection so callers can
/// pass `"the focused window"` and we recognize it.
#[must_use]
pub fn is_deictic_surface(s: &str) -> bool {
    const RECOGNIZED: &[&str] = &["this", "that", "it", "the", "current", "focused"];
    let trimmed = s.trim().to_lowercase();
    if trimmed.is_empty() {
        return false;
    }
    // Exact match.
    if RECOGNIZED.iter().any(|&r| r == trimmed) {
        return true;
    }
    // Prefix match: "this file", "the focused window", "current app".
    RECOGNIZED.iter().any(|&r| {
        trimmed.starts_with(r)
            && trimmed
                .as_bytes()
                .get(r.len())
                .copied()
                .is_some_and(|b| b == b' ')
    })
}

// --- Cross-crate FileRef conversion ----------------------------------------

/// `sensorium_core::FileRef` and `pneuma_core::FileRef` are
/// structurally identical but distinct types. v0.2 converts via the
/// public path field; v0.3 may unify both behind a shared crate.
fn file_ref_from_sensorium(f: &sensorium_core::FileRef) -> FileRef {
    FileRef::new(f.path.clone())
}

// --- Cross-crate Generation bridge -----------------------------------------

/// Bridge a `sensorium_core::Generation` to a `pneuma_core::Generation`.
///
/// Both types are `u64` newtypes that serialize transparently, but
/// they live in different crates and Rust's orphan rule forbids
/// `From`/`Into` impls in either core crate (the trait and both
/// types would be foreign in either site). `pneuma-resolver` depends
/// on both, so the bridge lives here as a free function.
///
/// Use this at the seam where a streaming substrate emits voice
/// `StreamUpdate<TranscriptDelta>`s tagged with
/// `sensorium_core::Generation`, and you want to attach the same
/// generation to the derived `Directive<Composing>`:
///
/// ```rust,ignore
/// // generation arrives from sensorium_voice::VoiceSession::current_generation()
/// let directive = parse_to_directive(&transcript)
///     .with_generation(pneuma_resolver::bridge_generation(generation));
/// ```
#[must_use]
pub fn bridge_generation(g: sensorium_core::Generation) -> pneuma_core::Generation {
    pneuma_core::Generation::new(g.into_inner())
}
