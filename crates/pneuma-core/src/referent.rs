//! Referent — the entities a directive can pick out.
//!
//! From `MIL-PROJECT.md` §6.1:
//!
//! ```text
//! enum Referent {
//!     File(FileRef), Region(SelectionRef), Selection,
//!     Window(WindowId), App(AppId), Symbol(SymbolRef), Url(String),
//!     Set(Vec<Referent>), Range { from: Box<Referent>, to: Box<Referent> },
//!     Anaphor(AnaphorRef), Locus(SpatialAnchor),
//! }
//! ```
//!
//! ## Coordination with `sensorium-core`
//!
//! [`AppId`], [`WindowId`], [`FileRef`], [`SymbolRef`], [`SelectionRef`]
//! are structurally identical to their counterparts in
//! `sensorium_core::entity`. Both crates ship them; for v0.2 we accept
//! the duplication. A future `pneuma-sensorium` glue crate will
//! provide `From`/`Into` impls. The wire formats are byte-identical.
//!
//! ## Why two enums (`ReferentValue` and `ReferentType`)
//!
//! `ReferentValue` carries data; `ReferentType` is the discriminant for
//! type-checking slot signatures. Slot validation compares types, not
//! values, so we keep them separate. This matches the `Act` schema's
//! `SlotKind::referent_type` — the act says "this slot wants a `File`",
//! and we check the `ReferentValue::File(...)` variant against
//! `ReferentType::File`.

use serde::{Deserialize, Serialize};

use crate::error::{ContractError, Result};

// --- ID newtypes -------------------------------------------------------------

/// Application identifier — bundle ID on macOS, `wmclass` on X11, etc.
/// Mirrors `sensorium_core::AppId`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AppId(String);

impl AppId {
    /// Construct, rejecting empty / whitespace-only input.
    pub fn new(raw: impl Into<String>) -> Result<Self> {
        let trimmed = raw.into().trim().to_owned();
        if trimmed.is_empty() {
            return Err(ContractError::EmptyIdentifier { field: "AppId" });
        }
        Ok(Self(trimmed))
    }

    /// Borrow the underlying string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Window identifier within an application. Mirrors `sensorium_core::WindowId`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WindowId(String);

impl WindowId {
    /// Construct, rejecting empty input.
    pub fn new(raw: impl Into<String>) -> Result<Self> {
        let trimmed = raw.into().trim().to_owned();
        if trimmed.is_empty() {
            return Err(ContractError::EmptyIdentifier { field: "WindowId" });
        }
        Ok(Self(trimmed))
    }

    /// Borrow the underlying string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// --- File / Selection / Symbol ----------------------------------------------

/// A byte-offset span. `[start, end)` half-open. `end >= start` enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TextSpan {
    /// Start byte offset.
    pub start: u64,
    /// End byte offset (exclusive).
    pub end: u64,
}

impl TextSpan {
    /// Construct, rejecting `end < start`.
    pub fn new(start: u64, end: u64) -> Result<Self> {
        if end < start {
            return Err(ContractError::InvalidSpan { start, end });
        }
        Ok(Self { start, end })
    }

    /// Length in bytes.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.end - self.start
    }

    /// `true` if zero length (cursor position).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// Reference to a file. Mirrors `sensorium_core::FileRef`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileRef {
    /// Absolute filesystem path.
    pub path: std::path::PathBuf,
    /// MIME type if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

impl FileRef {
    /// Construct from a path.
    #[must_use]
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into(), mime: None }
    }

    /// Attach a MIME type.
    #[must_use]
    pub fn with_mime(mut self, mime: impl Into<String>) -> Self {
        self.mime = Some(mime.into());
        self
    }
}

/// Reference to a code symbol. Mirrors `sensorium_core::SymbolRef`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SymbolRef {
    /// Owning file.
    pub file: FileRef,
    /// Fully-qualified symbol name.
    pub qualified_name: String,
    /// Symbol kind (free-form, e.g. "function", "struct").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

impl SymbolRef {
    /// Construct, rejecting empty `qualified_name`.
    pub fn new(file: FileRef, qualified_name: impl Into<String>) -> Result<Self> {
        let qn = qualified_name.into().trim().to_owned();
        if qn.is_empty() {
            return Err(ContractError::EmptyIdentifier {
                field: "SymbolRef.qualified_name",
            });
        }
        Ok(Self { file, qualified_name: qn, kind: None })
    }

