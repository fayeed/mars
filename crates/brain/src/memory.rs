//! Human-approved, traceable memory.
//!
//! Two rules govern everything here. Every memory is approved by a person, and
//! every memory can be traced back to the exact conversation turn that
//! motivated it. Nothing is written from inference alone.
//!
//! The first rule is enforced structurally rather than by discipline: an agent
//! can only reach [`MemoryService::propose`], which writes to a different table
//! than the one retrieval reads. There is no code path from a proposal to a
//! memory that does not pass through [`MemoryService::approve`] and record who
//! approved it. The second rule is why a proposal without the excerpt that
//! motivated it is rejected outright.
//!
//! This is distinct from [`crate::knowledge`], which projects one completed
//! agent session onto reviewable claims about that task. Memory is durable
//! guidance that outlives the conversation it came from.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use mars_indexer::embedding;
use mars_storage::{Storage, StorageError};
use rusqlite::{OptionalExtension, Row, ToSql, params, params_from_iter};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

/// A memory is a sentence or two of guidance. A cap keeps it that way: a
/// memory long enough to need scrolling is a document, and belongs in one.
const MAX_TEXT_CHARS: usize = 2_000;

/// The excerpt that motivated a proposal. Generous, because a motivating turn
/// can be a paragraph, but bounded so a transcript cannot be smuggled whole.
const MAX_QUOTED_CHARS: usize = 8_000;

/// Cosine similarity above which two active memories are shown to a human as a
/// possible conflict. Tuned to catch restatements of the same rule; see the
/// note on [`ConflictSignal`] for what this can and cannot detect.
const CONFLICT_SIMILARITY: f32 = 0.72;

pub const DEFAULT_RETRIEVAL_LIMIT: usize = 12;
pub const MAX_RETRIEVAL_LIMIT: usize = 50;
pub const MAX_LIST_LIMIT: usize = 500;

/// Retrieval floor. Below this a memory is not about the task at hand, and
/// injecting it would spend context to make the agent less focused.
const RETRIEVAL_FLOOR: f32 = 0.05;

// --- vocabulary ---------------------------------------------------------

/// Where a memory applies.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    Repo,
    Task,
    Global,
}

impl MemoryScope {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Repo => "repo",
            Self::Task => "task",
            Self::Global => "global",
        }
    }

    fn parse(value: &str) -> Result<Self, MemoryError> {
        match value {
            "repo" => Ok(Self::Repo),
            "task" => Ok(Self::Task),
            "global" => Ok(Self::Global),
            other => Err(MemoryError::InvalidStoredValue(other.into())),
        }
    }

    /// How much a match in this scope is preferred over an equally similar
    /// match in a broader one. A task-scoped memory was written about exactly
    /// this work; a global one was not.
    fn specificity_bonus(self) -> f32 {
        match self {
            Self::Task => 0.10,
            Self::Repo => 0.05,
            Self::Global => 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Convention,
    Decision,
    Preference,
    Fact,
    Caution,
}

impl MemoryKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Convention => "convention",
            Self::Decision => "decision",
            Self::Preference => "preference",
            Self::Fact => "fact",
            Self::Caution => "caution",
        }
    }

    fn parse(value: &str) -> Result<Self, MemoryError> {
        match value {
            "convention" => Ok(Self::Convention),
            "decision" => Ok(Self::Decision),
            "preference" => Ok(Self::Preference),
            "fact" => Ok(Self::Fact),
            "caution" => Ok(Self::Caution),
            other => Err(MemoryError::InvalidStoredValue(other.into())),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Active,
    Superseded,
    Revoked,
}

impl MemoryStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
            Self::Revoked => "revoked",
        }
    }

    fn parse(value: &str) -> Result<Self, MemoryError> {
        match value {
            "active" => Ok(Self::Active),
            "superseded" => Ok(Self::Superseded),
            "revoked" => Ok(Self::Revoked),
            other => Err(MemoryError::InvalidStoredValue(other.into())),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Pending,
    Approved,
    Rejected,
}

impl ProposalStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }

    fn parse(value: &str) -> Result<Self, MemoryError> {
        match value {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            other => Err(MemoryError::InvalidStoredValue(other.into())),
        }
    }
}

/// Lifecycle transitions. Each one is an entry in the audit spine, and an
/// approval additionally carries the conversation turn it was spoken in, so a
/// transcript view can render it inline as the message it is.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryEventKind {
    Proposed,
    Approved,
    EditedAndApproved,
    Rejected,
    Revised,
    Superseded,
    Revoked,
    ConflictResolved,
}

impl MemoryEventKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Approved => "approved",
            Self::EditedAndApproved => "edited_and_approved",
            Self::Rejected => "rejected",
            Self::Revised => "revised",
            Self::Superseded => "superseded",
            Self::Revoked => "revoked",
            Self::ConflictResolved => "conflict_resolved",
        }
    }

    fn parse(value: &str) -> Result<Self, MemoryError> {
        match value {
            "proposed" => Ok(Self::Proposed),
            "approved" => Ok(Self::Approved),
            "edited_and_approved" => Ok(Self::EditedAndApproved),
            "rejected" => Ok(Self::Rejected),
            "revised" => Ok(Self::Revised),
            "superseded" => Ok(Self::Superseded),
            "revoked" => Ok(Self::Revoked),
            "conflict_resolved" => Ok(Self::ConflictResolved),
            other => Err(MemoryError::InvalidStoredValue(other.into())),
        }
    }
}

// --- records ------------------------------------------------------------

