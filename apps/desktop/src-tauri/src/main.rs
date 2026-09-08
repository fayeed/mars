use std::fs;
use std::path::Path;
use std::sync::Arc;

use mars_agents::adapter::{
    AdapterError, AdapterKind, AdapterProbe, AgentAdapter, AgentLaunchRequest, JsonLineDecoder,
    NormalizedAgentEvent,
};
use mars_agents::claude::ClaudeAdapter;
use mars_agents::codex::CodexAdapter;
use mars_agents::process::{
    ProcessError, ProcessEventKind, ProcessSpec, ProcessSupervisor, SessionRecord,
};
use mars_brain::tools::{BrainEdge, BrainNeighborhood, BrainSymbol, Dependency};
use mars_brain::{
    ApprovalOutcome, ApproveRequest, AskError, AskRequest, BrainAnswer, BrainError, BrainEvent,
    BrainService, ContextPack, ContextPackError, ContextPackRequest, ContextPackService,
    DependencySlice, EntityKind, EntityRef, ExtractionError, ExtractionProvider, ExtractionRequest,
    ExtractionService, FindingStatus, InjectionContext, KnowledgeError, KnowledgeKind,
    KnowledgeRecord, KnowledgeService, Memory, MemoryCategory, MemoryError, MemoryEvent,
    MemoryFilters, MemoryProposal, MemoryService, MemoryUsage, NewKnowledge, NewProposal,
    NewProvenance, ProjectBrain, ProposalFilters, ProvenanceKind, RejectRequest, RetrievalContext,
    RetrievedMemory, ReviewError, ReviewHandoff, ReviewHandoffRequest, ReviewProvider,
    ReviewRecord, ReviewService, ReviseRequest, RevokeRequest, WhyAnswer, WhyError, WhyRequest,
    WhyService, injections_from_pack, render_context_pack, render_extraction_prompt,
};
use mars_context::{CaptureError, ContextItem, ContextService};
use mars_core::Core;
use mars_core::files::{FileDocument, FileError, FilePage, FileService};
use mars_core::projects::{OpenRepositoryOutcome, ProjectError, ProjectService};
use mars_core::protocol::{CommandRequest, CommandResponse};
use mars_core::terminal::{TerminalError, TerminalService, TerminalSessionInfo};
use mars_forge::{
    CredentialVault, ForgeConnection, ForgeError, ForgeMutation, ForgeProvider, ForgeWorkflow,
    OsCredentialVault, PullRequestPlan, PullRequestSnapshot, StoredIssue,
};
use mars_forge_github::GitHubProvider;
use mars_git::{
    OverlapError, OverlapWarning, TaskOverlapService, TaskWorktreeError, TaskWorktreeService,
    WorktreePlan,
};
use mars_indexer::{IndexError, IndexSummary, Indexer};
use mars_security::TrustDecision;
use mars_security::permissions::{
    Capability, PermissionAudit, PermissionDecision, PermissionError, PermissionOutcome,
    PermissionRequest, PermissionScope,
};
use mars_skills::{
    InspectedSkill, SkillCandidate, SkillError, SkillInstallation, SkillScope, SkillService,
    SkillsShRegistry,
};
use mars_storage::Storage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    agents: ProcessSupervisor,
    brain: BrainService,
    /// The Phase 1 answer pipeline. Distinct from `brain`, which serves the
    /// bounded structural queries of ADR-019.
    project_brain: ProjectBrain,
    /// Phase 2 human-approved memory.
    memory: MemoryService,
    context_packs: ContextPackService,
    context: ContextService,
    credentials: OsCredentialVault,
    files: FileService,
    forge: ForgeWorkflow,
    indexer: Indexer,
    knowledge: KnowledgeService,
    overlaps: TaskOverlapService,
    projects: ProjectService,
    reviews: ReviewService,
    permissions: PermissionAudit,
    skills: SkillService,
    tasks: TaskWorktreeService,
    terminals: TerminalService,
    why: WhyService,
    storage: Arc<Storage>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommandFailure {
    code: &'static str,
    message: String,
}