    /// Attach a kind.
    #[must_use]
    pub fn with_kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = Some(kind.into());
        self
    }
}

/// Selection reference: file + byte span. Mirrors
/// `sensorium_core::SelectionRef`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SelectionRef {
    /// Owning file.
    pub file: FileRef,
    /// Byte span.
    pub span: TextSpan,
}

impl SelectionRef {
    /// Construct.
    #[must_use]
    pub fn new(file: FileRef, span: TextSpan) -> Self {
        Self { file, span }
    }
}

// --- Anaphora & spatial ------------------------------------------------------

/// Reference to a previously-mentioned entity ("the file", "that
/// selection"). Resolved by `pneuma-resolver` against recent activity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AnaphorRef {
    /// The deictic surface form ("it", "that", "the previous one").
    pub surface: String,
    /// Optional disambiguator hint extracted by the parser.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl AnaphorRef {
    /// Construct, rejecting empty surface form.
    pub fn new(surface: impl Into<String>) -> Result<Self> {
        let s = surface.into().trim().to_owned();
        if s.is_empty() {
            return Err(ContractError::EmptyIdentifier { field: "AnaphorRef.surface" });
        }
        Ok(Self { surface: s, hint: None })
    }

    /// Attach a disambiguator hint.
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// User-established referent in signing space ("over here", drawn
/// region in air). v0.3 feature; the type is in v0.2 for forward
/// compatibility.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpatialAnchor {
    /// Anchor identifier within the current spatial coordinate frame.
    pub anchor_id: String,
    /// Center of the anchor in 3D normalized signing-space coordinates.
    pub center: [f32; 3],
    /// Anchor radius (normalized).
    pub radius: f32,
}

// --- ReferentType (the discriminant) ----------------------------------------

/// The type of a referent — used by slot signatures to declare what
/// kind of entity a slot accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ReferentType {
    /// File reference.
    File,
    /// Selection (file + range).
    Selection,
    /// Window reference.
    Window,
    /// Application reference.
    App,
    /// Code symbol.
    Symbol,
    /// URL.
    Url,
    /// Set of referents (array).
    Set,
    /// Range from one referent to another.
    Range,
    /// Anaphor (must be resolved before dispatch).
    Anaphor,
    /// Spatial locus.
    Locus,
    /// "Any referent type" — slot accepts any leaf.
    Any,
}

// --- ReferentValue (the data) ------------------------------------------------

/// A bound referent value. Each variant carries the leaf type's data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ReferentValue {
    /// File.
    File(FileRef),
    /// Selection.
    Selection(SelectionRef),
    /// Window.
    Window(WindowId),
    /// Application.
    App(AppId),
    /// Code symbol.
    Symbol(SymbolRef),
    /// URL.
    Url(String),
    /// Set of referents.
    Set(Vec<ReferentValue>),
    /// Range.
    Range {
        /// Start of the range.
        from: Box<ReferentValue>,
        /// End of the range.
        to: Box<ReferentValue>,
    },
    /// Anaphor — unresolved at this state. Resolver must replace
    /// before dispatch.
    Anaphor(AnaphorRef),
    /// Spatial locus.
    Locus(SpatialAnchor),
}

impl ReferentValue {
    /// The type discriminant of this value, for slot type-checking.
    #[must_use]
    pub fn type_of(&self) -> ReferentType {
        match self {
            Self::File(_) => ReferentType::File,
            Self::Selection(_) => ReferentType::Selection,
            Self::Window(_) => ReferentType::Window,
            Self::App(_) => ReferentType::App,
            Self::Symbol(_) => ReferentType::Symbol,
            Self::Url(_) => ReferentType::Url,
            Self::Set(_) => ReferentType::Set,
            Self::Range { .. } => ReferentType::Range,
            Self::Anaphor(_) => ReferentType::Anaphor,
            Self::Locus(_) => ReferentType::Locus,
        }
    }

    /// `true` if this value is a leaf (not a Set, Range, or Anaphor).
    /// Composite/unresolved values must be flattened before dispatch.
    #[must_use]
    pub fn is_leaf(&self) -> bool {
        !matches!(self, Self::Set(_) | Self::Range { .. } | Self::Anaphor(_))
    }
}