/// The conversation turn a memory came from.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryOrigin {
    pub conversation_id: String,
    pub message_id: String,
    /// The exact excerpt that motivated the proposal.
    pub quoted_text: String,
    /// Whether `quoted_text` was checked against the referenced turn. False
    /// when no transcript source was configured, which makes the trace
    /// recorded but unconfirmed.
    #[serde(default)]
    pub verified: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProposal {
    pub id: String,
    pub project_id: String,
    pub repository_id: Option<String>,
    pub task_id: Option<String>,
    pub scope: MemoryScope,
    pub kind: MemoryKind,
    pub text: String,
    pub confidence: f64,
    pub origin: MemoryOrigin,
    pub proposed_by: String,
    pub proposed_at: String,
    pub status: ProposalStatus,
    pub resolved_by: Option<String>,
    pub resolved_at: Option<String>,
    pub rejection_reason: Option<String>,
    /// The memory this proposal became, once approved.
    pub memory_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    pub id: String,
    pub project_id: String,
    pub repository_id: Option<String>,
    pub task_id: Option<String>,
    pub scope: MemoryScope,
    pub kind: MemoryKind,
    pub text: String,
    pub confidence: f64,
    pub origin: MemoryOrigin,
    pub proposal_id: String,
    pub approved_by: String,
    pub approved_at: String,
    pub supersedes: Option<String>,
    pub status: MemoryStatus,
    pub revoked_by: Option<String>,
    pub revoked_at: Option<String>,
    pub revocation_reason: Option<String>,
    /// How many times this memory has been injected into an agent's context.
    pub use_count: usize,
    /// How many distinct tasks it has shaped -- the figure behind "used in 4
    /// tasks". The full trail is served by [`MemoryService::usage`] rather than
    /// inlined here, so listing memories never carries an unbounded array.
    pub used_in_tasks: usize,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryUse {
    pub id: String,
    pub memory_id: String,
    pub task_id: Option<String>,
    pub session_id: Option<String>,
    /// What injected it: an agent context pack, a Brain answer, a prompt.
    pub surface: String,
    pub relevance: f64,
    pub used_at: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryUsage {
    pub memory_id: String,
    pub use_count: usize,
    pub used_in_tasks: usize,
    pub uses: Vec<MemoryUse>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEvent {
    pub id: String,
    pub proposal_id: Option<String>,
    pub memory_id: Option<String>,
    pub kind: MemoryEventKind,
    pub actor: String,
    pub conversation_id: String,
    pub message_id: String,
    pub detail: String,
    pub occurred_at: String,
}

// --- conflicts ----------------------------------------------------------

/// Why two memories were shown together.
///
/// Neither value claims to have detected a contradiction. Deciding that two
/// statements contradict each other is a judgement about meaning, and this
/// layer has a feature-hashing embedder and a word list, not a reasoner. What
/// it does is narrow the field to the pairs a person should look at, and then
/// require the person to choose. `OpposingPolarity` says the two say similar
/// things and one of them negates; that is a stronger hint, not a verdict.
/// Declaration order is severity order: `OpposingPolarity` sorts above
/// `Overlap`, so the pair most likely to need a decision is shown first.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictSignal {
    Overlap,
    OpposingPolarity,
}

impl ConflictSignal {
    fn as_str(self) -> &'static str {
        match self {
            Self::Overlap => "overlap",
            Self::OpposingPolarity => "opposing_polarity",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedConflict {
    pub other: Memory,
    pub similarity: f64,
    pub signal: ConflictSignal,
    /// Plain-language statement of why this pair was surfaced.
    pub explanation: String,
}

/// What a person decided about one flagged pair.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictChoice {
    /// Both are true; they were only similar.
    KeepBoth,
    /// The new memory replaces the old one, linked by `supersedes`.
    SupersedeOther,
    /// The old one was wrong and is withdrawn, keeping its audit trail.
    RevokeOther,
    /// The old one wins; this proposal is rejected.
    KeepExisting,
}

impl ConflictChoice {
    fn as_resolution(self) -> &'static str {
        match self {
            Self::KeepBoth | Self::KeepExisting => "kept_both",
            Self::SupersedeOther => "superseded_other",
            Self::RevokeOther => "revoked_other",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictResolution {
    pub other_memory_id: String,
    pub choice: ConflictChoice,
}

/// The result of asking to approve a proposal.
// Returned once per human approval, so the size gap between variants costs
// nothing and boxing would only make the public API noisier to consume.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "outcome"
)]
pub enum ApprovalOutcome {
    Approved {
        memory: Memory,
        /// The approval, as a message in the origin conversation.
        message: MemoryEvent,
    },
    /// Nothing was written. The listed pairs must each be resolved and the
    /// approval retried with those choices.
    ConflictsRequireChoice { conflicts: Vec<DetectedConflict> },
    /// The person chose an existing memory over this proposal, so the
    /// proposal was rejected instead of being applied.
    KeptExisting {
        proposal: MemoryProposal,
        kept: Vec<String>,
    },
}

// --- requests -----------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewProposal {
    pub project_id: String,
    pub repository_id: Option<String>,
    pub task_id: Option<String>,
    pub scope: MemoryScope,
    pub kind: MemoryKind,
    pub text: String,
    pub confidence: f64,
    pub conversation_id: String,
    pub message_id: String,
    /// The exact transcript excerpt that motivated this. Required: a memory
    /// nobody can trace is not a memory, it is a rumour.
    pub quoted_text: String,
    /// The agent or summarizer proposing it.
    pub proposed_by: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalFilters {
    pub project_id: String,
    pub repository_id: Option<String>,
    pub task_id: Option<String>,
    #[serde(default)]
    pub statuses: Vec<ProposalStatus>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApproveRequest {
    pub proposal_id: String,
    /// The person approving. Recorded verbatim on the memory.
    pub approved_by: String,
    /// Present when the human edited before accepting.
    pub edited_text: Option<String>,
    /// Choices for every conflict a previous call reported. Must match that
    /// set exactly; a stale set is refused rather than partly applied.
    #[serde(default)]
    pub resolutions: Vec<ConflictResolution>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectRequest {
    pub proposal_id: String,
    pub rejected_by: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviseRequest {
    pub memory_id: String,
    pub text: String,
    pub revised_by: String,
    /// Omitted keeps the current kind.
    pub kind: Option<MemoryKind>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeRequest {
    pub memory_id: String,
    pub revoked_by: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFilters {
    pub project_id: String,
    pub repository_id: Option<String>,
    pub task_id: Option<String>,
    #[serde(default)]
    pub scopes: Vec<MemoryScope>,
    #[serde(default)]
    pub kinds: Vec<MemoryKind>,
    /// Empty means active only. Listing withdrawn memories is deliberate.
    #[serde(default)]
    pub statuses: Vec<MemoryStatus>,
    pub contains: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// What an agent is about to do, so memory can be selected for it.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalContext {
    pub project_id: String,
    pub repository_id: Option<String>,
    pub task_id: Option<String>,
    pub session_id: Option<String>,
    /// The task intent or question the memories will be injected alongside.
    pub query: String,
    /// What is injecting them. Recorded on every use.
    pub surface: String,
    #[serde(default)]
    pub limit: Option<usize>,
    /// False previews the selection without recording a use, so a UI can show
    /// what would be injected without inflating the usage trail.
    #[serde(default = "default_true")]
    pub record_uses: bool,
}

fn default_true() -> bool {
    true
}

/// One memory that was actually put in front of an agent, with the relevance
/// it was selected at.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryInjection {
    pub memory_id: String,
    pub relevance: f64,
}

/// Where an injection happened.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InjectionContext {
    pub task_id: Option<String>,
    pub session_id: Option<String>,
    pub surface: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievedMemory {
    pub memory: Memory,
    pub relevance: f64,
    /// Why this memory was selected, in the retriever's own words.
    pub reason: String,
}

// --- errors -------------------------------------------------------------

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("failed to format a memory timestamp: {0}")]
    Timestamp(#[from] time::error::Format),
    #[error("stored memory value `{0}` is not one this version understands")]
    InvalidStoredValue(String),
    #[error("proposal `{0}` does not exist")]
    ProposalNotFound(String),
    #[error("memory `{0}` does not exist")]
    MemoryNotFound(String),
    #[error("proposal `{id}` is already {status}")]
    ProposalNotPending { id: String, status: String },
    #[error("memory `{id}` is {status}, so it cannot be {action}")]
    MemoryNotActive {
        id: String,
        status: String,
        action: &'static str,
    },
    #[error("{field} must not be empty")]
    Empty { field: &'static str },
    #[error("{field} exceeds {maximum} characters")]
    TooLong { field: &'static str, maximum: usize },
    #[error("confidence {0} is outside 0.0 to 1.0")]
    ConfidenceOutOfRange(f64),
    #[error("a {scope}-scoped memory needs {field}")]
    ScopeIncomplete {
        scope: &'static str,
        field: &'static str,
    },
    #[error(
        "the conflicts resolved do not match the conflicts detected; re-read them and choose again"
    )]
    StaleConflictSet,
    #[error("only one conflicting memory may be superseded by a single approval")]
    MultipleSupersedes,
    #[error("the quoted excerpt does not appear in message `{message_id}` of `{conversation_id}`")]
    OriginNotFound {
        conversation_id: String,
        message_id: String,
    },
    #[error("task `{0}` does not exist")]
    TaskNotFound(String),
}

// --- transcripts --------------------------------------------------------

/// Resolves a conversation turn so a proposal's excerpt can be checked against
/// it. Without one, an origin is recorded but unverified.
pub trait TranscriptSource: Send + Sync {
    fn message_text(&self, conversation_id: &str, message_id: &str) -> Option<String>;
}

/// Reads agent session events as a transcript: the conversation is a session
/// and a message is one event's sequence number.
pub struct AgentSessionTranscript {
    storage: Arc<Storage>,
}

impl AgentSessionTranscript {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }
}

impl TranscriptSource for AgentSessionTranscript {
    fn message_text(&self, conversation_id: &str, message_id: &str) -> Option<String> {
        let sequence: i64 = message_id.parse().ok()?;
        self.storage
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT payload_json FROM agent_events
                         WHERE session_id = ?1 AND sequence = ?2",
                        params![conversation_id, sequence],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
            })
            .ok()
            .flatten()
    }
}

// --- service ------------------------------------------------------------

pub struct MemoryService {
    storage: Arc<Storage>,
    transcript: Option<Arc<dyn TranscriptSource>>,
}

impl MemoryService {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            storage,
            transcript: None,
        }
    }

    /// Builds a service that verifies every proposal's excerpt against the
    /// turn it claims to come from.
    #[must_use]
    pub fn with_transcript(storage: Arc<Storage>, transcript: Arc<dyn TranscriptSource>) -> Self {
        Self {
            storage,
            transcript: Some(transcript),
        }
    }

    /// `memory.propose` — queue a candidate. Never applied, never retrieved.
    pub fn propose(&self, input: NewProposal) -> Result<MemoryProposal, MemoryError> {
        let text = validate_text(&input.text)?;
        let quoted = validate_quoted(&input.quoted_text)?;
        require_non_empty(&input.conversation_id, "conversationId")?;
        require_non_empty(&input.message_id, "messageId")?;
        require_non_empty(&input.proposed_by, "proposedBy")?;
        if !(0.0..=1.0).contains(&input.confidence) {
            return Err(MemoryError::ConfidenceOutOfRange(input.confidence));
        }

        // A task-scoped memory carries its repository too, so that scope
        // overlap can be resolved later without a second lookup.
        let mut repository_id = input.repository_id.clone();
        if let Some(task_id) = &input.task_id {
            let task_repository = self.task_repository(task_id)?;
            repository_id = repository_id.or(Some(task_repository));
        }
        match input.scope {
            MemoryScope::Task if input.task_id.is_none() => {
                return Err(MemoryError::ScopeIncomplete {
                    scope: "task",
                    field: "taskId",
                });
            }
            MemoryScope::Repo if repository_id.is_none() => {
                return Err(MemoryError::ScopeIncomplete {
                    scope: "repo",
                    field: "repositoryId",
                });
            }
            _ => {}
        }

        let verified = match &self.transcript {
            Some(transcript) => {
                let message = transcript
                    .message_text(&input.conversation_id, &input.message_id)
                    .ok_or_else(|| MemoryError::OriginNotFound {
                        conversation_id: input.conversation_id.clone(),
                        message_id: input.message_id.clone(),
                    })?;
                if !message.contains(quoted.as_str()) {
                    return Err(MemoryError::OriginNotFound {
                        conversation_id: input.conversation_id.clone(),
                        message_id: input.message_id.clone(),
                    });
                }
                true
            }
            None => false,
        };

        let id = Uuid::now_v7().to_string();
        let now = now_utc()?;
        self.storage.with_transaction(|transaction| {
            transaction.execute(
                "INSERT INTO memory_proposals
                    (id, project_id, repository_id, task_id, scope, kind, text, confidence,
                     origin_conversation_id, origin_message_id, origin_quoted_text,
                     origin_verified, proposed_by, proposed_at, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 'pending')",
                params![
                    id,
                    input.project_id,
                    repository_id,
                    input.task_id,
                    input.scope.as_str(),
                    input.kind.as_str(),
                    text,
                    input.confidence,
                    input.conversation_id,
                    input.message_id,
                    quoted,
                    verified as i64,
                    input.proposed_by,
                    now,
                ],
            )?;
            insert_event(
                transaction,
                &EventInput {
                    proposal_id: Some(&id),
                    memory_id: None,
                    kind: MemoryEventKind::Proposed,
                    actor: &input.proposed_by,
                    conversation_id: &input.conversation_id,
                    message_id: &input.message_id,
                    detail: "proposed from the quoted turn; awaiting human approval",
                    occurred_at: &now,
                },
            )
        })?;
        self.proposal(&id)
    }

    /// `memory.pending` — the approval queue.
    pub fn pending(&self, filters: &ProposalFilters) -> Result<Vec<MemoryProposal>, MemoryError> {
        let mut sql = String::from(
            "SELECT id, project_id, repository_id, task_id, scope, kind, text, confidence,
                    origin_conversation_id, origin_message_id, origin_quoted_text,
                    origin_verified, proposed_by, proposed_at, status, resolved_by,
                    resolved_at, rejection_reason, memory_id
             FROM memory_proposals WHERE project_id = ?1",
        );
        let mut arguments: Vec<Box<dyn ToSql>> = vec![Box::new(filters.project_id.clone())];
        if let Some(repository_id) = &filters.repository_id {
            arguments.push(Box::new(repository_id.clone()));
            sql.push_str(&format!(" AND repository_id = ?{}", arguments.len()));
        }
        if let Some(task_id) = &filters.task_id {
            arguments.push(Box::new(task_id.clone()));
            sql.push_str(&format!(" AND task_id = ?{}", arguments.len()));
        }
        let statuses = if filters.statuses.is_empty() {
            vec![ProposalStatus::Pending]
        } else {
            filters.statuses.clone()
        };
        let placeholders = statuses
            .iter()
            .map(|status| {
                arguments.push(Box::new(status.as_str().to_owned()));
                format!("?{}", arguments.len())
            })
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" AND status IN ({placeholders})"));
        arguments.push(Box::new(
            filters.limit.unwrap_or(MAX_LIST_LIMIT).min(MAX_LIST_LIMIT) as i64,
        ));
        sql.push_str(&format!(
            " ORDER BY proposed_at, id LIMIT ?{}",
            arguments.len()
        ));

        let rows = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(&sql)?;
            statement
                .query_map(
                    params_from_iter(arguments.iter().map(AsRef::as_ref)),
                    StoredProposal::from_row,
                )?
                .collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.into_iter()
            .map(StoredProposal::into_proposal)
            .collect()
    }

    /// `memory.approve` — a human accepts, edits and accepts, or is shown a
    /// conflict to resolve first. This is the only path that writes a memory.
    pub fn approve(&self, request: &ApproveRequest) -> Result<ApprovalOutcome, MemoryError> {
        require_non_empty(&request.approved_by, "approvedBy")?;
        let proposal = self.proposal(&request.proposal_id)?;
        if proposal.status != ProposalStatus::Pending {
            return Err(MemoryError::ProposalNotPending {
                id: proposal.id.clone(),
                status: proposal.status.as_str().to_owned(),
            });
        }
        let edited = request.edited_text.is_some();
        let text = match &request.edited_text {
            Some(text) => validate_text(text)?,
            None => proposal.text.clone(),
        };
        let vector = embedding::embed(&text);

        // Conflicts are recomputed on every attempt. A resolution set that no
        // longer matches what is actually active is refused rather than partly
        // applied, the same revalidation rule ADR-021 applies to context packs.
        let conflicts = self.detect_conflicts(&proposal, &text, &vector)?;
        if conflicts.is_empty() && !request.resolutions.is_empty() {
            return Err(MemoryError::StaleConflictSet);
        }
        if !conflicts.is_empty() {
            if request.resolutions.is_empty() {
                return Ok(ApprovalOutcome::ConflictsRequireChoice { conflicts });
            }
            let detected: HashSet<&str> = conflicts
                .iter()
                .map(|conflict| conflict.other.id.as_str())
                .collect();
            let resolved: HashSet<&str> = request
                .resolutions
                .iter()
                .map(|resolution| resolution.other_memory_id.as_str())
                .collect();
            if detected != resolved {
                return Err(MemoryError::StaleConflictSet);
            }
        }

        let kept: Vec<String> = request
            .resolutions
            .iter()
            .filter(|resolution| resolution.choice == ConflictChoice::KeepExisting)
            .map(|resolution| resolution.other_memory_id.clone())
            .collect();
        if !kept.is_empty() {
            // The person preferred what is already remembered, so the proposal
            // is rejected rather than silently dropped.
            let proposal = self.reject(&RejectRequest {
                proposal_id: proposal.id.clone(),
                rejected_by: request.approved_by.clone(),
                reason: format!("an existing memory was kept instead: {}", kept.join(", ")),
            })?;
            return Ok(ApprovalOutcome::KeptExisting { proposal, kept });
        }

        let supersedes: Vec<&ConflictResolution> = request
            .resolutions
            .iter()
            .filter(|resolution| resolution.choice == ConflictChoice::SupersedeOther)
            .collect();
        if supersedes.len() > 1 {
            return Err(MemoryError::MultipleSupersedes);
        }
        let supersedes_id = supersedes
            .first()
            .map(|resolution| resolution.other_memory_id.clone());

        let memory_id = Uuid::now_v7().to_string();
        let now = now_utc()?;
        let encoded = embedding::encode(&vector);
        let event_kind = if edited {
            MemoryEventKind::EditedAndApproved
        } else {
            MemoryEventKind::Approved
        };
        // The approval is a new turn in the conversation the proposal came
        // from, so a transcript view can render it where it belongs.
        let approval_message_id = Uuid::now_v7().to_string();
        let resolutions = request.resolutions.clone();
        let conflict_index: HashMap<&str, &DetectedConflict> = conflicts
            .iter()
            .map(|conflict| (conflict.other.id.as_str(), conflict))
            .collect();

        self.storage.with_transaction(|transaction| {
            transaction.execute(
                "INSERT INTO memories
                    (id, project_id, repository_id, task_id, scope, kind, text, confidence,
                     origin_conversation_id, origin_message_id, origin_quoted_text,
                     origin_verified, proposal_id, approved_by, approved_at, supersedes_id,
                     status, embedding, embedding_model, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                         ?16, 'active', ?17, ?18, ?15, ?15)",
                params![
                    memory_id,
                    proposal.project_id,
                    proposal.repository_id,
                    proposal.task_id,
                    proposal.scope.as_str(),
                    proposal.kind.as_str(),
                    text,
                    proposal.confidence,
                    proposal.origin.conversation_id,
                    proposal.origin.message_id,
                    proposal.origin.quoted_text,
                    proposal.origin.verified as i64,
                    proposal.id,
                    request.approved_by,
                    now,
                    supersedes_id,
                    encoded,
                    embedding::MODEL,
                ],
            )?;
            transaction.execute(
                "UPDATE memory_proposals
                 SET status = 'approved', resolved_by = ?2, resolved_at = ?3, memory_id = ?4
                 WHERE id = ?1",
                params![proposal.id, request.approved_by, now, memory_id],
            )?;
            insert_event(
                transaction,
                &EventInput {
                    proposal_id: Some(&proposal.id),
                    memory_id: Some(&memory_id),
                    kind: event_kind,
                    actor: &request.approved_by,
                    conversation_id: &proposal.origin.conversation_id,
                    message_id: &approval_message_id,
                    detail: if edited {
                        "edited, then approved by a human"
                    } else {
                        "approved by a human"
                    },
                    occurred_at: &now,
                },
            )?;

            for resolution in &resolutions {
                let Some(conflict) = conflict_index.get(resolution.other_memory_id.as_str()) else {
                    continue;
                };
                match resolution.choice {
                    ConflictChoice::SupersedeOther => {
                        transaction.execute(
                            "UPDATE memories SET status = 'superseded', updated_at = ?2
                             WHERE id = ?1",
                            params![resolution.other_memory_id, now],
                        )?;
                        insert_event(
                            transaction,
                            &EventInput {
                                proposal_id: None,
                                memory_id: Some(&resolution.other_memory_id),
                                kind: MemoryEventKind::Superseded,
                                actor: &request.approved_by,
                                conversation_id: &proposal.origin.conversation_id,
                                message_id: &approval_message_id,
                                detail: &format!("superseded by memory {memory_id}"),
                                occurred_at: &now,
                            },
                        )?;
                    }
                    ConflictChoice::RevokeOther => {
                        transaction.execute(
                            "UPDATE memories
                             SET status = 'revoked', revoked_by = ?2, revoked_at = ?3,
                                 revocation_reason = ?4, updated_at = ?3
                             WHERE id = ?1",
                            params![
                                resolution.other_memory_id,
                                request.approved_by,
                                now,
                                format!("withdrawn when memory {memory_id} was approved"),
                            ],
                        )?;
                        insert_event(
                            transaction,
                            &EventInput {
                                proposal_id: None,
                                memory_id: Some(&resolution.other_memory_id),
                                kind: MemoryEventKind::Revoked,
                                actor: &request.approved_by,
                                conversation_id: &proposal.origin.conversation_id,
                                message_id: &approval_message_id,
                                detail: &format!("revoked when memory {memory_id} was approved"),
                                occurred_at: &now,
                            },
                        )?;
                    }
                    ConflictChoice::KeepBoth | ConflictChoice::KeepExisting => {}
                }
                transaction.execute(
                    "INSERT INTO memory_conflicts
                        (id, memory_id, other_memory_id, similarity, signal, resolution,
                         resolved_by, resolved_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        Uuid::now_v7().to_string(),
                        memory_id,
                        resolution.other_memory_id,
                        conflict.similarity,
                        conflict.signal.as_str(),
                        resolution.choice.as_resolution(),
                        request.approved_by,
                        now,
                    ],
                )?;
                insert_event(
                    transaction,
                    &EventInput {
                        proposal_id: None,
                        memory_id: Some(&memory_id),
                        kind: MemoryEventKind::ConflictResolved,
                        actor: &request.approved_by,
                        conversation_id: &proposal.origin.conversation_id,
                        message_id: &approval_message_id,
                        detail: &format!(
                            "conflict with {} resolved as {}",
                            resolution.other_memory_id,
                            resolution.choice.as_resolution()
                        ),
                        occurred_at: &now,
                    },
                )?;
            }
            Ok(())
        })?;

        let memory = self.get(&memory_id)?;
        let message = self.approval_message(&memory_id, &approval_message_id)?;
        Ok(ApprovalOutcome::Approved { memory, message })
    }

    /// `memory.reject` — the proposal is closed and never becomes a memory.
    pub fn reject(&self, request: &RejectRequest) -> Result<MemoryProposal, MemoryError> {
        require_non_empty(&request.rejected_by, "rejectedBy")?;
        require_non_empty(&request.reason, "reason")?;
        let proposal = self.proposal(&request.proposal_id)?;
        if proposal.status != ProposalStatus::Pending {
            return Err(MemoryError::ProposalNotPending {
                id: proposal.id.clone(),
                status: proposal.status.as_str().to_owned(),
            });
        }
        let now = now_utc()?;
        self.storage.with_transaction(|transaction| {
            transaction.execute(
                "UPDATE memory_proposals
                 SET status = 'rejected', resolved_by = ?2, resolved_at = ?3,
                     rejection_reason = ?4
                 WHERE id = ?1",
                params![proposal.id, request.rejected_by, now, request.reason],
            )?;
            insert_event(
                transaction,
                &EventInput {
                    proposal_id: Some(&proposal.id),
                    memory_id: None,
                    kind: MemoryEventKind::Rejected,
                    actor: &request.rejected_by,
                    conversation_id: &proposal.origin.conversation_id,
                    message_id: &Uuid::now_v7().to_string(),
                    detail: &request.reason,
                    occurred_at: &now,
                },
            )
        })?;
        self.proposal(&proposal.id)
    }

    /// Editing an approved memory creates a new version linked by
    /// `supersedes`. The old text stays readable, because the record of what
    /// was once believed is part of the audit trail.
    pub fn revise(&self, request: &ReviseRequest) -> Result<Memory, MemoryError> {
        require_non_empty(&request.revised_by, "revisedBy")?;
        let text = validate_text(&request.text)?;
        let existing = self.get(&request.memory_id)?;
        if existing.status != MemoryStatus::Active {
            return Err(MemoryError::MemoryNotActive {
                id: existing.id.clone(),
                status: existing.status.as_str().to_owned(),
                action: "revised",
            });
        }
        let kind = request.kind.unwrap_or(existing.kind);
        let vector = embedding::embed(&text);
        let encoded = embedding::encode(&vector);
        let id = Uuid::now_v7().to_string();
        let now = now_utc()?;
        let message_id = Uuid::now_v7().to_string();

        self.storage.with_transaction(|transaction| {
            transaction.execute(
                "INSERT INTO memories
                    (id, project_id, repository_id, task_id, scope, kind, text, confidence,
                     origin_conversation_id, origin_message_id, origin_quoted_text,
                     origin_verified, proposal_id, approved_by, approved_at, supersedes_id,
                     status, embedding, embedding_model, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                         ?16, 'active', ?17, ?18, ?15, ?15)",
                params![
                    id,
                    existing.project_id,
                    existing.repository_id,
                    existing.task_id,
                    existing.scope.as_str(),
                    kind.as_str(),
                    text,
                    existing.confidence,
                    existing.origin.conversation_id,
                    existing.origin.message_id,
                    existing.origin.quoted_text,
                    existing.origin.verified as i64,
                    existing.proposal_id,
                    request.revised_by,
                    now,
                    existing.id,
                    encoded,
                    embedding::MODEL,
                ],
            )?;
            transaction.execute(
                "UPDATE memories SET status = 'superseded', updated_at = ?2 WHERE id = ?1",
                params![existing.id, now],
            )?;
            insert_event(
                transaction,
                &EventInput {
                    proposal_id: None,
                    memory_id: Some(&id),
                    kind: MemoryEventKind::Revised,
                    actor: &request.revised_by,
                    conversation_id: &existing.origin.conversation_id,
                    message_id: &message_id,
                    detail: &format!("revised from memory {}", existing.id),
                    occurred_at: &now,
                },
            )?;
            insert_event(
                transaction,
                &EventInput {
                    proposal_id: None,
                    memory_id: Some(&existing.id),
                    kind: MemoryEventKind::Superseded,
                    actor: &request.revised_by,
                    conversation_id: &existing.origin.conversation_id,
                    message_id: &message_id,
                    detail: &format!("superseded by memory {id}"),
                    occurred_at: &now,
                },
            )
        })?;
        self.get(&id)
    }

    /// Withdraws a memory from retrieval while keeping it for audit. There is
    /// no hard delete.
    pub fn revoke(&self, request: &RevokeRequest) -> Result<Memory, MemoryError> {
        require_non_empty(&request.revoked_by, "revokedBy")?;
        require_non_empty(&request.reason, "reason")?;
        let existing = self.get(&request.memory_id)?;
        if existing.status == MemoryStatus::Revoked {
            return Err(MemoryError::MemoryNotActive {
                id: existing.id.clone(),
                status: existing.status.as_str().to_owned(),
                action: "revoked again",
            });
        }
        let now = now_utc()?;
        self.storage.with_transaction(|transaction| {
            transaction.execute(
                "UPDATE memories
                 SET status = 'revoked', revoked_by = ?2, revoked_at = ?3,
                     revocation_reason = ?4, updated_at = ?3
                 WHERE id = ?1",
                params![existing.id, request.revoked_by, now, request.reason],
            )?;
            insert_event(
                transaction,
                &EventInput {
                    proposal_id: None,
                    memory_id: Some(&existing.id),
                    kind: MemoryEventKind::Revoked,
                    actor: &request.revoked_by,
                    conversation_id: &existing.origin.conversation_id,
                    message_id: &Uuid::now_v7().to_string(),
                    detail: &request.reason,
                    occurred_at: &now,
                },
            )
        })?;
        self.get(&existing.id)
    }

    /// `memory.list(filters)`.
    pub fn list(&self, filters: &MemoryFilters) -> Result<Vec<Memory>, MemoryError> {
        let mut sql = String::from(
            "SELECT m.id, m.project_id, m.repository_id, m.task_id, m.scope, m.kind, m.text,
                    m.confidence, m.origin_conversation_id, m.origin_message_id,
                    m.origin_quoted_text, m.origin_verified, m.proposal_id, m.approved_by,
                    m.approved_at, m.supersedes_id, m.status, m.revoked_by, m.revoked_at,
                    m.revocation_reason, m.created_at, m.updated_at,
                    (SELECT COUNT(*) FROM memory_uses u WHERE u.memory_id = m.id),
                    (SELECT COUNT(DISTINCT u.task_id) FROM memory_uses u
                      WHERE u.memory_id = m.id AND u.task_id IS NOT NULL)
             FROM memories m WHERE m.project_id = ?1",
        );
        let mut arguments: Vec<Box<dyn ToSql>> = vec![Box::new(filters.project_id.clone())];
        if let Some(repository_id) = &filters.repository_id {
            arguments.push(Box::new(repository_id.clone()));
            sql.push_str(&format!(" AND m.repository_id = ?{}", arguments.len()));
        }
        if let Some(task_id) = &filters.task_id {
            arguments.push(Box::new(task_id.clone()));
            sql.push_str(&format!(" AND m.task_id = ?{}", arguments.len()));
        }
        if !filters.scopes.is_empty() {
            let placeholders = filters
                .scopes
                .iter()
                .map(|scope| {
                    arguments.push(Box::new(scope.as_str().to_owned()));
                    format!("?{}", arguments.len())
                })
                .collect::<Vec<_>>()
                .join(", ");
            sql.push_str(&format!(" AND m.scope IN ({placeholders})"));
        }
        if !filters.kinds.is_empty() {
            let placeholders = filters
                .kinds
                .iter()
                .map(|kind| {
                    arguments.push(Box::new(kind.as_str().to_owned()));
                    format!("?{}", arguments.len())
                })
                .collect::<Vec<_>>()
                .join(", ");
            sql.push_str(&format!(" AND m.kind IN ({placeholders})"));
        }
        let statuses = if filters.statuses.is_empty() {
            vec![MemoryStatus::Active]
        } else {
            filters.statuses.clone()
        };
        let placeholders = statuses
            .iter()
            .map(|status| {
                arguments.push(Box::new(status.as_str().to_owned()));
                format!("?{}", arguments.len())
            })
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" AND m.status IN ({placeholders})"));
        if let Some(contains) = &filters.contains {
            arguments.push(Box::new(format!("%{}%", contains.trim())));
            sql.push_str(&format!(" AND m.text LIKE ?{}", arguments.len()));
        }
        arguments.push(Box::new(
            filters.limit.unwrap_or(MAX_LIST_LIMIT).min(MAX_LIST_LIMIT) as i64,
        ));
        sql.push_str(&format!(
            " ORDER BY m.approved_at DESC, m.id LIMIT ?{}",
            arguments.len()
        ));

        let rows = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(&sql)?;
            statement
                .query_map(
                    params_from_iter(arguments.iter().map(AsRef::as_ref)),
                    StoredMemory::from_row,
                )?
                .collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.into_iter().map(StoredMemory::into_memory).collect()
    }

    /// `memory.forRetrieval(context)` — select memories for an agent and log
    /// that they were injected.
    ///
    /// Only active memories are eligible. Selection is by scope, then by
    /// embedding relevance to what the agent is about to do, with a bonus for
    /// the narrower scope so a rule written about this task outranks an equally
    /// similar rule written about everything.
    pub fn for_retrieval(
        &self,
        context: &RetrievalContext,
    ) -> Result<Vec<RetrievedMemory>, MemoryError> {
        let limit = context
            .limit
            .unwrap_or(DEFAULT_RETRIEVAL_LIMIT)
            .clamp(1, MAX_RETRIEVAL_LIMIT);
        let eligible = self.eligible_for(context)?;
        let query = embedding::embed(&context.query);

        let mut scored: Vec<(Memory, f32, f32)> = eligible
            .into_iter()
            .map(|(memory, vector)| {
                let similarity = embedding::similarity(&query, &vector);
                let relevance = (similarity + memory.scope.specificity_bonus()).clamp(0.0, 1.0);
                (memory, similarity, relevance)
            })
            .filter(|(_, _, relevance)| *relevance >= RETRIEVAL_FLOOR)
            .collect();
        scored.sort_by(|left, right| {
            right
                .2
                .total_cmp(&left.2)
                .then_with(|| left.0.id.cmp(&right.0.id))
        });
        scored.truncate(limit);

        let now = now_utc()?;
        if context.record_uses && !scored.is_empty() {
            let rows: Vec<(String, f64)> = scored
                .iter()
                .map(|(memory, _, relevance)| (memory.id.clone(), f64::from(*relevance)))
                .collect();
            self.storage.with_transaction(|transaction| {
                for (memory_id, relevance) in &rows {
                    transaction.execute(
                        "INSERT INTO memory_uses
                            (id, memory_id, task_id, session_id, surface, relevance, used_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            Uuid::now_v7().to_string(),
                            memory_id,
                            context.task_id,
                            context.session_id,
                            context.surface,
                            relevance,
                            now,
                        ],
                    )?;
                }
                Ok(())
            })?;
        }

        Ok(scored
            .into_iter()
            .map(|(memory, similarity, relevance)| RetrievedMemory {
                reason: format!(
                    "{} memory, {:.2} similarity to the current work{}",
                    memory.scope.as_str(),
                    similarity,
                    match memory.scope {
                        MemoryScope::Global => String::new(),
                        scope => format!(
                            ", preferred by {:.2} for its narrower scope",
                            scope.specificity_bonus()
                        ),
                    }
                ),
                memory,
                relevance: f64::from(relevance),
            })
            .collect())
    }

    /// Records that these memories were injected, after the fact.
    ///
    /// [`Self::for_retrieval`] logs its own selection because calling it *is*
    /// the injection. A context pack is different: it is built to be previewed
    /// and rebuilt to be revalidated before anything reaches an agent, so the
    /// injection happens later and somewhere else. This records exactly what
    /// was sent, from the pack that was sent, rather than re-deriving a
    /// selection that may have moved on.
    pub fn record_uses(
        &self,
        injections: &[MemoryInjection],
        context: &InjectionContext,
    ) -> Result<usize, MemoryError> {
        if injections.is_empty() {
            return Ok(0);
        }
        require_non_empty(&context.surface, "surface")?;
        let now = now_utc()?;
        self.storage.with_transaction(|transaction| {
            for injection in injections {
                transaction.execute(
                    "INSERT INTO memory_uses
                        (id, memory_id, task_id, session_id, surface, relevance, used_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        Uuid::now_v7().to_string(),
                        injection.memory_id,
                        context.task_id,
                        context.session_id,
                        context.surface,
                        injection.relevance.clamp(0.0, 1.0),
                        now,
                    ],
                )?;
            }
            Ok(())
        })?;
        Ok(injections.len())
    }

    /// `memory.usage(id)` — what this memory has actually shaped.
    pub fn usage(&self, memory_id: &str) -> Result<MemoryUsage, MemoryError> {
        let memory = self.get(memory_id)?;
        let uses = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, memory_id, task_id, session_id, surface, relevance, used_at
                 FROM memory_uses WHERE memory_id = ?1 ORDER BY used_at DESC, id DESC",
            )?;
            statement
                .query_map([memory_id], |row| {
                    Ok(MemoryUse {
                        id: row.get(0)?,
                        memory_id: row.get(1)?,
                        task_id: row.get(2)?,
                        session_id: row.get(3)?,
                        surface: row.get(4)?,
                        relevance: row.get(5)?,
                        used_at: row.get(6)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })?;
        Ok(MemoryUsage {
            memory_id: memory.id,
            use_count: memory.use_count,
            used_in_tasks: memory.used_in_tasks,
            uses,
        })
    }

    /// The full audit trail for one memory, oldest first.
    pub fn history(&self, memory_id: &str) -> Result<Vec<MemoryEvent>, MemoryError> {
        let rows = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT e.id, e.proposal_id, e.memory_id, e.kind, e.actor, e.conversation_id,
                        e.message_id, e.detail, e.occurred_at
                 FROM memory_events e
                 WHERE e.memory_id = ?1
                    OR e.proposal_id = (SELECT proposal_id FROM memories WHERE id = ?1)
                 ORDER BY e.occurred_at, e.id",
            )?;
            statement
                .query_map([memory_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.into_iter()
            .map(|row| {
                Ok(MemoryEvent {
                    id: row.0,
                    proposal_id: row.1,
                    memory_id: row.2,
                    kind: MemoryEventKind::parse(&row.3)?,
                    actor: row.4,
                    conversation_id: row.5,
                    message_id: row.6,
                    detail: row.7,
                    occurred_at: row.8,
                })
            })
            .collect()
    }

    pub fn get(&self, memory_id: &str) -> Result<Memory, MemoryError> {
        let row = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT m.id, m.project_id, m.repository_id, m.task_id, m.scope, m.kind,
                            m.text, m.confidence, m.origin_conversation_id, m.origin_message_id,
                            m.origin_quoted_text, m.origin_verified, m.proposal_id, m.approved_by,
                            m.approved_at, m.supersedes_id, m.status, m.revoked_by, m.revoked_at,
                            m.revocation_reason, m.created_at, m.updated_at,
                            (SELECT COUNT(*) FROM memory_uses u WHERE u.memory_id = m.id),
                            (SELECT COUNT(DISTINCT u.task_id) FROM memory_uses u
                              WHERE u.memory_id = m.id AND u.task_id IS NOT NULL)
                     FROM memories m WHERE m.id = ?1",
                    [memory_id],
                    StoredMemory::from_row,
                )
                .optional()
        })?;
        row.ok_or_else(|| MemoryError::MemoryNotFound(memory_id.into()))?
            .into_memory()
    }

    pub fn proposal(&self, proposal_id: &str) -> Result<MemoryProposal, MemoryError> {
        let row = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, project_id, repository_id, task_id, scope, kind, text, confidence,
                            origin_conversation_id, origin_message_id, origin_quoted_text,
                            origin_verified, proposed_by, proposed_at, status, resolved_by,
                            resolved_at, rejection_reason, memory_id
                     FROM memory_proposals WHERE id = ?1",
                    [proposal_id],
                    StoredProposal::from_row,
                )
                .optional()
        })?;
        row.ok_or_else(|| MemoryError::ProposalNotFound(proposal_id.into()))?
            .into_proposal()
    }

    /// Active memories whose scope co-applies with the context, paired with
    /// their stored vectors.
    fn eligible_for(
        &self,
        context: &RetrievalContext,
    ) -> Result<Vec<(Memory, Vec<f32>)>, MemoryError> {
        let rows = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT m.id, m.project_id, m.repository_id, m.task_id, m.scope, m.kind, m.text,
                        m.confidence, m.origin_conversation_id, m.origin_message_id,
                        m.origin_quoted_text, m.origin_verified, m.proposal_id, m.approved_by,
                        m.approved_at, m.supersedes_id, m.status, m.revoked_by, m.revoked_at,
                        m.revocation_reason, m.created_at, m.updated_at,
                        (SELECT COUNT(*) FROM memory_uses u WHERE u.memory_id = m.id),
                        (SELECT COUNT(DISTINCT u.task_id) FROM memory_uses u
                          WHERE u.memory_id = m.id AND u.task_id IS NOT NULL),
                        m.embedding
                 FROM memories m
                 WHERE m.project_id = ?1 AND m.status = 'active'
                   AND (m.scope = 'global'
                        OR (m.scope = 'repo' AND m.repository_id = ?2)
                        OR (m.scope = 'task' AND m.task_id = ?3))",
            )?;
            statement
                .query_map(
                    params![context.project_id, context.repository_id, context.task_id],
                    |row| Ok((StoredMemory::from_row(row)?, row.get::<_, Vec<u8>>(24)?)),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.into_iter()
            .map(|(raw, blob)| {
                let memory = raw.into_memory()?;
                let vector = embedding::decode(&blob).unwrap_or_default();
                Ok((memory, vector))
            })
            .collect()
    }

    /// Active memories whose scope could apply anywhere the proposal's scope
    /// applies, paired with their vectors.
    ///
    /// This is scope *overlap*, not scope equality, because a conflict only
    /// matters where two rules both hold. A global rule and a repository rule
    /// both apply inside that repository, so they can disagree; two rules
    /// scoped to different repositories never meet and cannot.
    fn co_applicable(
        &self,
        proposal: &MemoryProposal,
    ) -> Result<Vec<(Memory, Vec<f32>)>, MemoryError> {
        let rows = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT m.id, m.project_id, m.repository_id, m.task_id, m.scope, m.kind, m.text,
                        m.confidence, m.origin_conversation_id, m.origin_message_id,
                        m.origin_quoted_text, m.origin_verified, m.proposal_id, m.approved_by,
                        m.approved_at, m.supersedes_id, m.status, m.revoked_by, m.revoked_at,
                        m.revocation_reason, m.created_at, m.updated_at,
                        (SELECT COUNT(*) FROM memory_uses u WHERE u.memory_id = m.id),
                        (SELECT COUNT(DISTINCT u.task_id) FROM memory_uses u
                          WHERE u.memory_id = m.id AND u.task_id IS NOT NULL),
                        m.embedding
                 FROM memories m
                 WHERE m.project_id = ?1 AND m.status = 'active'
                   AND (
                        m.scope = 'global' OR ?4 = 'global'
                        OR (m.repository_id = ?2
                            AND (m.scope = 'repo' OR ?4 = 'repo' OR m.task_id = ?3))
                   )",
            )?;
            statement
                .query_map(
                    params![
                        proposal.project_id,
                        proposal.repository_id,
                        proposal.task_id,
                        proposal.scope.as_str(),
                    ],
                    |row| Ok((StoredMemory::from_row(row)?, row.get::<_, Vec<u8>>(24)?)),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.into_iter()
            .map(|(raw, blob)| {
                let memory = raw.into_memory()?;
                let vector = embedding::decode(&blob).unwrap_or_default();
                Ok((memory, vector))
            })
            .collect()
    }

    /// Active memories that could apply wherever the proposal would, similar
    /// enough that a person should look at them together.
    fn detect_conflicts(
        &self,
        proposal: &MemoryProposal,
        text: &str,
        vector: &[f32],
    ) -> Result<Vec<DetectedConflict>, MemoryError> {
        let candidates = self.co_applicable(proposal)?;

        let negated = is_negated(text);
        let mut conflicts: Vec<DetectedConflict> = candidates
            .into_iter()
            .filter_map(|(memory, other_vector)| {
                let similarity = embedding::similarity(vector, &other_vector);
                if similarity < CONFLICT_SIMILARITY {
                    return None;
                }
                let signal = if is_negated(&memory.text) == negated {
                    ConflictSignal::Overlap
                } else {
                    ConflictSignal::OpposingPolarity
                };
                let explanation = match signal {
                    ConflictSignal::OpposingPolarity => format!(
                        "This says nearly the same thing as an active {} memory ({:.0}% similar), \
                         but one of the two is phrased as a prohibition and the other is not.",
                        memory.scope.as_str(),
                        similarity * 100.0
                    ),
                    ConflictSignal::Overlap => format!(
                        "This overlaps an active {} memory ({:.0}% similar). They may both be \
                         true, or the new one may be meant to replace it.",
                        memory.scope.as_str(),
                        similarity * 100.0
                    ),
                };
                Some(DetectedConflict {
                    other: memory,
                    similarity: f64::from(similarity),
                    signal,
                    explanation,
                })
            })
            .collect();
        // Strongest signal first: a polarity clash needs a decision more
        // urgently than a restatement.
        conflicts.sort_by(|left, right| {
            right
                .signal
                .cmp(&left.signal)
                .then_with(|| right.similarity.total_cmp(&left.similarity))
                .then_with(|| left.other.id.cmp(&right.other.id))
        });
        Ok(conflicts)
    }

    fn approval_message(
        &self,
        memory_id: &str,
        message_id: &str,
    ) -> Result<MemoryEvent, MemoryError> {
        let row = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, proposal_id, memory_id, kind, actor, conversation_id, message_id,
                            detail, occurred_at
                     FROM memory_events
                     WHERE memory_id = ?1 AND message_id = ?2
                       AND kind IN ('approved', 'edited_and_approved')",
                    params![memory_id, message_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, String>(7)?,
                            row.get::<_, String>(8)?,
                        ))
                    },
                )
                .optional()
        })?;
        let row = row.ok_or_else(|| MemoryError::MemoryNotFound(memory_id.into()))?;
        Ok(MemoryEvent {
            id: row.0,
            proposal_id: row.1,
            memory_id: row.2,
            kind: MemoryEventKind::parse(&row.3)?,
            actor: row.4,
            conversation_id: row.5,
            message_id: row.6,
            detail: row.7,
            occurred_at: row.8,
        })
    }

    fn task_repository(&self, task_id: &str) -> Result<String, MemoryError> {
        let row: Option<String> = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT repository_id FROM tasks WHERE id = ?1",
                    [task_id],
                    |row| row.get(0),
                )
                .optional()
        })?;
        row.ok_or_else(|| MemoryError::TaskNotFound(task_id.into()))
    }
}