impl CommandFailure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<ProjectError> for CommandFailure {
    fn from(error: ProjectError) -> Self {
        let code = match error {
            ProjectError::NotDirectory(_) => "repository.not_directory",
            ProjectError::NotGitRepository(_) => "repository.not_git",
            ProjectError::MalformedGitFile(_) => "repository.invalid_git_file",
            ProjectError::MissingName(_) => "repository.invalid_name",
            ProjectError::Filesystem { .. } => "repository.filesystem",
            ProjectError::Storage(_) => "repository.storage",
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

impl From<FileError> for CommandFailure {
    fn from(error: FileError) -> Self {
        let code = match error {
            FileError::Storage(_) => "file.storage",
            FileError::InvalidRelativePath(_) => "file.invalid_path",
            FileError::OutsideRepository(_) => "file.outside_repository",
            FileError::IgnoredPath(_) => "file.ignored",
            FileError::NotFile(_) => "file.not_file",
            FileError::BinaryFile(_) => "file.binary",
            FileError::FileTooLarge { .. } => "file.too_large",
            FileError::WriteConflict { .. } => "file.write_conflict",
            FileError::Walk(_) => "file.walk",
            FileError::Filesystem { .. } => "file.filesystem",
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

impl From<TerminalError> for CommandFailure {
    fn from(error: TerminalError) -> Self {
        let code = match error {
            TerminalError::Storage(_) => "terminal.storage",
            TerminalError::InvalidDimensions => "terminal.invalid_dimensions",
            TerminalError::SessionNotFound(_) => "terminal.not_found",
            TerminalError::Closed(_) => "terminal.closed",
            TerminalError::StatePoisoned(_) => "terminal.state",
            TerminalError::Pty(_) => "terminal.pty",
            TerminalError::Io(_) => "terminal.io",
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

impl From<IndexError> for CommandFailure {
    fn from(error: IndexError) -> Self {
        let code = match error {
            IndexError::Storage(_) => "index.storage",
            IndexError::Json(_) => "index.json",
            IndexError::Timestamp(_) => "index.timestamp",
            IndexError::Filesystem { .. } => "index.filesystem",
            IndexError::OutsideRepository(_) => "index.outside_repository",
            IndexError::Walk(_) => "index.walk",
            IndexError::Parse(_) => "index.parse",
            IndexError::Git(_) => "index.git",
            IndexError::Watch(_) => "index.watch",
            IndexError::Cancelled { .. } => "index.cancelled",
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

impl From<BrainError> for CommandFailure {
    fn from(error: BrainError) -> Self {
        let code = match error {
            BrainError::Storage(_) => "brain.storage",
            BrainError::Json(_) => "brain.evidence",
            BrainError::InvalidBounds { .. } => "brain.invalid_bounds",
            BrainError::SymbolNotFound(_) => "brain.symbol_not_found",
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

impl From<ContextPackError> for CommandFailure {
    fn from(error: ContextPackError) -> Self {
        let code = match error {
            ContextPackError::Storage(_) => "context_pack.storage",
            ContextPackError::InvalidBudget => "context_pack.invalid_budget",
            ContextPackError::Memory(_) => "context_pack.memory",
        };
        Self::new(code, error.to_string())
    }
}

impl From<KnowledgeError> for CommandFailure {
    fn from(error: KnowledgeError) -> Self {
        Self::new("knowledge.invalid", error.to_string())
    }
}

impl From<ExtractionError> for CommandFailure {
    fn from(error: ExtractionError) -> Self {
        Self::new("memory.extraction", error.to_string())
    }
}

impl From<WhyError> for CommandFailure {
    fn from(error: WhyError) -> Self {
        Self::new("memory.why", error.to_string())
    }
}

impl From<ReviewError> for CommandFailure {
    fn from(error: ReviewError) -> Self {
        Self::new("review.invalid", error.to_string())
    }
}

impl From<ForgeError> for CommandFailure {
    fn from(error: ForgeError) -> Self {
        let code = match error {
            ForgeError::ApprovalRequired => "forge.approval_required",
            ForgeError::StalePlan => "forge.stale_plan",
            ForgeError::NoCommits(_) => "forge.no_commits",
            ForgeError::TaskNotReady(_) => "forge.task_not_ready",
            ForgeError::RepositoryMismatch => "forge.repository_mismatch",
            ForgeError::UnsupportedRemote(_) => "forge.unsupported_remote",
            ForgeError::Http { .. } => "forge.http",
            ForgeError::Git(_) => "forge.git",
            ForgeError::Request(_) => "forge.request",
            ForgeError::InvalidResponse(_) | ForgeError::Json(_) => "forge.invalid_response",
            ForgeError::Storage(_) => "forge.storage",
            ForgeError::Credential(_) => "forge.credential",
        };
        Self::new(code, error.to_string())
    }
}

impl From<TaskWorktreeError> for CommandFailure {
    fn from(error: TaskWorktreeError) -> Self {
        let code = match error {
            TaskWorktreeError::Storage(_) => "task.storage",
            TaskWorktreeError::Git(_) => "task.git",
            TaskWorktreeError::TaskNotFound(_) => "task.not_found",
            TaskWorktreeError::InvalidTransition { .. } => "task.invalid_transition",
            TaskWorktreeError::InvalidBaseRef { .. } => "task.invalid_base",
            TaskWorktreeError::DirtyWorktree { .. } => "task.dirty_worktree",
            TaskWorktreeError::UnmergedBranch { .. } => "task.unmerged_branch",
            TaskWorktreeError::PathCollision(_) => "task.path_collision",
            TaskWorktreeError::InvalidStoredState(_) => "task.invalid_state",
            TaskWorktreeError::Filesystem { .. } => "task.filesystem",
            TaskWorktreeError::Timestamp(_) => "task.timestamp",
            TaskWorktreeError::Json(_) => "task.json",
        };
        Self::new(code, error.to_string())
    }
}

impl From<OverlapError> for CommandFailure {
    fn from(error: OverlapError) -> Self {
        Self::new("overlap.inspect", error.to_string())
    }
}

impl From<ProcessError> for CommandFailure {
    fn from(error: ProcessError) -> Self {
        let code = match error {
            ProcessError::Storage(_) => "agent.storage",
            ProcessError::Io(_) => "agent.process",
            ProcessError::SessionNotFound(_) => "agent.session_not_found",
            ProcessError::SessionNotRunning(_) => "agent.not_running",
            ProcessError::StatePoisoned => "agent.state",
            ProcessError::MissingStream(_) => "agent.missing_stream",
            ProcessError::InvalidStoredStatus(_) => "agent.invalid_status",
            ProcessError::Json(_) => "agent.json",
            ProcessError::Timestamp(_) => "agent.timestamp",
        };
        Self::new(code, error.to_string())
    }
}

impl From<AdapterError> for CommandFailure {
    fn from(error: AdapterError) -> Self {
        let code = match error {
            AdapterError::ProbeLaunch { .. } => "adapter.not_installed",
            AdapterError::ProbeFailed { .. } => "adapter.probe_failed",
            AdapterError::EmptyPrompt => "adapter.empty_prompt",
        };
        Self::new(code, error.to_string())
    }
}

impl From<PermissionError> for CommandFailure {
    fn from(error: PermissionError) -> Self {
        Self::new("permission.state", error.to_string())
    }
}

impl From<CaptureError> for CommandFailure {
    fn from(error: CaptureError) -> Self {
        let code = match error {
            CaptureError::Storage(_) => "context.storage",
            CaptureError::Filesystem { .. } => "context.filesystem",
            CaptureError::NotAFile(_) => "context.not_file",
            CaptureError::ArtifactTooLarge { .. } => "context.too_large",
            CaptureError::DigestMismatch { .. } => "context.digest_mismatch",
            CaptureError::EmptyText => "context.empty_text",
            CaptureError::InvalidStoredKind(_) => "context.invalid_kind",
            CaptureError::ItemNotFound(_) => "context.not_found",
            CaptureError::Quarantined(_) => "context.quarantined",
            CaptureError::Timestamp(_) => "context.timestamp",
        };
        Self::new(code, error.to_string())
    }
}

impl From<SkillError> for CommandFailure {
    fn from(error: SkillError) -> Self {
        let code = match error {
            SkillError::Storage(_) => "skill.storage",
            SkillError::Network(_) => "skill.network",
            SkillError::InvalidSource(_) => "skill.invalid_source",
            SkillError::HashMismatch { .. } => "skill.hash_mismatch",
            SkillError::InvalidRegistryHash(_) => "skill.invalid_registry_hash",
            SkillError::MissingSkillMarkdown => "skill.missing_manifest",
            SkillError::UnsafePath(_) => "skill.unsafe_path",
            SkillError::NotInspected(_) => "skill.not_inspected",
            SkillError::Blocked(_) => "skill.blocked",
            SkillError::InstallationNotFound(_) => "skill.installation_not_found",
            SkillError::InvalidStoredScope(_) => "skill.invalid_scope",
            SkillError::MissingScopeTarget { .. } => "skill.missing_scope",
            SkillError::Filesystem { .. } => "skill.filesystem",
            SkillError::Json(_) => "skill.json",
            SkillError::Timestamp(_) => "skill.timestamp",
        };
        Self::new(code, error.to_string())
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentPermissions {
    write_files: bool,
    run_commands: bool,
    network: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentLaunchInput {
    task_id: String,
    adapter: AdapterKind,
    prompt: String,
    context_pack: ContextPack,
    resume_session_id: Option<String>,
    permissions: AgentPermissions,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MemoryResolutionAction {
    Approve,
    Reject,
    Correct,
    MarkStale,
    Pin,
    Unpin,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemoryResolutionInput {
    task_id: String,
    record_id: String,
    action: MemoryResolutionAction,
    title: Option<String>,
    body: Option<String>,
}

struct CliExtractionProvider {
    adapter: AdapterKind,
    cwd: std::path::PathBuf,
}

struct CliReviewProvider {
    adapter: AdapterKind,
    cwd: std::path::PathBuf,
}

impl ReviewProvider for CliReviewProvider {
    fn review(&self, prompt: &str) -> Result<String, String> {
        let adapter = adapter_for(self.adapter);
        let command = adapter
            .command(&AgentLaunchRequest {
                executable: executable_name(self.adapter).into(),
                cwd: self.cwd.clone(),
                prompt: prompt.into(),
                resume_session_id: None,
                allowed_tools: Vec::new(),
                allow_write: false,
            })
            .map_err(|error| error.to_string())?;
        let output = std::process::Command::new(command.program)
            .args(command.arguments)
            .current_dir(command.cwd)
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
        let mut decoder = JsonLineDecoder::new();
        let mut events = decoder.push(&output.stdout, adapter.as_ref());
        events.extend(decoder.finish(adapter.as_ref()));
        events
            .into_iter()
            .filter_map(|event| match event {
                NormalizedAgentEvent::Text { text } => Some(strip_json_fence(&text)),
                _ => None,
            })
            .next_back()
            .ok_or_else(|| "reviewer returned no review JSON".into())
    }
}

impl ExtractionProvider for CliExtractionProvider {
    fn extract(&self, bundle: &mars_brain::EvidenceBundle) -> Result<String, String> {
        let prompt = render_extraction_prompt(bundle).map_err(|error| error.to_string())?;
        let adapter = adapter_for(self.adapter);
        let command = adapter
            .command(&AgentLaunchRequest {
                executable: executable_name(self.adapter).into(),
                cwd: self.cwd.clone(),
                prompt,
                resume_session_id: None,
                allowed_tools: Vec::new(),
                allow_write: false,
            })
            .map_err(|error| error.to_string())?;
        let output = std::process::Command::new(command.program)
            .args(command.arguments)
            .current_dir(command.cwd)
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
        let mut decoder = JsonLineDecoder::new();
        let mut events = decoder.push(&output.stdout, adapter.as_ref());
        events.extend(decoder.finish(adapter.as_ref()));
        events
            .into_iter()
            .filter_map(|event| match event {
                NormalizedAgentEvent::Text { text } => Some(strip_json_fence(&text)),
                _ => None,
            })
            .next_back()
            .ok_or_else(|| "agent returned no memory extraction JSON".into())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentLaunchOutcome {
    session_id: String,
    adapter: AdapterKind,
    version: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentTimelinePayload {
    session_id: String,
    event: NormalizedAgentEvent,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContextArtifactContent {
    bytes: Vec<u8>,
    mime_type: String,
    original_name: Option<String>,
}

#[tauri::command]
fn core_info(request: CommandRequest) -> CommandResponse {
    Core::new().execute(request)
}

#[tauri::command]
fn open_repository(
    path: String,
    trust: TrustDecision,
    state: State<'_, AppState>,
) -> Result<OpenRepositoryOutcome, CommandFailure> {
    state
        .projects
        .open_repository(path, trust)
        .map_err(Into::into)
}

#[tauri::command]
fn list_files(
    repository_id: String,
    cursor: Option<String>,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<FilePage, CommandFailure> {
    state
        .files
        .list_files(&repository_id, cursor.as_deref(), limit)
        .map_err(Into::into)
}

#[tauri::command]
fn read_file(
    repository_id: String,
    relative_path: String,
    state: State<'_, AppState>,
) -> Result<FileDocument, CommandFailure> {
    state
        .files
        .read_file(&repository_id, &relative_path)
        .map_err(Into::into)
}

#[tauri::command]
fn write_file(
    repository_id: String,
    relative_path: String,
    content: String,
    expected_version: String,
    state: State<'_, AppState>,
) -> Result<FileDocument, CommandFailure> {
    state
        .files
        .write_file(&repository_id, &relative_path, &content, &expected_version)
        .map_err(Into::into)
}

#[tauri::command]
fn spawn_terminal(
    repository_id: String,
    columns: u16,
    rows: u16,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<TerminalSessionInfo, CommandFailure> {
    let output_app = app.clone();
    state
        .terminals
        .spawn_terminal(
            &repository_id,
            columns,
            rows,
            Arc::new(move |output| {
                let _ = output_app.emit("terminal-output", output);
            }),
        )
        .map_err(Into::into)
}

#[tauri::command]
fn write_terminal(
    session_id: String,
    data: String,
    state: State<'_, AppState>,
) -> Result<(), CommandFailure> {
    state
        .terminals
        .write_terminal(&session_id, &data)
        .map_err(Into::into)
}

#[tauri::command]
fn resize_terminal(
    session_id: String,
    columns: u16,
    rows: u16,
    state: State<'_, AppState>,
) -> Result<(), CommandFailure> {
    state
        .terminals
        .resize_terminal(&session_id, columns, rows)
        .map_err(Into::into)
}

#[tauri::command]
fn cancel_terminal(session_id: String, state: State<'_, AppState>) -> Result<(), CommandFailure> {
    state
        .terminals
        .cancel_terminal(&session_id)
        .map_err(Into::into)
}

#[tauri::command]
fn index_repository(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<IndexSummary, CommandFailure> {
    state
        .indexer
        .index_repository(&repository_id, &tokio_util::sync::CancellationToken::new())
        .map_err(Into::into)
}

#[tauri::command]
fn find_symbols(
    repository_id: String,
    query: String,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<BrainSymbol>, CommandFailure> {
    state
        .brain
        .find_symbol(&repository_id, &query, limit)
        .map_err(Into::into)
}

#[tauri::command]
fn brain_references(
    repository_id: String,
    symbol_id: String,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<BrainEdge>, CommandFailure> {
    state
        .brain
        .references(&repository_id, &symbol_id, limit)
        .map_err(Into::into)
}

#[tauri::command]
fn brain_callers(
    repository_id: String,
    symbol_id: String,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<BrainEdge>, CommandFailure> {
    state
        .brain
        .callers(&repository_id, &symbol_id, limit)
        .map_err(Into::into)
}

#[tauri::command]
fn brain_callees(
    repository_id: String,
    symbol_id: String,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<BrainEdge>, CommandFailure> {
    state
        .brain
        .callees(&repository_id, &symbol_id, limit)
        .map_err(Into::into)
}

#[tauri::command]
fn brain_dependencies(
    repository_id: String,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<Dependency>, CommandFailure> {
    state
        .brain
        .dependencies(&repository_id, limit)
        .map_err(Into::into)
}

#[tauri::command]
fn brain_related_code(
    repository_id: String,
    symbol_id: String,
    depth: usize,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<BrainNeighborhood, CommandFailure> {
    state
        .brain
        .related_code(&repository_id, &symbol_id, depth, limit)
        .map_err(Into::into)
}

#[tauri::command]
fn brain_blast_radius(
    repository_id: String,
    symbol_id: String,
    depth: usize,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<BrainNeighborhood, CommandFailure> {
    state
        .brain
        .blast_radius(&repository_id, &symbol_id, depth, limit)
        .map_err(Into::into)
}

impl From<AskError> for CommandFailure {
    fn from(error: AskError) -> Self {
        let code = match error {
            AskError::Storage(_) => "brain.storage",
            AskError::Index(_) => "brain.index",
            AskError::RepositoryNotFound(_) => "brain.repository_not_found",
            AskError::EmptyQuestion => "brain.empty_question",
            AskError::SymbolNotFound(_) => "brain.symbol_not_found",
            AskError::TargetNotFound(_) => "brain.target_not_found",
            AskError::InvalidBudget(_) => "brain.invalid_budget",
            AskError::Provider(_) => "brain.provider",
            AskError::InvalidOutput(_) => "brain.invalid_output",
            AskError::UnknownSource { .. } => "brain.unknown_source",
            AskError::UnknownSupport { .. } => "brain.unknown_support",
            AskError::DuplicateClaim(_) => "brain.duplicate_claim",
            // A contract violation is the validator refusing to emit an answer
            // that would blur fact and inference. It is reported as such rather
            // than as a generic failure.
            AskError::Contract(_) => "brain.contract",
            AskError::Cancelled { .. } => "brain.cancelled",
        };
        Self::new(code, error.to_string())
    }
}

/// The event name a Brain answer streams on. One channel per question, keyed by
/// the caller's `stream_id`, so two questions in flight cannot interleave.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BrainStreamPayload {
    stream_id: String,
    event: BrainEvent,
}

/// Streams pipeline events to the webview when the caller supplied a stream id.
fn emit_brain_events(app: &AppHandle, stream_id: Option<&String>, event: &BrainEvent) {
    let Some(stream_id) = stream_id else {
        return;
    };
    let _ = app.emit(
        "brain-event",
        BrainStreamPayload {
            stream_id: stream_id.clone(),
            event: event.clone(),
        },
    );
}

#[tauri::command]
fn brain_ask(
    repository_id: String,
    question: String,
    budget_tokens: Option<usize>,
    max_sources: Option<usize>,
    stream_id: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BrainAnswer, CommandFailure> {
    let mut request = AskRequest::new(repository_id, question);
    if let Some(budget) = budget_tokens {
        request.budget_tokens = budget;
    }
    if let Some(sources) = max_sources {
        request.max_sources = sources;
    }
    state
        .project_brain
        .ask_observing(
            &request,
            &tokio_util::sync::CancellationToken::new(),
            |event| emit_brain_events(&app, stream_id.as_ref(), event),
        )
        .map_err(Into::into)
}

#[tauri::command]
fn brain_explain(
    repository_id: String,
    symbol_id: String,
    stream_id: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BrainAnswer, CommandFailure> {
    state
        .project_brain
        .explain_observing(
            &repository_id,
            &symbol_id,
            &tokio_util::sync::CancellationToken::new(),
            |event| emit_brain_events(&app, stream_id.as_ref(), event),
        )
        .map_err(Into::into)
}

#[tauri::command]
fn brain_where_is(
    repository_id: String,
    concept: String,
    stream_id: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BrainAnswer, CommandFailure> {
    state
        .project_brain
        .where_is_observing(
            &repository_id,
            &concept,
            &tokio_util::sync::CancellationToken::new(),
            |event| emit_brain_events(&app, stream_id.as_ref(), event),
        )
        .map_err(Into::into)
}

#[tauri::command]
fn brain_dependency_graph(
    repository_id: String,
    target: String,
    state: State<'_, AppState>,
) -> Result<DependencySlice, CommandFailure> {
    state
        .project_brain
        .dependencies(
            &repository_id,
            &target,
            &tokio_util::sync::CancellationToken::new(),
        )
        .map_err(Into::into)
}

/// The person at this machine.
///
/// An approval is the entire authority behind a memory, so who gave it is not
/// something the webview gets to assert. Every actor field on a memory command
/// is filled here and any value the client sent is discarded.
fn local_actor() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "local user".to_owned())
}

impl From<MemoryError> for CommandFailure {
    fn from(error: MemoryError) -> Self {
        let code = match error {
            MemoryError::Storage(_) => "memory.storage",
            MemoryError::Timestamp(_) => "memory.timestamp",
            MemoryError::InvalidStoredValue(_) => "memory.invalid_stored_value",
            MemoryError::ProposalNotFound(_) => "memory.proposal_not_found",
            MemoryError::MemoryNotFound(_) => "memory.not_found",
            MemoryError::ProposalNotPending { .. } => "memory.proposal_not_pending",
            MemoryError::MemoryNotActive { .. } => "memory.not_active",
            MemoryError::Empty { .. } => "memory.empty",
            MemoryError::TooLong { .. } => "memory.too_long",
            MemoryError::ConfidenceOutOfRange(_) => "memory.confidence_out_of_range",
            MemoryError::ScopeIncomplete { .. } => "memory.scope_incomplete",
            MemoryError::StaleConflictSet => "memory.stale_conflict_set",
            MemoryError::MultipleSupersedes => "memory.multiple_supersedes",
            MemoryError::OriginNotFound { .. } => "memory.origin_not_found",
            MemoryError::TaskNotFound(_) => "memory.task_not_found",
        };
        Self::new(code, error.to_string())
    }
}

/// `memory.propose` — an agent queues a candidate. This is the only memory
/// command an agent tool is given, and it cannot write a memory.
#[tauri::command]
fn memory_propose(
    proposal: NewProposal,
    state: State<'_, AppState>,
) -> Result<MemoryProposal, CommandFailure> {
    state.memory.propose(proposal).map_err(Into::into)
}

/// `memory.pending` — the human's approval queue.
#[tauri::command]
fn memory_pending(
    filters: ProposalFilters,
    state: State<'_, AppState>,
) -> Result<Vec<MemoryProposal>, CommandFailure> {
    state.memory.pending(&filters).map_err(Into::into)
}

/// `memory.approve` — accept, edit and accept, or resolve a conflict first.
#[tauri::command]
fn memory_approve(
    mut request: ApproveRequest,
    state: State<'_, AppState>,
) -> Result<ApprovalOutcome, CommandFailure> {
    request.approved_by = local_actor();
    state.memory.approve(&request).map_err(Into::into)
}

/// `memory.reject`.
#[tauri::command]
fn memory_reject(
    mut request: RejectRequest,
    state: State<'_, AppState>,
) -> Result<MemoryProposal, CommandFailure> {
    request.rejected_by = local_actor();
    state.memory.reject(&request).map_err(Into::into)
}

/// `memory.list(filters)`.
#[tauri::command]
fn memory_list(
    filters: MemoryFilters,
    state: State<'_, AppState>,
) -> Result<Vec<Memory>, CommandFailure> {
    state.memory.list(&filters).map_err(Into::into)
}

/// `memory.forRetrieval(context)`. The UI calls this to preview what an agent
/// would be given, so it passes `recordUses: false`; a caller that is itself
/// the injection passes true.
#[tauri::command]
fn memory_for_retrieval(
    context: RetrievalContext,
    state: State<'_, AppState>,
) -> Result<Vec<RetrievedMemory>, CommandFailure> {
    state.memory.for_retrieval(&context).map_err(Into::into)
}

/// `memory.usage(id)` — which tasks this memory has actually shaped.
#[tauri::command]
fn memory_usage(
    memory_id: String,
    state: State<'_, AppState>,
) -> Result<MemoryUsage, CommandFailure> {
    state.memory.usage(&memory_id).map_err(Into::into)
}

/// The full audit trail, including the approval message.
#[tauri::command]
fn memory_history(
    memory_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<MemoryEvent>, CommandFailure> {
    state.memory.history(&memory_id).map_err(Into::into)
}

/// Editing an approved memory creates a new version linked by `supersedes`.
#[tauri::command]
fn memory_revise(
    mut request: ReviseRequest,
    state: State<'_, AppState>,
) -> Result<Memory, CommandFailure> {
    request.revised_by = local_actor();
    state.memory.revise(&request).map_err(Into::into)
}

/// Withdraws a memory from retrieval while keeping it for audit.
#[tauri::command]
fn memory_revoke(
    mut request: RevokeRequest,
    state: State<'_, AppState>,
) -> Result<Memory, CommandFailure> {
    request.revoked_by = local_actor();
    state.memory.revoke(&request).map_err(Into::into)
}

#[tauri::command]
fn capture_text_context(
    repository_id: String,
    task_id: Option<String>,
    title: Option<String>,
    text: String,
    state: State<'_, AppState>,
) -> Result<ContextItem, CommandFailure> {
    state
        .context
        .capture_text(&repository_id, task_id.as_deref(), title.as_deref(), &text)
        .map_err(Into::into)
}

#[tauri::command]
fn capture_context_bytes(
    repository_id: String,
    task_id: Option<String>,
    title: Option<String>,
    original_name: String,
    bytes: Vec<u8>,
    mime_type: String,
    state: State<'_, AppState>,
) -> Result<ContextItem, CommandFailure> {
    state
        .context
        .capture_bytes(
            &repository_id,
            task_id.as_deref(),
            title.as_deref(),
            &original_name,
            &bytes,
            &mime_type,
        )
        .map_err(Into::into)
}

#[tauri::command]
fn search_context(
    repository_id: String,
    task_id: Option<String>,
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<ContextItem>, CommandFailure> {
    state
        .context
        .search(&repository_id, task_id.as_deref(), &query, 100)
        .map_err(Into::into)
}

#[tauri::command]
fn classify_context(
    item_id: String,
    classification: String,
    state: State<'_, AppState>,
) -> Result<ContextItem, CommandFailure> {
    state
        .context
        .classify(&item_id, &classification)
        .map_err(Into::into)
}

#[tauri::command]
fn read_context_artifact(
    item_id: String,
    state: State<'_, AppState>,
) -> Result<ContextArtifactContent, CommandFailure> {
    let item = state.context.get(&item_id)?;
    let Some(path) = item.artifact_path else {
        return Err(CommandFailure::new(
            "context.no_artifact",
            "text context does not have a binary artifact",
        ));
    };
    let bytes = fs::read(&path).map_err(|error| {
        CommandFailure::new(
            "context.filesystem",
            format!("failed to read `{}`: {error}", path.display()),
        )
    })?;
    Ok(ContextArtifactContent {
        bytes,
        mime_type: item.mime_type,
        original_name: item.original_name,
    })
}

#[tauri::command]
fn search_skills(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<SkillCandidate>, CommandFailure> {
    state.skills.search(&query).map_err(Into::into)
}

#[tauri::command]
fn inspect_skill(
    candidate: SkillCandidate,
    state: State<'_, AppState>,
) -> Result<InspectedSkill, CommandFailure> {
    state.skills.inspect(&candidate).map_err(Into::into)
}

#[tauri::command]
fn install_skill(
    skill_id: String,
    repository_id: String,
    task_id: String,
    scope: SkillScope,
    allow_claude: bool,
    allow_codex: bool,
    state: State<'_, AppState>,
) -> Result<SkillInstallation, CommandFailure> {
    let scoped_repository =
        (!matches!(scope, SkillScope::Global)).then_some(repository_id.as_str());
    let scoped_task =
        matches!(scope, SkillScope::Task | SkillScope::Workflow).then_some(task_id.as_str());
    state
        .skills
        .trust_and_install(
            &skill_id,
            scoped_repository,
            scoped_task,
            scope,
            allow_claude,
            allow_codex,
        )
        .map_err(Into::into)
}

#[tauri::command]
fn list_task_skills(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<SkillInstallation>, CommandFailure> {
    state.skills.list_for_task(&task_id).map_err(Into::into)
}

#[tauri::command]
fn remove_skill_installation(
    installation_id: String,
    state: State<'_, AppState>,
) -> Result<(), CommandFailure> {
    state.skills.remove(&installation_id).map_err(Into::into)
}

#[tauri::command]
fn list_task_memory(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<KnowledgeRecord>, CommandFailure> {
    state.knowledge.list_for_task(&task_id).map_err(Into::into)
}

#[tauri::command]
fn extract_task_memory(
    task_id: String,
    adapter: AdapterKind,
    model_permission: bool,
    state: State<'_, AppState>,
) -> Result<Vec<KnowledgeRecord>, CommandFailure> {
    let task = state.tasks.get(&task_id)?;
    let (session_id, observed_at): (String, String) = state.storage.with_connection(|connection| {
        connection.query_row(
            "SELECT id, COALESCE(ended_at, started_at) FROM agent_sessions
             WHERE task_id = ?1 AND status = 'completed' ORDER BY COALESCE(ended_at, started_at) DESC, id DESC LIMIT 1",
            [&task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    }).map_err(|error| CommandFailure::new("memory.session", error.to_string()))?;
    let diff = mars_git::command::run(
        &task.worktree_path,
        ["diff", "--no-ext-diff", "--unified=3", "HEAD"],
    )
    .map_err(|error| CommandFailure::new("memory.diff", error.to_string()))?
    .stdout_text();
    let provider = Arc::new(CliExtractionProvider {
        adapter,
        cwd: task.worktree_path.clone(),
    });
    let extracted =
        ExtractionService::new(state.storage.clone(), provider).extract(ExtractionRequest {
            session_id: session_id.clone(),
            diff,
            model_permission,
        })?;
    let project_id: String = state
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT project_id FROM tasks WHERE id = ?1",
                [&task_id],
                |row| row.get(0),
            )
        })
        .map_err(|error| CommandFailure::new("memory.task", error.to_string()))?;
    let affected = extracted.affected_entities;
    let claims = std::iter::once(extracted.summary)
        .chain(extracted.decisions)
        .chain(extracted.constraints)
        .chain(extracted.attempts)
        .chain(extracted.risks)
        .chain(extracted.questions)
        .chain(extracted.tests)
        .chain(extracted.commits);
    let mut records = Vec::new();
    for claim in claims {
        let provenance = state.knowledge.create_provenance(NewProvenance {
            project_id: project_id.clone(),
            kind: ProvenanceKind::AgentInferred,
            source: EntityRef::new(EntityKind::AgentSession, &session_id),
            evidence: serde_json::json!({ "evidenceIds": claim.evidence, "category": claim.category }),
            observed_at: observed_at.clone(),
        })?;
        let record = state.knowledge.create_candidate(NewKnowledge {
            project_id: project_id.clone(),
            kind: knowledge_kind(claim.category),
            title: claim.title,
            body: claim.body,
            confidence: claim.confidence,
            provenance_id: provenance.id.clone(),
        })?;
        for entity in &affected {
            state.knowledge.link(
                &project_id,
                EntityRef::new(EntityKind::Knowledge, &record.id),
                "AFFECTS",
                EntityRef::new(entity.kind, &entity.id),
                &provenance.id,
                claim.confidence,
            )?;
        }
        records.push(record);
    }
    Ok(records)
}

#[tauri::command]
fn resolve_memory(
    request: MemoryResolutionInput,
    state: State<'_, AppState>,
) -> Result<KnowledgeRecord, CommandFailure> {
    let (project_id, observed_at): (String, String) = state
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT project_id, updated_at FROM tasks WHERE id = ?1",
                [&request.task_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .map_err(|error| CommandFailure::new("memory.task", error.to_string()))?;
    let human = state.knowledge.create_provenance(NewProvenance {
        project_id,
        kind: ProvenanceKind::Human,
        source: EntityRef::new(EntityKind::Task, &request.task_id),
        evidence: serde_json::json!({ "action": format!("{:?}", request.action), "visibleReview": true }),
        observed_at,
    })?;
    match request.action {
        MemoryResolutionAction::Approve => state.knowledge.approve(&request.record_id, &human.id),
        MemoryResolutionAction::Reject => state.knowledge.reject(&request.record_id, &human.id),
        MemoryResolutionAction::Correct => state.knowledge.correct(
            &request.record_id,
            request.title.as_deref().unwrap_or_default(),
            request.body.as_deref().unwrap_or_default(),
            &human.id,
        ),
        MemoryResolutionAction::MarkStale => {
            state.knowledge.mark_stale(&request.record_id, &human.id)
        }
        MemoryResolutionAction::Pin => {
            state
                .knowledge
                .set_pinned(&request.record_id, true, &human.id)
        }
        MemoryResolutionAction::Unpin => {
            state
                .knowledge
                .set_pinned(&request.record_id, false, &human.id)
        }
    }
    .map_err(Into::into)
}

#[tauri::command]
fn why_query(
    repository_id: String,
    subject: EntityRef,
    max_depth: usize,
    max_paths: usize,
    state: State<'_, AppState>,
) -> Result<WhyAnswer, CommandFailure> {
    let project_id = state
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT project_id FROM repositories WHERE id = ?1",
                [&repository_id],
                |row| row.get(0),
            )
        })
        .map_err(|error| CommandFailure::new("why.repository", error.to_string()))?;
    state
        .why
        .ask(WhyRequest {
            project_id,
            subject,
            max_depth,
            max_paths,
        })
        .map_err(Into::into)
}

#[tauri::command]
fn build_review_handoff(
    task_id: String,
    reviewer_adapter: AdapterKind,
    max_bytes: usize,
    state: State<'_, AppState>,
) -> Result<ReviewHandoff, CommandFailure> {
    let task = state.tasks.get(&task_id)?;
    let reviewer_name = adapter_name(reviewer_adapter);
    let implementation_session_id: String = state
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT id FROM agent_sessions
                 WHERE task_id = ?1 AND status = 'completed' AND adapter_kind != ?2
                 ORDER BY COALESCE(ended_at, started_at) DESC, id DESC LIMIT 1",
                [task_id.as_str(), reviewer_name],
                |row| row.get(0),
            )
        })
        .map_err(|error| CommandFailure::new("review.session", error.to_string()))?;
    let diff = mars_git::command::run(
        &task.worktree_path,
        ["diff", "--no-ext-diff", "--unified=3", "HEAD"],
    )
    .map_err(|error| CommandFailure::new("review.diff", error.to_string()))?
    .stdout_text();
    state
        .reviews
        .build_handoff(ReviewHandoffRequest {
            task_id,
            implementation_session_id,
            reviewer_adapter: reviewer_name.into(),
            diff,
            max_bytes,
        })
        .map_err(Into::into)
}

#[tauri::command]
fn run_cross_agent_review(
    handoff: ReviewHandoff,
    model_permission: bool,
    state: State<'_, AppState>,
) -> Result<ReviewRecord, CommandFailure> {
    let task = state.tasks.get(&handoff.task_id)?;
    let reviewer_adapter = parse_adapter(&handoff.reviewer_adapter)?;
    let diff = mars_git::command::run(
        &task.worktree_path,
        ["diff", "--no-ext-diff", "--unified=3", "HEAD"],
    )
    .map_err(|error| CommandFailure::new("review.diff", error.to_string()))?
    .stdout_text();
    let verified = state.reviews.build_handoff(ReviewHandoffRequest {
        task_id: handoff.task_id.clone(),
        implementation_session_id: handoff.implementation_session_id.clone(),
        reviewer_adapter: handoff.reviewer_adapter.clone(),
        diff,
        max_bytes: handoff.max_bytes,
    })?;
    if verified != handoff {
        return Err(CommandFailure::new(
            "review.handoff_stale",
            "implementation evidence changed after preview; review the refreshed handoff",
        ));
    }
    state
        .reviews
        .run_review(
            &verified,
            model_permission,
            &CliReviewProvider {
                adapter: reviewer_adapter,
                cwd: task.worktree_path,
            },
        )
        .map_err(Into::into)
}

#[tauri::command]
fn list_task_reviews(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ReviewRecord>, CommandFailure> {
    state.reviews.list_for_task(&task_id).map_err(Into::into)
}

#[tauri::command]
fn set_review_finding_status(
    finding_id: String,
    status: FindingStatus,
    state: State<'_, AppState>,
) -> Result<mars_brain::ReviewFinding, CommandFailure> {
    state
        .reviews
        .set_finding_status(&finding_id, status)
        .map_err(Into::into)
}

#[tauri::command]
fn review_followup(
    review_id: String,
    state: State<'_, AppState>,
) -> Result<String, CommandFailure> {
    state
        .reviews
        .render_followup(&review_id)
        .map_err(Into::into)
}

#[tauri::command]
fn build_context_pack(
    request: ContextPackRequest,
    state: State<'_, AppState>,
) -> Result<ContextPack, CommandFailure> {
    state.context_packs.build(request).map_err(Into::into)
}

#[tauri::command]
fn list_tasks(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<WorktreePlan>, CommandFailure> {
    state.tasks.list(&repository_id).map_err(Into::into)
}

#[tauri::command]
fn create_task(
    repository_id: String,
    title: String,
    intent: String,
    base_ref: String,
    state: State<'_, AppState>,
) -> Result<WorktreePlan, CommandFailure> {
    state
        .tasks
        .create_task(&repository_id, &title, &intent, &base_ref)
        .map_err(Into::into)
}

#[tauri::command]
fn task_overlap_warnings(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<OverlapWarning>, CommandFailure> {
    state.overlaps.warnings(&task_id).map_err(Into::into)
}

#[tauri::command]
fn import_github_issue(
    repository_id: String,
    remote_name: String,
    issue_number: u64,
    state: State<'_, AppState>,
) -> Result<StoredIssue, CommandFailure> {
    let remote = state.forge.resolve_remote(&repository_id, &remote_name)?;
    let provider = connected_github_provider(&state, &remote.id)?;
    state
        .forge
        .import_issue(&repository_id, &remote_name, issue_number, &provider)
        .map_err(Into::into)
}

#[tauri::command]
fn connect_github(
    repository_id: String,
    remote_name: String,
    token: String,
    state: State<'_, AppState>,
) -> Result<ForgeConnection, CommandFailure> {
    let remote = state.forge.resolve_remote(&repository_id, &remote_name)?;
    let api_base = github_api_base(&remote.repository);
    let provider = github_provider(remote.repository, &token)?;
    let user = provider.current_user()?;
    let credential_ref = format!("github:{}:{}", remote.id, user.login);
    state.credentials.set(&credential_ref, token.trim())?;
    match state.forge.save_connection(
        &remote.id,
        "github",
        &api_base,
        Some(&user.login),
        &credential_ref,
    ) {
        Ok(connection) => Ok(connection),
        Err(error) => {
            let _ = state.credentials.delete(&credential_ref);
            Err(error.into())
        }
    }
}

#[tauri::command]
fn create_task_from_issue(
    issue_id: String,
    base_ref: String,
    state: State<'_, AppState>,
) -> Result<WorktreePlan, CommandFailure> {
    let issue = state.forge.get_issue(&issue_id)?;
    let task = state.tasks.create_task(
        &issue.repository_id,
        &issue.issue.title,
        &issue.issue.body,
        &base_ref,
    )?;
    state.forge.link_issue_task(&issue.id, &task.task_id)?;
    Ok(task)
}

#[tauri::command]
fn preview_pull_request(
    task_id: String,
    remote_name: String,
    title: String,
    body: String,
    draft: bool,
    state: State<'_, AppState>,
) -> Result<PullRequestPlan, CommandFailure> {
    state
        .forge
        .plan_pull_request(&task_id, &remote_name, &title, &body, draft)
        .map_err(Into::into)
}

#[tauri::command]
fn get_task_pull_request(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<Option<PullRequestSnapshot>, CommandFailure> {
    state
        .forge
        .get_task_pull_request(&task_id)
        .map_err(Into::into)
}

#[tauri::command]
fn list_task_forge_mutations(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ForgeMutation>, CommandFailure> {
    state
        .forge
        .list_task_mutations(&task_id, 20)
        .map_err(Into::into)
}

#[tauri::command]
fn create_github_pull_request(
    plan: PullRequestPlan,
    push_approved: bool,
    forge_approved: bool,
    state: State<'_, AppState>,
) -> Result<PullRequestSnapshot, CommandFailure> {
    let provider = connected_github_provider(&state, &plan.remote_id)?;
    state
        .forge
        .apply_pull_request(&plan, push_approved, forge_approved, &provider)
        .map_err(Into::into)
}

#[tauri::command]
fn refresh_github_pull_request(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<PullRequestSnapshot, CommandFailure> {
    let stored = state
        .forge
        .get_task_pull_request(&task_id)?
        .ok_or_else(|| CommandFailure::new("forge.pr_not_found", "Task has no pull request"))?;
    let provider = connected_github_provider(&state, &stored.remote_id)?;
    state
        .forge
        .refresh_pull_request(&task_id, &provider)
        .map_err(Into::into)
}

fn github_provider(
    repository: mars_forge::ForgeRepository,
    token: &str,
) -> Result<GitHubProvider, CommandFailure> {
    if token.trim().is_empty() {
        return Err(CommandFailure::new(
            "forge.missing_token",
            "A GitHub access token is required for this operation",
        ));
    }
    let api_base = github_api_base(&repository);
    GitHubProvider::new(repository, api_base, token).map_err(Into::into)
}

fn connected_github_provider(
    state: &AppState,
    remote_id: &str,
) -> Result<GitHubProvider, CommandFailure> {
    let connection = state
        .forge
        .get_connection(remote_id)?
        .ok_or_else(|| CommandFailure::new("forge.not_connected", "Connect GitHub first"))?;
    let remote = state.forge.get_remote(remote_id)?;
    let token = state.credentials.get(&connection.credential_ref)?;
    GitHubProvider::new(remote.repository, connection.api_base, &token).map_err(Into::into)
}

fn github_api_base(repository: &mars_forge::ForgeRepository) -> String {
    if repository.host == "github.com" {
        "https://api.github.com".into()
    } else {
        format!("https://{}/api/v3", repository.host)
    }
}

#[tauri::command]
fn provision_task(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<WorktreePlan, CommandFailure> {
    state.tasks.provision(&task_id).map_err(Into::into)
}

#[tauri::command]
fn remove_task_worktree(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<WorktreePlan, CommandFailure> {
    state.tasks.remove(&task_id).map_err(Into::into)
}

#[tauri::command]
fn open_task_worktree(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<OpenRepositoryOutcome, CommandFailure> {
    let task = state.tasks.get(&task_id)?;
    state
        .projects
        .open_repository(task.worktree_path, TrustDecision::Trust)
        .map_err(Into::into)
}

#[tauri::command]
fn probe_agent(adapter: AdapterKind) -> Result<AdapterProbe, CommandFailure> {
    let adapter_impl = adapter_for(adapter);
    adapter_impl
        .probe(Path::new(executable_name(adapter)))
        .map_err(Into::into)
}

#[tauri::command]
async fn launch_agent(
    request: AgentLaunchInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AgentLaunchOutcome, CommandFailure> {
    let AgentLaunchInput {
        task_id,
        adapter,
        prompt,
        context_pack,
        resume_session_id,
        permissions,
    } = request;
    let task = state.tasks.get(&task_id)?;
    if task.state != mars_git::WorktreeState::Ready {
        return Err(CommandFailure::new(
            "agent.worktree_not_ready",
            "approve and provision the task worktree before launching an agent",
        ));
    }
    if adapter == AdapterKind::Codex && !permissions.run_commands {
        return Err(CommandFailure::new(
            "permission.command_required",
            "Codex CLI sessions require explicit command permission",
        ));
    }
    let adapter_name = match adapter {
        AdapterKind::Claude => "claude",
        AdapterKind::Codex => "codex",
    };
    if context_pack.task_id != task_id || context_pack.adapter != adapter_name {
        return Err(CommandFailure::new(
            "context_pack.mismatch",
            "the preview does not match this task and adapter; review context again",
        ));
    }
    let preview_exclusions = context_pack
        .exclusions
        .iter()
        .filter(|item| item.reason == "removed in pack preview")
        .map(|item| item.entity_ref.clone())
        .collect();
    let verified_pack = state.context_packs.build(ContextPackRequest {
        task_id: task_id.clone(),
        adapter: adapter_name.into(),
        max_bytes: context_pack.max_bytes,
        excluded_entity_refs: preview_exclusions,
    })?;
    if verified_pack != context_pack {
        return Err(CommandFailure::new(
            "context_pack.stale",
            "task context changed after preview; review the refreshed pack before launch",
        ));
    }
    // The pack is now the one the agent will actually receive, so this is the
    // moment its memories count as used -- not when it was previewed, and not
    // when it was rebuilt for the comparison above, either of which would
    // record influence the memory never had.
    state.memory.record_uses(
        &injections_from_pack(&verified_pack),
        &InjectionContext {
            task_id: Some(task_id.clone()),
            session_id: None,
            surface: "context_pack".into(),
        },
    )?;
    let prompt = render_context_pack(&verified_pack, &prompt);

    let adapter_impl = adapter_for(adapter);
    let executable = Path::new(executable_name(adapter));
    let probe = adapter_impl.probe(executable)?;
    let allowed_tools =
        authorize_agent(&state.permissions, &task, adapter, permissions, executable)?;
    let command = adapter_impl.command(&AgentLaunchRequest {
        executable: executable.into(),
        cwd: task.worktree_path.clone(),
        prompt,
        resume_session_id,
        allowed_tools,
        allow_write: permissions.write_files,
    })?;
    let mut spec = ProcessSpec::new(
        match adapter {
            AdapterKind::Claude => "claude",
            AdapterKind::Codex => "codex",
        },
        command.program,
        command.arguments,
        command.cwd,
    );
    spec.task_id = Some(task_id);
    let launched = state.agents.launch(spec)?;
    let session_id = launched.session_id.clone();
    stream_agent_events(app, state.agents.clone(), adapter_impl, launched);
    Ok(AgentLaunchOutcome {
        session_id,
        adapter,
        version: probe.version,
    })
}

#[tauri::command]
fn cancel_agent(session_id: String, state: State<'_, AppState>) -> Result<(), CommandFailure> {
    state.agents.cancel(&session_id).map_err(Into::into)
}

#[tauri::command]
fn agent_session(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<SessionRecord, CommandFailure> {
    state.agents.session(&session_id).map_err(Into::into)
}

fn adapter_for(kind: AdapterKind) -> Box<dyn AgentAdapter> {
    match kind {
        AdapterKind::Claude => Box::new(ClaudeAdapter),
        AdapterKind::Codex => Box::new(CodexAdapter),
    }
}

fn executable_name(kind: AdapterKind) -> &'static str {
    match kind {
        AdapterKind::Claude => "claude",
        AdapterKind::Codex => "codex",
    }
}

fn adapter_name(kind: AdapterKind) -> &'static str {
    match kind {
        AdapterKind::Claude => "claude",
        AdapterKind::Codex => "codex",
    }
}

fn parse_adapter(value: &str) -> Result<AdapterKind, CommandFailure> {
    match value {
        "claude" => Ok(AdapterKind::Claude),
        "codex" => Ok(AdapterKind::Codex),
        _ => Err(CommandFailure::new(
            "review.adapter",
            "review adapter must be claude or codex",
        )),
    }
}

fn strip_json_fence(value: &str) -> String {
    let trimmed = value.trim();
    trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|value| value.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed)
        .to_owned()
}

fn knowledge_kind(category: MemoryCategory) -> KnowledgeKind {
    match category {
        MemoryCategory::Summary => KnowledgeKind::Summary,
        MemoryCategory::Decision => KnowledgeKind::Decision,
        MemoryCategory::Constraint => KnowledgeKind::Constraint,
        MemoryCategory::Attempt => KnowledgeKind::Attempt,
        MemoryCategory::Risk => KnowledgeKind::Risk,
        MemoryCategory::Question => KnowledgeKind::Question,
        MemoryCategory::Test => KnowledgeKind::Test,
        MemoryCategory::Commit => KnowledgeKind::Commit,
    }
}

fn authorize_agent(
    audit: &PermissionAudit,
    task: &WorktreePlan,
    adapter: AdapterKind,
    permissions: AgentPermissions,
    executable: &Path,
) -> Result<Vec<String>, CommandFailure> {
    let mut tools = vec!["Read".to_owned()];
    record_permission(
        audit,
        PermissionRequest::new(
            Capability::ReadFiles,
            PermissionScope::Worktree {
                task_id: task.task_id.clone(),
                root: task.worktree_path.clone(),
            },
        ),
        true,
    )?;
    if record_permission(
        audit,
        PermissionRequest::new(
            Capability::WriteFiles,
            PermissionScope::Worktree {
                task_id: task.task_id.clone(),
                root: task.worktree_path.clone(),
            },
        ),
        permissions.write_files,
    )? {
        tools.extend(["Edit".into(), "Write".into()]);
    }
    if record_permission(
        audit,
        PermissionRequest::new(
            Capability::RunCommand,
            PermissionScope::Command {
                program: executable.display().to_string(),
                cwd: task.worktree_path.clone(),
            },
        ),
        permissions.run_commands,
    )? && adapter == AdapterKind::Claude
    {
        tools.push("Bash".into());
    }
    if record_permission(
        audit,
        PermissionRequest::new(
            Capability::Network,
            PermissionScope::NetworkHost { host: "*".into() },
        ),
        permissions.network,
    )? && adapter == AdapterKind::Claude
    {
        tools.extend(["WebFetch".into(), "WebSearch".into()]);
    }
    Ok(tools)
}

fn record_permission(
    audit: &PermissionAudit,
    request: PermissionRequest,
    allowed: bool,
) -> Result<bool, CommandFailure> {
    let decision = if allowed {
        PermissionDecision::AllowSession
    } else {
        PermissionDecision::Deny
    };
    Ok(matches!(
        audit.record(&request, decision)?,
        PermissionOutcome::Allowed(_)
    ))
}

fn stream_agent_events(
    app: AppHandle,
    supervisor: ProcessSupervisor,
    adapter: Box<dyn AgentAdapter>,
    mut launched: mars_agents::process::LaunchedProcess,
) {
    tauri::async_runtime::spawn(async move {
        let mut decoder = JsonLineDecoder::new();
        let mut provider_ended = false;
        while let Some(process_event) = launched.events.recv().await {
            let events = match process_event.kind {
                ProcessEventKind::Stdout { text } => {
                    decoder.push(text.as_bytes(), adapter.as_ref())
                }
                ProcessEventKind::Stderr { text } => vec![NormalizedAgentEvent::Unknown {
                    provider_type: "stderr".into(),
                    raw: Value::String(text),
                }],
                ProcessEventKind::Exit { code, .. } => {
                    let mut events = decoder.finish(adapter.as_ref());
                    if !provider_ended {
                        events.push(adapter.on_exit(code));
                    }
                    events
                }
                ProcessEventKind::CancelRequested => vec![NormalizedAgentEvent::Permission {
                    tool: "process".into(),
                    description: "Cancellation requested".into(),
                }],
                ProcessEventKind::KillEscalated => vec![NormalizedAgentEvent::Unknown {
                    provider_type: "kill_escalated".into(),
                    raw: Value::Null,
                }],
                ProcessEventKind::DeliveryLagged => vec![NormalizedAgentEvent::Unknown {
                    provider_type: "delivery_lagged".into(),
                    raw: Value::Null,
                }],
                ProcessEventKind::InterruptedOnRestart => vec![NormalizedAgentEvent::End {
                    success: false,
                    message: Some("Session was interrupted by restart".into()),
                }],
            };
            for event in events {
                provider_ended |= matches!(event, NormalizedAgentEvent::End { .. });
                let _ = supervisor.record_normalized(&launched.session_id, &event);
                let _ = app.emit(
                    "agent-event",
                    AgentTimelinePayload {
                        session_id: launched.session_id.clone(),
                        event,
                    },
                );
            }
        }
    });
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            fs::create_dir_all(&data_dir)?;
            let storage = Arc::new(Storage::open(data_dir.join("mars.sqlite3"))?);
            let agents = ProcessSupervisor::new(storage.clone());
            agents.recover_interrupted()?;
            let forge = ForgeWorkflow::new(storage.clone());
            forge.recover_interrupted()?;
            let skill_registry = Arc::new(SkillsShRegistry::new()?);
            app.manage(AppState {
                agents,
                brain: BrainService::new(storage.clone()),
                project_brain: ProjectBrain::new(
                    storage.clone(),
                    Arc::new(mars_indexer::api::IndexApi::new(storage.clone())),
                ),
                // Proposals are verified against the agent transcript they
                // claim to come from, so an excerpt nobody said is refused.
                memory: MemoryService::with_transcript(
                    storage.clone(),
                    Arc::new(mars_brain::AgentSessionTranscript::new(storage.clone())),
                ),
                context_packs: ContextPackService::new(storage.clone()),
                context: ContextService::new(storage.clone(), data_dir.join("artifacts")),
                credentials: OsCredentialVault,
                files: FileService::new(storage.clone()),
                forge,
                indexer: Indexer::new(storage.clone()),
                knowledge: KnowledgeService::new(storage.clone()),
                overlaps: TaskOverlapService::new(storage.clone()),
                projects: ProjectService::new(storage.clone()),
                reviews: ReviewService::new(storage.clone()),
                permissions: PermissionAudit::new(storage.clone()),
                skills: SkillService::new(storage.clone(), skill_registry, data_dir.join("skills")),
                tasks: TaskWorktreeService::new(storage.clone(), data_dir.join("worktrees")),
                terminals: TerminalService::new(storage.clone()),
                why: WhyService::new(storage.clone()),
                storage,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core_info,
            open_repository,
            list_files,
            read_file,
            write_file,
            spawn_terminal,
            write_terminal,
            resize_terminal,
            cancel_terminal,
            index_repository,
            find_symbols,
            brain_references,
            brain_callers,
            brain_callees,
            brain_dependencies,
            brain_related_code,
            brain_blast_radius,
            brain_ask,
            brain_explain,
            brain_where_is,
            brain_dependency_graph,
            memory_propose,
            memory_pending,
            memory_approve,
            memory_reject,
            memory_list,
            memory_for_retrieval,
            memory_usage,
            memory_history,
            memory_revise,
            memory_revoke,
            capture_text_context,
            capture_context_bytes,
            search_context,
            classify_context,
            read_context_artifact,
            search_skills,
            inspect_skill,
            install_skill,
            list_task_skills,
            remove_skill_installation,
            list_task_memory,
            extract_task_memory,
            resolve_memory,
            why_query,
            build_review_handoff,
            run_cross_agent_review,
            list_task_reviews,
            set_review_finding_status,
            review_followup,
            build_context_pack,
            list_tasks,
            create_task,
            task_overlap_warnings,
            connect_github,
            import_github_issue,
            create_task_from_issue,
            preview_pull_request,
            get_task_pull_request,
            list_task_forge_mutations,
            create_github_pull_request,
            refresh_github_pull_request,
            provision_task,
            remove_task_worktree,
            open_task_worktree,
            probe_agent,
            launch_agent,
            cancel_agent,
            agent_session
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Mars desktop");
}
