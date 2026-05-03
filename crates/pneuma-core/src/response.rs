//! Bidirectional types — what the agent emits back to the user.
//!
//! From `MIL-PROJECT.md` §6.4:
//!
//! > Agents emit IR fragments back through the same contract — `Plan`,
//! > `Progress`, `Proposed`, `Clarify`, `Done`, `Error`. Same language
//! > in both directions. Prosopon renders them; the user responds via
//! > the same primitive channels.
//!
//! These types live here (in `pneuma-core`) for v0.2. They may be
//! extracted to `prosopon-core` in a later refactor — see
//! `MIL-PROJECT.md` §11.1 task 3. The wire format is what matters; the
//! crate boundary can shift later without breaking journals.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::directive::DirectiveId;

// --- StepStatus --------------------------------------------------------------

/// Status of a single planned step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum StepStatus {
    /// Step is queued, not yet started.
    Pending,
    /// Step is currently executing.
    Running,
    /// Step completed successfully.
    Done,
    /// Step failed.
    Failed,
    /// Step was skipped (e.g. preconditions not met).
    Skipped,
}

// --- CostClass ---------------------------------------------------------------

/// Coarse cost classification for a planned step. Used by the HUD to
/// surface "this step might be expensive" before commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CostClass {
    /// Free / negligible.
    Free,
    /// Small cost — quick LLM call, single API request.
    Small,
    /// Medium cost — multi-step agent loop, several API calls.
    Medium,
    /// Large cost — long-running agent task, expensive model call.
    Large,
    /// External cost — actual money / external billing involved.
    External,
}

// --- PlannedStep -------------------------------------------------------------

/// A single step in the agent's plan. Streamed back as the agent
/// works.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedStep {
    /// Unique step identifier within the directive's plan.
    pub step_id: String,
    /// Free-form description for the HUD.
    pub description: String,
    /// Cost classification.
    pub cost: CostClass,
    /// Current status.
    pub status: StepStatus,
    /// IDs of steps this one depends on.
    #[serde(default)]
    pub depends_on: Vec<String>,
}

// --- ProgressUpdate ----------------------------------------------------------

/// A progress update from the agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgressUpdate {
    /// The step being updated.
    pub step_id: String,
    /// Progress fraction in `[0.0, 1.0]`.
    pub fraction: f32,
    /// Free-form status message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

// --- ProposalKind ------------------------------------------------------------

/// What kind of proposal the agent is making.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ProposalKind {
    /// "Here's what I plan to do" — show the plan, await approval.
    PlanApproval,
    /// "I need to do X, which is Irreversible" — ratification gate.
    IrreversibleAction,
    /// "I have results — accept or reject" — output review.
    ResultReview,
}

// --- ClarifyOption -----------------------------------------------------------

/// A single option in a clarification request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClarifyOption {
    /// Identifier the user picks.
    pub option_id: String,
    /// Display label.
    pub label: String,
    /// Optional preview / description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

// --- DirectiveError ----------------------------------------------------------

/// Error returned by an executor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectiveError {
    /// Stable error code (e.g. `"file.not_found"`).
    pub code: String,
    /// Human-readable error message.
    pub message: String,
    /// Optional structured payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

// --- DirectiveResult ---------------------------------------------------------

/// Successful executor result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectiveResult {
    /// Free-form structured result data.
    pub payload: serde_json::Value,
    /// Reverse-action recipe (or recipe ID) the executor produced for
    /// undo. Praxis fills this synchronously; Arcan fills it at
    /// completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverse_action: Option<serde_json::Value>,
}

// --- AgentResponse -----------------------------------------------------------

/// What the agent emits back through the contract. Prosopon renders
/// these in the HUD.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AgentResponse {
    /// "Here is my plan."
    Plan {
        /// Which directive this plan responds to.
        directive_id: DirectiveId,
        /// The planned steps.
        steps: Vec<PlannedStep>,
        /// When the plan was emitted.
        emitted_at: DateTime<Utc>,
    },
    /// Streaming progress on a step.
    Progress {
        /// Which directive is in progress.
        directive_id: DirectiveId,
        /// The progress update.
        update: ProgressUpdate,
        /// When emitted.
        emitted_at: DateTime<Utc>,
    },
    /// "I need approval before proceeding."
    Proposed {
        /// Which directive is proposing.
        directive_id: DirectiveId,
        /// What kind of proposal.
        kind: ProposalKind,
        /// Free-form summary for the HUD.
        summary: String,
        /// When emitted.
        emitted_at: DateTime<Utc>,
    },
    /// "I'm uncertain — pick one."
    Clarify {
        /// Which directive needs clarification.
        directive_id: DirectiveId,
        /// What's ambiguous.
        question: String,
        /// Options for the user.
        options: Vec<ClarifyOption>,
        /// When emitted.
        emitted_at: DateTime<Utc>,
    },
    /// "Done."
    Done {
        /// Which directive completed.
        directive_id: DirectiveId,
        /// Result payload.
        result: DirectiveResult,
        /// When emitted.
        emitted_at: DateTime<Utc>,
    },
    /// "I failed."
    Error {
        /// Which directive failed.
        directive_id: DirectiveId,
        /// Error details.
        error: DirectiveError,
        /// When emitted.
        emitted_at: DateTime<Utc>,
    },
}

impl AgentResponse {
    /// The directive ID this response references.
    #[must_use]
    pub fn directive_id(&self) -> DirectiveId {
        match self {
            Self::Plan { directive_id, .. }
            | Self::Progress { directive_id, .. }
            | Self::Proposed { directive_id, .. }
            | Self::Clarify { directive_id, .. }
            | Self::Done { directive_id, .. }
            | Self::Error { directive_id, .. } => *directive_id,
        }
    }
}