/// The prefix a context pack uses for a memory it carries.
const PACK_ENTITY_PREFIX: &str = "memory:";

/// Reads the memories a built context pack actually carries.
///
/// Pairing this with [`MemoryService::record_uses`] is what makes the usage
/// trail describe what an agent was really given: the pack that was sent is
/// the evidence, not a second guess at what retrieval would return now.
#[must_use]
pub fn injections_from_pack(pack: &crate::ContextPack) -> Vec<MemoryInjection> {
    pack.items
        .iter()
        .filter_map(|item| {
            item.entity_ref
                .strip_prefix(PACK_ENTITY_PREFIX)
                .map(|memory_id| MemoryInjection {
                    memory_id: memory_id.to_owned(),
                    relevance: item.score.clamp(0.0, 1.0),
                })
        })
        .collect()
}

/// Builds the `entity_ref` a context pack uses for a memory.
#[must_use]
pub(crate) fn pack_entity_ref(memory_id: &str) -> String {
    format!("{PACK_ENTITY_PREFIX}{memory_id}")
}

// --- helpers ------------------------------------------------------------

/// Words that turn a statement into a prohibition. Used only to decide whether
/// two similar memories deserve a stronger warning when a person reviews them;
/// nothing acts on this without a human choice.
const NEGATIONS: &[&str] = &[
    "avoid",
    "cannot",
    "dont",
    "except",
    "forbidden",
    "must not",
    "mustn't",
    "never",
    "no longer",
    "not",
    "prohibited",
    "refuse",
    "should not",
    "shouldn't",
    "stop",
    "without",
];

/// True when the text reads as a prohibition. Matching is on whole words, so
/// "notation" is not read as "not".
fn is_negated(text: &str) -> bool {
    let words: Vec<String> = text
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric() && character != '\'')
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect();
    NEGATIONS.iter().any(|marker| {
        let marker_words: Vec<&str> = marker.split(' ').collect();
        words.windows(marker_words.len()).any(|window| {
            window
                .iter()
                .zip(&marker_words)
                .all(|(word, expected)| word == expected)
        })
    })
}

struct EventInput<'a> {
    proposal_id: Option<&'a str>,
    memory_id: Option<&'a str>,
    kind: MemoryEventKind,
    actor: &'a str,
    conversation_id: &'a str,
    message_id: &'a str,
    detail: &'a str,
    occurred_at: &'a str,
}

fn insert_event(
    transaction: &rusqlite::Transaction<'_>,
    input: &EventInput<'_>,
) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT INTO memory_events
            (id, proposal_id, memory_id, kind, actor, conversation_id, message_id, detail,
             occurred_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            Uuid::now_v7().to_string(),
            input.proposal_id,
            input.memory_id,
            input.kind.as_str(),
            input.actor,
            input.conversation_id,
            input.message_id,
            input.detail,
            input.occurred_at,
        ],
    )?;
    Ok(())
}

fn validate_text(text: &str) -> Result<String, MemoryError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(MemoryError::Empty { field: "text" });
    }
    if trimmed.chars().count() > MAX_TEXT_CHARS {
        return Err(MemoryError::TooLong {
            field: "text",
            maximum: MAX_TEXT_CHARS,
        });
    }
    Ok(trimmed.to_owned())
}

fn validate_quoted(quoted: &str) -> Result<String, MemoryError> {
    let trimmed = quoted.trim();
    if trimmed.is_empty() {
        return Err(MemoryError::Empty {
            field: "quotedText",
        });
    }
    if trimmed.chars().count() > MAX_QUOTED_CHARS {
        return Err(MemoryError::TooLong {
            field: "quotedText",
            maximum: MAX_QUOTED_CHARS,
        });
    }
    Ok(trimmed.to_owned())
}

fn require_non_empty(value: &str, field: &'static str) -> Result<(), MemoryError> {
    if value.trim().is_empty() {
        return Err(MemoryError::Empty { field });
    }
    Ok(())
}

fn now_utc() -> Result<String, MemoryError> {
    Ok(OffsetDateTime::now_utc().format(&Rfc3339)?)
}

/// One `memories` row as stored, with the enumerated columns still as text.
/// Parsing them is fallible in a way `rusqlite` cannot express, so reading and
/// interpreting a row are two steps.
struct StoredMemory {
    id: String,
    project_id: String,
    repository_id: Option<String>,
    task_id: Option<String>,
    scope: String,
    kind: String,
    text: String,
    confidence: f64,
    conversation_id: String,
    message_id: String,
    quoted_text: String,
    origin_verified: i64,
    proposal_id: String,
    approved_by: String,
    approved_at: String,
    supersedes_id: Option<String>,
    status: String,
    revoked_by: Option<String>,
    revoked_at: Option<String>,
    revocation_reason: Option<String>,
    created_at: String,
    updated_at: String,
    use_count: i64,
    used_in_tasks: i64,
}

impl StoredMemory {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            project_id: row.get(1)?,
            repository_id: row.get(2)?,
            task_id: row.get(3)?,
            scope: row.get(4)?,
            kind: row.get(5)?,
            text: row.get(6)?,
            confidence: row.get(7)?,
            conversation_id: row.get(8)?,
            message_id: row.get(9)?,
            quoted_text: row.get(10)?,
            origin_verified: row.get(11)?,
            proposal_id: row.get(12)?,
            approved_by: row.get(13)?,
            approved_at: row.get(14)?,
            supersedes_id: row.get(15)?,
            status: row.get(16)?,
            revoked_by: row.get(17)?,
            revoked_at: row.get(18)?,
            revocation_reason: row.get(19)?,
            created_at: row.get(20)?,
            updated_at: row.get(21)?,
            use_count: row.get(22)?,
            used_in_tasks: row.get(23)?,
        })
    }

    fn into_memory(self) -> Result<Memory, MemoryError> {
        Ok(Memory {
            scope: MemoryScope::parse(&self.scope)?,
            kind: MemoryKind::parse(&self.kind)?,
            status: MemoryStatus::parse(&self.status)?,
            origin: MemoryOrigin {
                conversation_id: self.conversation_id,
                message_id: self.message_id,
                quoted_text: self.quoted_text,
                verified: self.origin_verified != 0,
            },
            id: self.id,
            project_id: self.project_id,
            repository_id: self.repository_id,
            task_id: self.task_id,
            text: self.text,
            confidence: self.confidence,
            proposal_id: self.proposal_id,
            approved_by: self.approved_by,
            approved_at: self.approved_at,
            supersedes: self.supersedes_id,
            revoked_by: self.revoked_by,
            revoked_at: self.revoked_at,
            revocation_reason: self.revocation_reason,
            use_count: self.use_count.max(0) as usize,
            used_in_tasks: self.used_in_tasks.max(0) as usize,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

/// One `memory_proposals` row as stored.
struct StoredProposal {
    id: String,
    project_id: String,
    repository_id: Option<String>,
    task_id: Option<String>,
    scope: String,
    kind: String,
    text: String,
    confidence: f64,
    conversation_id: String,
    message_id: String,
    quoted_text: String,
    origin_verified: i64,
    proposed_by: String,
    proposed_at: String,
    status: String,
    resolved_by: Option<String>,
    resolved_at: Option<String>,
    rejection_reason: Option<String>,
    memory_id: Option<String>,
}

impl StoredProposal {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            project_id: row.get(1)?,
            repository_id: row.get(2)?,
            task_id: row.get(3)?,
            scope: row.get(4)?,
            kind: row.get(5)?,
            text: row.get(6)?,
            confidence: row.get(7)?,
            conversation_id: row.get(8)?,
            message_id: row.get(9)?,
            quoted_text: row.get(10)?,
            origin_verified: row.get(11)?,
            proposed_by: row.get(12)?,
            proposed_at: row.get(13)?,
            status: row.get(14)?,
            resolved_by: row.get(15)?,
            resolved_at: row.get(16)?,
            rejection_reason: row.get(17)?,
            memory_id: row.get(18)?,
        })
    }

    fn into_proposal(self) -> Result<MemoryProposal, MemoryError> {
        Ok(MemoryProposal {
            scope: MemoryScope::parse(&self.scope)?,
            kind: MemoryKind::parse(&self.kind)?,
            status: ProposalStatus::parse(&self.status)?,
            origin: MemoryOrigin {
                conversation_id: self.conversation_id,
                message_id: self.message_id,
                quoted_text: self.quoted_text,
                verified: self.origin_verified != 0,
            },
            id: self.id,
            project_id: self.project_id,
            repository_id: self.repository_id,
            task_id: self.task_id,
            text: self.text,
            confidence: self.confidence,
            proposed_by: self.proposed_by,
            proposed_at: self.proposed_at,
            resolved_by: self.resolved_by,
            resolved_at: self.resolved_at,
            rejection_reason: self.rejection_reason,
            memory_id: self.memory_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negation_markers_respect_word_boundaries() {
        assert!(is_negated("never commit generated files"));
        assert!(is_negated("do not use the legacy client"));
        assert!(is_negated("avoid the global mutex"));
        assert!(!is_negated("use notation consistently"));
        assert!(!is_negated("always run the formatter"));
    }

    #[test]
    fn negation_markers_stay_sorted_and_lowercase() {
        let mut sorted = NEGATIONS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, NEGATIONS, "NEGATIONS must stay sorted");
        assert!(
            NEGATIONS
                .iter()
                .all(|marker| marker.to_lowercase() == *marker)
        );
    }
}
