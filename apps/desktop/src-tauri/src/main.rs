// A release build is a windowed app on Windows: no console opens behind it,
// and the `mcp` mode an agent CLI starts speaks over the pipes it is given.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod forge_commands;
mod knowledge_commands;
mod runs;
mod scm_commands;

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mars_activity::{
    ActivityEntry, ActivityError, ActivityEvent, ActivityKind, ActivityRecord, Actor,
    InterruptedRun, RecoveryAction, RecoveryService, RunRecord, RunRole, RunStore, TaskProjection,
    replay,
};
use mars_agents::account::{AgentAccount, SignInPreferences, read_account};
use mars_agents::adapter::{AdapterError, AdapterKind, AdapterProbe, AgentAdapter};
use mars_agents::claude::ClaudeAdapter;
use mars_agents::codex::CodexAdapter;
use mars_agents::process::{ProcessError, ProcessSupervisor, SessionRecord};
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
    ReviewRecord, ReviewService, ReviseRequest, RevokeRequest, RunContextPack,
    RunContextPackBuilder, RunPackRequest, ToolGrant, WhyAnswer, WhyError, WhyRequest, WhyService,
    injections_from_pack, render_context_pack, render_extraction_prompt,
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
    BaseDrift, DriftChoice, OverlapError, OverlapWarning, PlanRequest, SetupCommand,
    TaskOverlapService, TaskWorktreeError, TaskWorktreeService, WorktreePlan, WorktreePlanner,
    WorktreeProposal,
};
use mars_indexer::{IndexError, IndexSummary, Indexer};
use mars_security::permissions::{
    Capability, PermissionAudit, PermissionDecision, PermissionError, PermissionOutcome,
    PermissionRequest, PermissionScope,
};
use mars_security::{
    BrokerDecision, BrokerOutcome, CapabilityCeiling, GrantView, PendingRequest, PermissionBroker,
    TrustDecision,
};
use mars_skills::{
    InspectedSkill, SkillCandidate, SkillError, SkillInstallation, SkillScope, SkillService,
    SkillsShRegistry,
};
use mars_storage::Storage;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    /// The append-only log every surface in the product renders from.
    activity: ActivityRecord,
    agents: ProcessSupervisor,
    brain: BrainService,
    /// Standing grants, so a capability review can say what the agent will
    /// *not* be stopped for.
    broker: PermissionBroker,
    /// Where Mars keeps what it learns. Disclosed before trust is given.
    data_dir: PathBuf,
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
    /// Builds, approves and provisions worktree plans. Distinct from
    /// `tasks`, which only moves a task between worktree states.
    planner: WorktreePlanner,
    projects: ProjectService,
    /// Reconstructs what an interrupted run had done. Never resumes one.
    recovery: RecoveryService,
    reviews: ReviewService,
    runs: RunStore,
    permissions: PermissionAudit,
    /// Which agents run on their CLI's own sign-in even where an API key
    /// variable in Mars's environment would outrank it.
    sign_in: SignInPreferences,
    skills: SkillService,
    tasks: TaskWorktreeService,
    terminals: TerminalService,
    /// The cancellation token of each repository's in-flight index pass.
    ///
    /// Held here rather than in the indexer because it is a fact about this
    /// process's UI — which run a person can still stop — and not about the
    /// index itself.
    index_runs: Arc<Mutex<HashMap<String, tokio_util::sync::CancellationToken>>>,
    why: WhyService,
    storage: Arc<Storage>,
    /// The database file itself, which an agent's MCP server opens too.
    database: PathBuf,
    /// Runs whose processes this app started and is still reading.
    live_runs: Arc<runs::LiveRuns>,
    /// Where each agent CLI is, as a person chose it in Settings.
    executables: runs::AgentExecutables,
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
            // The pack a person approved is not the pack that would be sent.
            // The run does not start; the reviewer looks at it again.
            ContextPackError::PackChanged { .. } => "context_pack.changed",
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
            TaskWorktreeError::PlanNotFound(_) => "plan.not_found",
            TaskWorktreeError::PlanChanged { .. } => "plan.changed",
            TaskWorktreeError::PlanNotApproved { .. } => "plan.not_approved",
            TaskWorktreeError::PlanBlocked { .. } => "plan.blocked",
            TaskWorktreeError::SetupCommandFailed { .. } => "plan.setup_command_failed",
            TaskWorktreeError::CleanupNotFound(_) => "cleanup.not_found",
            TaskWorktreeError::CleanupNotRestorable { .. } => "cleanup.not_restorable",
            TaskWorktreeError::Activity(_) => "task.activity",
            TaskWorktreeError::Filesystem { .. } => "task.filesystem",
            TaskWorktreeError::Timestamp(_) => "task.timestamp",
            TaskWorktreeError::Json(_) => "task.json",
        };
        Self::new(code, error.to_string())
    }
}

impl From<ActivityError> for CommandFailure {
    fn from(error: ActivityError) -> Self {
        let code = match error {
            ActivityError::Storage(_) | ActivityError::Sqlite(_) => "activity.storage",
            ActivityError::Json(_) => "activity.json",
            ActivityError::Timestamp(_) => "activity.timestamp",
            ActivityError::PayloadIo { .. } => "activity.payload_io",
            // The bytes on disk do not match the digest they are filed under.
            // Refusing is the only safe answer: the caller asked for one exact
            // thing and this is not it.
            ActivityError::PayloadCorrupt { .. } => "activity.payload_corrupt",
            ActivityError::RunNotFound(_) => "run.not_found",
            ActivityError::RunAlreadyFinished(_) => "run.already_finished",
            _ => "activity.invalid",
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
    executable: PathBuf,
    cwd: std::path::PathBuf,
    removed_environment: Vec<OsString>,
}

struct CliReviewProvider {
    adapter: AdapterKind,
    executable: PathBuf,
    cwd: std::path::PathBuf,
    removed_environment: Vec<OsString>,
}

impl ReviewProvider for CliReviewProvider {
    fn review(&self, prompt: &str) -> Result<String, String> {
        runs::run_once(
            self.adapter,
            &self.executable,
            &self.cwd,
            prompt,
            &self.removed_environment,
        )
        .map_err(|error| {
            if error == "the agent returned no answer" {
                "reviewer returned no review JSON".to_owned()
            } else {
                error
            }
        })
    }
}

impl ExtractionProvider for CliExtractionProvider {
    fn extract(&self, bundle: &mars_brain::EvidenceBundle) -> Result<String, String> {
        let prompt = render_extraction_prompt(bundle).map_err(|error| error.to_string())?;
        runs::run_once(
            self.adapter,
            &self.executable,
            &self.cwd,
            &prompt,
            &self.removed_environment,
        )
        .map_err(|error| {
            if error == "the agent returned no answer" {
                "agent returned no memory extraction JSON".to_owned()
            } else {
                error
            }
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentLaunchOutcome {
    session_id: String,
    adapter: AdapterKind,
    version: String,
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
async fn open_repository(
    path: String,
    trust: TrustDecision,
    state: State<'_, AppState>,
) -> Result<OpenRepositoryOutcome, CommandFailure> {
    state
        .projects
        .open_repository(path, trust)
        .map_err(Into::into)
}

/// One class of file trust will not put in front of an agent, and why.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SkipRule {
    label: String,
    detail: String,
}

/// Where Mars keeps what it learns about this repository. Every path is inside
/// the app data directory; nothing is written into the repository itself.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StoragePaths {
    database: PathBuf,
    worktrees: PathBuf,
    artifacts: PathBuf,
    skills: PathBuf,
}

/// What a person is shown before they decide whether to trust a repository.
///
/// Every field is read without writing anything. A field Git could not answer
/// is `None` rather than a guess, so the screen can say "could not be read"
/// instead of showing a plausible wrong value.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryPreview {
    remote: Option<String>,
    branch: Option<String>,
    head_oid: Option<String>,
    head_summary: Option<String>,
    /// Files Git tracks. The size of the repository, not of the index.
    tracked_files: Option<usize>,
    /// Files the indexer would actually read, after ignore rules are applied.
    indexable_files: Option<usize>,
    skipped: Vec<SkipRule>,
    storage: StoragePaths,
}

/// A single-line Git read. A command that fails or answers with nothing yields
/// `None`: a repository with no commits and a repository Git refused to read
/// are both "not known", and neither is worth inventing a value for.
fn git_line<const N: usize>(root: &Path, arguments: [&str; N]) -> Option<String> {
    mars_git::command::run(root, arguments)
        .ok()
        .map(|output| output.stdout_text())
        .filter(|text| !text.is_empty())
}

/// Read a repository without trusting it, so the trust decision can be made
/// against what is actually there.
#[tauri::command]
async fn preview_repository(
    path: String,
    state: State<'_, AppState>,
) -> Result<RepositoryPreview, CommandFailure> {
    let root = Path::new(&path);
    if !root.is_dir() {
        return Err(CommandFailure::new(
            "repository.not_directory",
            format!("{path} is not a directory"),
        ));
    }

    let remote = git_line(root, ["remote", "get-url", "origin"]).or_else(|| {
        // A repository with no `origin` may still have exactly one remote.
        let names = git_line(root, ["remote"])?;
        let first = names.lines().next()?.trim().to_owned();
        git_line(root, ["remote", "get-url", &first])
    });

    let tracked_files = mars_git::command::run(root, ["ls-files"])
        .ok()
        .map(|output| {
            output
                .stdout_text()
                .lines()
                .filter(|line| !line.is_empty())
                .count()
        });

    // The honest number: what the indexer's own walk would read, not a guess
    // from the file count. Costly enough to be worth a loading state, cheap
    // enough to run before anyone commits to anything.
    let indexable_files = mars_indexer::walk::source_paths(root)
        .ok()
        .map(|paths| paths.len());

    Ok(RepositoryPreview {
        remote,
        branch: git_line(root, ["rev-parse", "--abbrev-ref", "HEAD"]),
        head_oid: git_line(root, ["rev-parse", "HEAD"]),
        head_summary: git_line(root, ["log", "-1", "--pretty=%s"]),
        tracked_files,
        indexable_files,
        skipped: vec![
            SkipRule {
                label: "Ignored directories".into(),
                detail: ".git, node_modules, target, dist, build and .next are never walked".into(),
            },
            SkipRule {
                label: "Ignore files".into(),
                detail: "anything .gitignore or .marsignore excludes, including nested ones".into(),
            },
            SkipRule {
                label: "Secrets".into(),
                detail: ".env files, private keys, certificates and credential files".into(),
            },
            SkipRule {
                label: "Large files".into(),
                detail: "files over 1 MiB are listed but never parsed or embedded".into(),
            },
        ],
        storage: StoragePaths {
            database: state.data_dir.join("mars.sqlite3"),
            worktrees: state.data_dir.join("worktrees"),
            artifacts: state.data_dir.join("artifacts"),
            skills: state.data_dir.join("skills"),
        },
    })
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
    task_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<FileDocument, CommandFailure> {
    let document =
        state
            .files
            .write_file(&repository_id, &relative_path, &content, &expected_version)?;

    // A person editing a file by hand is something that happened to the task,
    // and the record is what every surface renders from -- so an edit made in
    // the editor appears in the transcript exactly as an agent's would, with
    // the actor being the only difference.
    if let Some(task_id) = task_id {
        let _ = state.activity.append(
            ActivityEntry::new(
                &task_id,
                ActivityKind::FileEdit,
                Actor::Human,
                local_actor(),
                format!("edited {relative_path}"),
            )
            .with_payload(json!({
                "path": relative_path,
                "change": "modified",
                "bytes": content.len(),
            })),
        );
    }

    Ok(document)
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

/// Index a repository without holding up anything else.
///
/// A 20,000-file repository is minutes of parsing, hashing and embedding. Run
/// inline it occupied the command thread for the whole pass, reported nothing
/// while it did, and could not be stopped once started — so the only signal a
/// person had was that the app had gone quiet.
///
/// The work moves to a blocking worker, progress streams on `index-event`, and
/// the run's cancellation token is kept so `cancel_index` can reach it. The
/// summary is still returned to the caller that asked for it; the events are for
/// everyone else watching.
#[tauri::command]
async fn index_repository(
    repository_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<IndexSummary, CommandFailure> {
    let indexer = state.indexer.clone();
    let cancellation = tokio_util::sync::CancellationToken::new();
    let index_runs = state.index_runs.clone();
    {
        // Replacing an existing token cancels the run it belonged to: two full
        // passes over the same repository would fight over the same rows.
        let mut runs = index_runs.lock().expect("index run registry is poisoned");
        if let Some(previous) = runs.insert(repository_id.clone(), cancellation.clone()) {
            previous.cancel();
        }
    }

    let key = repository_id.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        indexer.run(
            &repository_id,
            &mars_indexer::IndexOptions::default(),
            &cancellation,
            |event| {
                // A dropped listener is not a reason to stop indexing.
                let _ = app.emit("index-event", event);
            },
        )
    })
    .await;

    index_runs
        .lock()
        .expect("index run registry is poisoned")
        .remove(&key);

    match outcome {
        Ok(result) => result.map_err(Into::into),
        Err(error) => Err(CommandFailure::new(
            "index.worker",
            format!("the index worker did not finish: {error}"),
        )),
    }
}

/// Stop an index pass that is still running. Nothing already written is undone;
/// the run is marked cancelled and the next pass picks up from the cursor.
#[tauri::command]
fn cancel_index(repository_id: String, state: State<'_, AppState>) -> Result<bool, CommandFailure> {
    let mut runs = state
        .index_runs
        .lock()
        .expect("index run registry is poisoned");
    match runs.remove(&repository_id) {
        Some(token) => {
            token.cancel();
            Ok(true)
        }
        None => Ok(false),
    }
}

/// The index as the home screen states it: what has been read, whether it still
/// matches the working tree, and what is running.
#[tauri::command]
fn index_status(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<mars_indexer::api::IndexStatus, CommandFailure> {
    mars_indexer::api::IndexApi::new(state.storage.clone())
        .status(&repository_id)
        .map_err(Into::into)
}

/// The working tree's current blob sha for each of a set of paths.
///
/// Everything the UI shows about source was derived from one exact version of
/// one file, and the sha is the only thing that keeps that claim checkable: a
/// path and a line survive the edit that moved the line elsewhere. A surface
/// asks for the paths its claims rest on and compares the answer against the
/// shas those claims carry, so it can say which of them the tree has moved past.
///
/// Bounded because a citation list is small; a caller with more paths than this
/// is asking the wrong question.
#[tauri::command]
fn blob_freshness(
    repository_id: String,
    paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<mars_indexer::api::Freshness>, CommandFailure> {
    const MAX_PATHS: usize = 256;
    let api = mars_indexer::api::IndexApi::new(state.storage.clone());
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for path in paths.into_iter().take(MAX_PATHS) {
        // `freshness` normalizes separators itself; dedupe on the same shape.
        let normalized = path.replace('\\', "/");
        if normalized.is_empty() || seen.contains(&normalized) {
            continue;
        }
        seen.push(normalized.clone());
        out.push(api.freshness(&repository_id, &normalized)?);
    }
    Ok(out)
}

/// Somewhere to start, and the evidence behind it.
///
/// A suggestion is only worth as much as the reason attached to it, so every
/// one of these carries the claim it rests on and where that claim came from.
/// Nothing here is inferred: each is read from the index or from Git.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartingPoint {
    id: String,
    title: String,
    /// Why this is being suggested, in terms a person can check.
    reason: String,
    /// Where the claim came from, so it can be verified.
    source: String,
    /// What picking this would put in the composer.
    prompt: String,
}

/// What Mars can suggest doing next, grounded in what it already knows.
#[tauri::command]
async fn starting_points(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<StartingPoint>, CommandFailure> {
    let index = mars_indexer::api::IndexApi::new(state.storage.clone());
    let status = index.status(&repository_id)?;
    let root = status.root_path.as_path();
    let mut points = Vec::new();

    if status.counts.files == 0 {
        points.push(StartingPoint {
            id: "index".into(),
            title: "Index this repository".into(),
            reason: "Nothing has been indexed yet, so questions about this code have nothing to \
                     read."
                .into(),
            source: "index status".into(),
            prompt: "Index this repository so you can answer questions about it.".into(),
        });
        // Everything below reads better against an index. Offering it alone is
        // clearer than offering it alongside suggestions that cannot work yet.
        return Ok(points);
    }

    if !status.head_matches_index {
        let stale = index
            .stale_paths(&repository_id)
            .map(|paths| paths.len())
            .unwrap_or(0);
        points.push(StartingPoint {
            id: "reindex".into(),
            title: "Re-index what moved".into(),
            reason: format!(
                "The working tree is at a different commit than the index, and {stale} \
                 file(s) no longer match what was read."
            ),
            source: "index freshness".into(),
            prompt: "Re-index the files that changed since the last run, then tell me what moved."
                .into(),
        });
    }

    // Uncommitted work is the most likely thing a person came back to finish.
    if let Ok(output) = mars_git::command::run(root, ["status", "--porcelain"]) {
        let text = output.stdout_text();
        let changed: Vec<&str> = text
            .lines()
            .filter_map(|line| line.get(3..))
            .filter(|path| !path.is_empty())
            .collect();
        if !changed.is_empty() {
            let sample = changed
                .iter()
                .take(3)
                .copied()
                .collect::<Vec<_>>()
                .join(", ");
            points.push(StartingPoint {
                id: "working-tree".into(),
                title: "Pick up the change in your working tree".into(),
                reason: format!("{} file(s) are modified and uncommitted.", changed.len()),
                source: "git status".into(),
                prompt: format!("Explain what is changing in {sample} and what is left to finish."),
            });
        }
    }

    if let Some(head) = status.head_oid.as_deref() {
        let summary = git_line(root, ["log", "-1", "--pretty=%s"]).unwrap_or_default();
        points.push(StartingPoint {
            id: "head".into(),
            title: "Review what landed last".into(),
            reason: format!("HEAD is {} — {summary}", &head[..head.len().min(7)]),
            source: "git log".into(),
            prompt: format!("Explain what changed in commit {head} and why it was done that way."),
        });
    }

    Ok(points)
}

/// Hybrid search over the code index: lexical and semantic, fused.
#[tauri::command]
async fn search_code(
    repository_id: String,
    query: String,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<mars_indexer::api::SearchResults, CommandFailure> {
    mars_indexer::api::IndexApi::new(state.storage.clone())
        .search_hybrid(
            &repository_id,
            &query,
            limit.clamp(1, 100),
            &mars_indexer::search::SearchFilters::default(),
        )
        .map_err(Into::into)
}

/// Every grant ever made, with whether it is still live. Pass a task to narrow
/// it; omit it for the whole machine, which is what Settings shows.
#[tauri::command]
fn permission_grants(
    task_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<GrantView>, CommandFailure> {
    state.broker.grants(task_id.as_deref()).map_err(Into::into)
}

/// Withdraw a standing grant. Who revoked it is filled in here, not asserted by
/// the webview.
#[tauri::command]
fn revoke_permission_grant(
    grant_id: String,
    state: State<'_, AppState>,
) -> Result<(), CommandFailure> {
    state
        .broker
        .revoke(&grant_id, &local_actor())
        .map_err(Into::into)
}

/// Whether the base moved out from under a task, and what can be done about it.
/// `None` means it did not.
#[tauri::command]
async fn task_base_drift(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<Option<BaseDrift>, CommandFailure> {
    state.planner.base_drift(&task_id).map_err(Into::into)
}

/// Act on drift. Every branch of this records what was chosen and why.
#[tauri::command]
async fn resolve_task_base_drift(
    task_id: String,
    choice: DriftChoice,
    state: State<'_, AppState>,
) -> Result<WorktreeProposal, CommandFailure> {
    state
        .planner
        .resolve_base_drift(&task_id, choice, &local_actor())
        .map_err(Into::into)
}

/// One hit in the command palette, whatever kind of thing it points at.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PaletteHit {
    kind: String,
    /// What to show. Already the most specific name the thing has.
    label: String,
    /// Where it is, in the terms that kind of thing is addressed by.
    detail: String,
    /// Enough to jump: a task id, a `path`, a `path:line`, or a memory id.
    target: String,
    /// Line to reveal, when the target is a file.
    line: Option<usize>,
    /// The task a message belongs to, so a jump can select it first.
    task_id: Option<String>,
    sequence: Option<i64>,
}

/// One search across everything Mars knows: tasks, messages, files, symbols and
/// memories.
///
/// Each kind is queried on its own terms and capped on its own, so a repository
/// with ten thousand files cannot crowd out the one memory that answers the
/// question. Nothing is ranked across kinds, because there is no honest way to
/// compare a file name to an approved memory; the caller groups them instead.
#[tauri::command]
async fn search_everything(
    repository_id: String,
    project_id: String,
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<PaletteHit>, CommandFailure> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Ok(Vec::new());
    }
    let mut hits = Vec::new();

    for task in state.tasks.list(&repository_id).unwrap_or_default() {
        if task.title.to_lowercase().contains(&needle)
            || task.branch_name.to_lowercase().contains(&needle)
        {
            hits.push(PaletteHit {
                kind: "task".into(),
                label: task.title.clone(),
                detail: task.branch_name.clone(),
                target: task.task_id.clone(),
                line: None,
                task_id: Some(task.task_id.clone()),
                sequence: None,
            });
        }
        if hits.len() >= 8 {
            break;
        }
    }

    // Messages are read per task because the log is per task; a global text
    // index over the transcript does not exist and is not worth faking.
    let mut messages = 0;
    for task in state.tasks.list(&repository_id).unwrap_or_default() {
        if messages >= 8 {
            break;
        }
        for event in state
            .activity
            .read(&task.task_id, 0, 500)
            .unwrap_or_default()
        {
            if event.kind != ActivityKind::Message {
                continue;
            }
            let text = event
                .payload
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or(&event.summary);
            if !text.to_lowercase().contains(&needle) {
                continue;
            }
            hits.push(PaletteHit {
                kind: "message".into(),
                label: text.lines().next().unwrap_or_default().to_owned(),
                detail: format!("{} · {}", task.title, event.actor_label),
                target: task.task_id.clone(),
                line: None,
                task_id: Some(task.task_id.clone()),
                sequence: Some(event.sequence),
            });
            messages += 1;
            if messages >= 8 {
                break;
            }
        }
    }

    if let Ok(page) = state.files.list_files(&repository_id, None, 2000) {
        for entry in page.entries.iter().take(2000) {
            if entry.relative_path.to_lowercase().contains(&needle) {
                hits.push(PaletteHit {
                    kind: "file".into(),
                    label: entry
                        .relative_path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&entry.relative_path)
                        .to_owned(),
                    detail: entry.relative_path.clone(),
                    target: entry.relative_path.clone(),
                    line: None,
                    task_id: None,
                    sequence: None,
                });
            }
            if hits.iter().filter(|hit| hit.kind == "file").count() >= 8 {
                break;
            }
        }
    }

    if let Ok(symbols) = state.brain.find_symbol(&repository_id, &query, 8) {
        for symbol in symbols {
            hits.push(PaletteHit {
                kind: "symbol".into(),
                label: symbol.name.clone(),
                detail: format!("{}:{}", symbol.relative_path, symbol.start_line),
                target: symbol.relative_path.clone(),
                line: Some(symbol.start_line),
                task_id: None,
                sequence: None,
            });
        }
    }

    if let Ok(memories) = state.memory.list(&MemoryFilters {
        project_id,
        repository_id: Some(repository_id.clone()),
        task_id: None,
        scopes: Vec::new(),
        kinds: Vec::new(),
        statuses: Vec::new(),
        contains: Some(query.clone()),
        limit: Some(8),
    }) {
        for memory in memories {
            hits.push(PaletteHit {
                kind: "memory".into(),
                label: memory.text.clone(),
                detail: format!("{} · {}", memory.kind.as_str(), memory.scope.as_str()),
                target: memory.id.clone(),
                line: None,
                task_id: memory.task_id.clone(),
                sequence: None,
            });
        }
    }

    Ok(hits)
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
async fn brain_ask(
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
async fn brain_explain(
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
async fn brain_where_is(
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
async fn brain_dependency_graph(
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
async fn search_skills(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<SkillCandidate>, CommandFailure> {
    state.skills.search(&query).map_err(Into::into)
}

#[tauri::command]
async fn inspect_skill(
    candidate: SkillCandidate,
    state: State<'_, AppState>,
) -> Result<InspectedSkill, CommandFailure> {
    state.skills.inspect(&candidate).map_err(Into::into)
}

#[tauri::command]
async fn install_skill(
    skill_id: String,
    repository_id: String,
    task_id: Option<String>,
    scope: SkillScope,
    allow_claude: bool,
    allow_codex: bool,
    state: State<'_, AppState>,
) -> Result<SkillInstallation, CommandFailure> {
    let scoped_repository =
        (!matches!(scope, SkillScope::Global)).then_some(repository_id.as_str());
    let scoped_task = matches!(scope, SkillScope::Task | SkillScope::Workflow)
        .then_some(task_id.as_deref())
        .flatten();
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

/// Every installation that matters to a repository: global, the
/// repository's own, and its tasks'.
#[tauri::command]
fn repository_skills(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<SkillInstallation>, CommandFailure> {
    state
        .skills
        .list_for_repository(&repository_id)
        .map_err(Into::into)
}

#[tauri::command]
fn set_skill_enabled(
    installation_id: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<SkillInstallation, CommandFailure> {
    state
        .skills
        .set_enabled(&installation_id, enabled)
        .map_err(Into::into)
}

#[tauri::command]
fn set_skill_adapter(
    installation_id: String,
    adapter: AdapterKind,
    allowed: bool,
    state: State<'_, AppState>,
) -> Result<SkillInstallation, CommandFailure> {
    let policy = match adapter {
        AdapterKind::Claude => mars_skills::AdapterPolicy::Claude,
        AdapterKind::Codex => mars_skills::AdapterPolicy::Codex,
    };
    state
        .skills
        .set_adapter_allowed(&installation_id, policy, allowed)
        .map_err(Into::into)
}

/// Whether the registry has a newer version than an installation pins.
/// Reads only; updating is inspecting and installing the new version.
#[tauri::command]
async fn check_skill_update(
    installation_id: String,
    state: State<'_, AppState>,
) -> Result<mars_skills::SkillUpdate, CommandFailure> {
    state
        .skills
        .check_update(&installation_id)
        .map_err(Into::into)
}

/// Stop trusting a skill everywhere it is installed.
#[tauri::command]
fn block_skill(skill_id: String, state: State<'_, AppState>) -> Result<(), CommandFailure> {
    state.skills.block(&skill_id).map_err(Into::into)
}

/// Skills already on this machine that the CLIs load themselves: the
/// repository's committed ones and the person's own.
#[tauri::command]
async fn local_skills(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<mars_skills::LocalSkill>, CommandFailure> {
    let repository = mars_storage::projects::ProjectRepository::new(state.storage.clone())
        .find_repository(&repository_id)
        .map_err(|error| CommandFailure::new("repository.not_found", error.to_string()))?
        .root_path;
    let home = std::env::var_os("USERPROFILE")
        .filter(|_| cfg!(windows))
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    Ok(mars_skills::discover_local(&repository, home.as_deref()))
}

#[tauri::command]
fn list_task_memory(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<KnowledgeRecord>, CommandFailure> {
    state.knowledge.list_for_task(&task_id).map_err(Into::into)
}

#[tauri::command]
async fn extract_task_memory(
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
    // Everything the task changed since its base, committed or not: an agent
    // that commits its work must not leave the extractor looking at nothing.
    let (diff, _) = scm_commands::task_change(&state, &task)?;
    let provider = Arc::new(CliExtractionProvider {
        adapter,
        executable: state.executables.resolve(adapter)?.path,
        cwd: task.worktree_path.clone(),
        removed_environment: sign_in_environment(&state, adapter),
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
async fn why_query(
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
async fn build_review_handoff(
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
    let (diff, _) = scm_commands::task_change(&state, &task)?;
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
async fn run_cross_agent_review(
    handoff: ReviewHandoff,
    model_permission: bool,
    state: State<'_, AppState>,
) -> Result<ReviewRecord, CommandFailure> {
    let task = state.tasks.get(&handoff.task_id)?;
    let reviewer_adapter = parse_adapter(&handoff.reviewer_adapter)?;
    let (diff, _) = scm_commands::task_change(&state, &task)?;
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
                executable: state.executables.resolve(reviewer_adapter)?.path,
                cwd: task.worktree_path,
                removed_environment: sign_in_environment(&state, reviewer_adapter),
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
async fn build_context_pack(
    request: ContextPackRequest,
    state: State<'_, AppState>,
) -> Result<ContextPack, CommandFailure> {
    state.context_packs.build(request).map_err(Into::into)
}

/// Put what a person said into the task's record.
///
/// The log is what every surface renders from, so a turn that is not in it did
/// not happen as far as reload, recovery and export are concerned. Who said it
/// is filled in here from the local user rather than taken from the webview.
#[tauri::command]
fn record_message(
    task_id: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<ActivityEvent, CommandFailure> {
    let summary = text.lines().next().unwrap_or_default().to_owned();
    state
        .activity
        .append(
            ActivityEntry::new(
                &task_id,
                ActivityKind::Message,
                Actor::Human,
                local_actor(),
                summary,
            )
            .with_payload(serde_json::json!({ "text": text })),
        )
        .map_err(Into::into)
}

/// A page of a task's log, in sequence order.
///
/// The UI renders from this and nothing else. Paging by `after_sequence` is what
/// makes a live stream and a reload the same operation: both ask for what they
/// have not seen yet.
#[tauri::command]
fn task_activity(
    task_id: String,
    after_sequence: i64,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<ActivityEvent>, CommandFailure> {
    state
        .activity
        .read(&task_id, after_sequence, limit)
        .map_err(Into::into)
}

/// What was written, and how much of it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditExport {
    path: String,
    byte_size: usize,
}

/// A task's whole record as a document a person can read.
///
/// Rendered from the append-only log and nothing else — no status column, no
/// cached summary — so what the document says and what the transcript shows
/// cannot disagree. Returned rather than written so the UI can show it before
/// anybody decides where it should go.
#[tauri::command]
fn task_audit(task_id: String, state: State<'_, AppState>) -> Result<String, CommandFailure> {
    state
        .activity
        .audit_markdown_now(&task_id)
        .map_err(Into::into)
}

/// Write that document to a path the person chose in the OS save dialog.
///
/// The destination is theirs, so this does not second-guess it; it does refuse
/// to render a task whose record is too large to be one document rather than
/// writing a partial file that looks whole.
#[tauri::command]
async fn write_task_audit(
    task_id: String,
    destination: String,
    state: State<'_, AppState>,
) -> Result<AuditExport, CommandFailure> {
    let document = state.activity.audit_markdown_now(&task_id)?;
    let path = PathBuf::from(&destination);
    std::fs::write(&path, document.as_bytes()).map_err(|error| {
        CommandFailure::new(
            "audit.write",
            format!("the audit could not be written to {destination}: {error}"),
        )
    })?;
    Ok(AuditExport {
        path: destination,
        byte_size: document.len(),
    })
}

/// The newest page of a task's record, and how much sits before it.
///
/// The conversation reads from the end. `task_activity` walks forward from the
/// beginning, which is right for an export and wrong here: a task that outgrows
/// one page would keep showing its first thousand events and never its live
/// tail. This starts at the end, and reports what it left behind so the screen
/// can say so rather than ending the record without comment.
#[tauri::command]
fn task_activity_window(
    task_id: String,
    before_sequence: Option<i64>,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<mars_activity::ActivityWindow, CommandFailure> {
    state
        .activity
        .read_window(&task_id, before_sequence, limit)
        .map_err(Into::into)
}

/// The whole of a task's state, folded from its log.
///
/// Nothing here reads a status column, so what the summary says and what the
/// transcript shows cannot disagree.
#[tauri::command]
fn task_projection(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<TaskProjection, CommandFailure> {
    replay(&state.activity, &task_id).map_err(Into::into)
}

/// Command output, diffs and test results, resolved from the digest the event
/// carries. Kept out of the event row so the log stays pageable.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivityPayload {
    digest: String,
    byte_size: usize,
    /// Lossy UTF-8. Every payload the record spills is program output.
    text: String,
}

#[tauri::command]
fn activity_payload(
    digest: String,
    state: State<'_, AppState>,
) -> Result<ActivityPayload, CommandFailure> {
    let bytes = state.activity.payload_bytes(&digest)?;
    Ok(ActivityPayload {
        digest,
        byte_size: bytes.len(),
        text: String::from_utf8_lossy(&bytes).into_owned(),
    })
}

/// The runs a task has had, newest state included.
#[tauri::command]
fn task_runs(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<RunRecord>, CommandFailure> {
    state.runs.list_for_task(&task_id).map_err(Into::into)
}

/// Every request still waiting on a person, oldest first.
#[tauri::command]
fn pending_permissions(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<PendingRequest>, CommandFailure> {
    state.broker.pending(&task_id).map_err(Into::into)
}

/// What the broker did with an answer.
#[derive(Serialize)]
#[serde(rename_all = "camelCase", tag = "outcome")]
enum PermissionOutcomeView {
    Allowed {
        grant_id: String,
        reason: String,
    },
    Denied {
        reason: String,
        /// True when the ceiling refused it, which no person can override.
        by_construction: bool,
    },
    /// Still waiting. Answering a request cannot produce this, but the broker's
    /// vocabulary includes it and flattening it away would be a lie.
    Pending {
        request_id: String,
    },
}

impl From<BrokerOutcome> for PermissionOutcomeView {
    fn from(outcome: BrokerOutcome) -> Self {
        match outcome {
            BrokerOutcome::Allowed { grant_id, reason } => Self::Allowed { grant_id, reason },
            BrokerOutcome::Denied {
                reason,
                by_construction,
            } => Self::Denied {
                reason,
                by_construction,
            },
            BrokerOutcome::Pending { request_id } => Self::Pending { request_id },
        }
    }
}

/// Answer a permission request and release the agent waiting on it.
///
/// Who answered is filled in here from the local user. A decision is the entire
/// authority behind an effect, so it is not the webview's to assert.
#[tauri::command]
fn decide_permission(
    request_id: String,
    decision: String,
    state: State<'_, AppState>,
) -> Result<PermissionOutcomeView, CommandFailure> {
    let parsed = BrokerDecision::parse(&decision).ok_or_else(|| {
        CommandFailure::new(
            "permission.unknown_decision",
            format!("`{decision}` is not one of allow_once, allow_task, allow_pattern, deny"),
        )
    })?;
    state
        .broker
        .decide(&request_id, parsed, &local_actor())
        .map(Into::into)
        .map_err(Into::into)
}

/// Runs that were alive when Mars stopped. Read-only: it changes no status and
/// starts no process.
#[tauri::command]
fn interrupted_runs(state: State<'_, AppState>) -> Result<Vec<InterruptedRun>, CommandFailure> {
    state.recovery.detect().map_err(Into::into)
}

/// Record that a run was found interrupted. The only status change recovery
/// makes on its own, and it moves the run further from running, never closer.
#[tauri::command]
fn note_interrupted_run(
    run_id: String,
    state: State<'_, AppState>,
) -> Result<InterruptedRun, CommandFailure> {
    state.recovery.mark_interrupted(&run_id).map_err(Into::into)
}

/// Record which way out of an interrupted run a person chose, so the choice is
/// in the transcript before anything acts on it.
#[tauri::command]
fn resolve_interrupted_run(
    run_id: String,
    action: RecoveryAction,
    state: State<'_, AppState>,
) -> Result<(), CommandFailure> {
    state
        .recovery
        .record_choice(&run_id, action, &local_actor())
        .map_err(Into::into)
}

/// The change a task has made, as Git states it.
///
/// Both halves matter and neither implies the other: `diff` is what is
/// committed-or-not against HEAD, and `untracked` names files Git has never
/// seen, which a diff against HEAD cannot mention at all. A review that showed
/// only the first would quietly omit every new file the agent wrote.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskDiff {
    /// Unified diff against HEAD, with no external diff driver and 3 lines of
    /// context. Empty when the worktree is clean.
    diff: String,
    untracked: Vec<String>,
    /// The commit the diff is against.
    head_oid: Option<String>,
    /// The sha256 of `diff`, as `mars_brain::review` computes it.
    ///
    /// A cross-agent review records the digest of the diff it read. Handing the
    /// same digest back with every fresh read is what lets the UI compare the
    /// two and say that a finding's `path:line` no longer describes what is on
    /// disk — without that, a stale finding stays perfectly readable and wrong.
    diff_sha256: String,
    branch_name: String,
    worktree_path: PathBuf,
}

#[tauri::command]
async fn task_diff(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<TaskDiff, CommandFailure> {
    let task = state.tasks.get(&task_id)?;
    if task.state != mars_git::WorktreeState::Ready {
        return Err(CommandFailure::new(
            "diff.worktree_not_ready",
            "the task worktree does not exist yet, so there is nothing to review",
        ));
    }

    // Measured from where the task left its base, not from HEAD: an agent that
    // commits as it goes would otherwise leave nothing here to review.
    let (diff, untracked) = scm_commands::task_change(&state, &task)?;

    let diff_sha256 = format!(
        "{:x}",
        <sha2::Sha256 as sha2::Digest>::digest(diff.as_bytes())
    );

    Ok(TaskDiff {
        diff,
        untracked,
        head_oid: git_line(&task.worktree_path, ["rev-parse", "HEAD"]),
        diff_sha256,
        branch_name: task.branch_name.clone(),
        worktree_path: task.worktree_path.clone(),
    })
}

/// One file as it stands in the task worktree, for the right-hand side of a
/// diff or for a file Git has never seen.
#[tauri::command]
async fn task_worktree_file(
    task_id: String,
    relative_path: String,
    state: State<'_, AppState>,
) -> Result<String, CommandFailure> {
    let task = state.tasks.get(&task_id)?;
    let full = task.worktree_path.join(&relative_path);
    // The path is joined onto the worktree root and must stay under it; a
    // `..` segment in a diff header is not something to follow.
    if !full.starts_with(&task.worktree_path) {
        return Err(CommandFailure::new(
            "diff.outside_worktree",
            format!("{relative_path} is outside the task worktree"),
        ));
    }
    fs::read_to_string(&full)
        .map_err(|error| CommandFailure::new("diff.read_file", format!("{relative_path}: {error}")))
}

#[tauri::command]
fn list_tasks(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<WorktreePlan>, CommandFailure> {
    state.tasks.list(&repository_id).map_err(Into::into)
}

#[tauri::command]
async fn create_task(
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
async fn task_overlap_warnings(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<OverlapWarning>, CommandFailure> {
    state.overlaps.warnings(&task_id).map_err(Into::into)
}

#[tauri::command]
async fn import_github_issue(
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

/// One commit the push would carry.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskCommit {
    oid: String,
    subject: String,
    author: String,
    date: String,
}

/// A workflow the forge would run, read from the worktree rather than guessed.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlannedCheck {
    /// The workflow file, relative to the repository root.
    path: String,
    name: String,
    /// True when the file names `pull_request` among its triggers.
    on_pull_request: bool,
}

/// The commits between the task's base and its head, newest first.
#[tauri::command]
async fn task_commits(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<TaskCommit>, CommandFailure> {
    let task = state.tasks.get(&task_id)?;
    let range = format!("{}..HEAD", task.base_ref);
    // A unit separator keeps subjects containing any punctuation intact.
    let output = mars_git::command::run(
        &task.worktree_path,
        [
            "log",
            "--no-color",
            "--format=%H\x1f%s\x1f%an\x1f%aI",
            &range,
        ],
    )
    .map_err(|error| CommandFailure::new("publish.commits", error.to_string()))?
    .stdout_text();

    Ok(output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut parts = line.split('\x1f');
            Some(TaskCommit {
                oid: parts.next()?.to_owned(),
                subject: parts.next()?.to_owned(),
                author: parts.next()?.to_owned(),
                date: parts.next()?.to_owned(),
            })
        })
        .collect())
}

/// What CI would run, read from the workflow files in the worktree.
///
/// This is what the repository configures, not what the forge has scheduled: no
/// run exists until the pull request does. The screen says which of those two
/// it is showing rather than presenting a plan as a result.
#[tauri::command]
async fn planned_checks(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<PlannedCheck>, CommandFailure> {
    let task = state.tasks.get(&task_id)?;
    let directory = task.worktree_path.join(".github").join("workflows");
    let Ok(entries) = fs::read_dir(&directory) else {
        return Ok(Vec::new());
    };

    let mut checks = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let extension = path.extension().and_then(|value| value.to_str());
        if !matches!(extension, Some("yml") | Some("yaml")) {
            continue;
        }
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_owned();
        // The workflow's own `name:` when it has one, else the file name. This
        // is a text scan, not a YAML parse, and it does not pretend otherwise.
        let name = contents
            .lines()
            .find_map(|line| line.strip_prefix("name:"))
            .map(|value| value.trim().trim_matches(['"', '\'']).to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| file_name.clone());
        checks.push(PlannedCheck {
            path: format!(".github/workflows/{file_name}"),
            name,
            on_pull_request: contents.contains("pull_request"),
        });
    }
    checks.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(checks)
}

#[tauri::command]
async fn connect_github(
    repository_id: String,
    remote_name: String,
    token: String,
    state: State<'_, AppState>,
) -> Result<ForgeConnection, CommandFailure> {
    connect_github_with_token(&state, &repository_id, &remote_name, &token)
}

/// Verify a token against the account it belongs to, keep it in the OS
/// keychain, and record only a reference to it (ADR-024).
fn connect_github_with_token(
    state: &AppState,
    repository_id: &str,
    remote_name: &str,
    token: &str,
) -> Result<ForgeConnection, CommandFailure> {
    let remote = state.forge.resolve_remote(repository_id, remote_name)?;
    let api_base = github_api_base(&remote.repository);
    let provider = github_provider(remote.repository, token)?;
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
async fn create_task_from_issue(
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
async fn preview_pull_request(
    task_id: String,
    remote_name: String,
    title: String,
    body: String,
    draft: bool,
    reviewers: Vec<String>,
    state: State<'_, AppState>,
) -> Result<PullRequestPlan, CommandFailure> {
    state
        .forge
        .plan_pull_request(&task_id, &remote_name, &title, &body, draft, &reviewers)
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
async fn create_github_pull_request(
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
async fn refresh_github_pull_request(
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

/// A setup command as the client asks for it. The classification is *not* here:
/// how risky a command is is decided by the classifier, never asserted by the
/// webview, so the person approving the plan is reading Mars's reading of it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetupCommandInput {
    command: String,
    purpose: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlanTaskInput {
    repository_id: String,
    title: String,
    intent: String,
    base_ref: String,
    /// Globs the task expects to stay inside. A write outside these reaches the
    /// permission broker instead of happening quietly.
    #[serde(default)]
    expected_scope: Vec<String>,
    #[serde(default)]
    setup_commands: Vec<SetupCommandInput>,
}

impl From<PlanTaskInput> for PlanRequest {
    fn from(input: PlanTaskInput) -> Self {
        Self {
            repository_id: input.repository_id,
            title: input.title,
            intent: input.intent,
            base_ref: input.base_ref,
            expected_scope: input.expected_scope,
            setup_commands: input
                .setup_commands
                .into_iter()
                .map(|command| SetupCommand::new(command.command, command.purpose))
                .collect(),
        }
    }
}

/// Build a worktree plan. Writes plan rows; touches Git only to read.
#[tauri::command]
async fn plan_task(
    request: PlanTaskInput,
    state: State<'_, AppState>,
) -> Result<WorktreeProposal, CommandFailure> {
    state.planner.propose(request.into()).map_err(Into::into)
}

/// The plan as it stands *now*, with everything that can move underneath it
/// refreshed: the base commit, the path, the branch, the disk.
#[tauri::command]
fn task_plan(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<WorktreeProposal, CommandFailure> {
    state.planner.current(&task_id).map_err(Into::into)
}

/// Change what a plan declares, before its worktree exists. Any approval the
/// plan had is dropped by the core, so the edited plan must be approved again.
#[tauri::command]
async fn amend_task_plan(
    task_id: String,
    expected_scope: Vec<String>,
    setup_commands: Vec<SetupCommandInput>,
    state: State<'_, AppState>,
) -> Result<WorktreeProposal, CommandFailure> {
    state
        .planner
        .amend(
            &task_id,
            expected_scope,
            setup_commands
                .into_iter()
                .map(|command| SetupCommand::new(command.command, command.purpose))
                .collect(),
        )
        .map_err(Into::into)
}

/// Record approval of the exact plan that was on screen. The fingerprint is the
/// client's assertion of *which* plan it showed, and is checked; who approved it
/// is filled in here and never taken from the webview.
#[tauri::command]
async fn approve_task_plan(
    task_id: String,
    fingerprint: String,
    state: State<'_, AppState>,
) -> Result<WorktreeProposal, CommandFailure> {
    state
        .planner
        .approve(&task_id, &fingerprint, &local_actor())
        .map_err(Into::into)
}

/// Create the worktree for an approved plan, then run its setup commands.
/// Refuses if the plan changed after it was approved.
#[tauri::command]
async fn provision_approved_task(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<WorktreeProposal, CommandFailure> {
    state
        .planner
        .provision_approved(&task_id)
        .map_err(Into::into)
}

/// The role an agent is launched in. The role picks the ceiling; no decision
/// made during the run can widen it, and the client cannot name a ceiling
/// directly.
#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
enum RunRoleInput {
    Implementer,
    Reviewer,
}

impl RunRoleInput {
    fn ceiling(self) -> CapabilityCeiling {
        match self {
            Self::Implementer => CapabilityCeiling::implementer(),
            Self::Reviewer => CapabilityCeiling::reviewer(),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Implementer => "implementer",
            Self::Reviewer => "reviewer",
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunPackInput {
    task_id: String,
    agent_id: String,
    adapter: AdapterKind,
    role: RunRoleInput,
    /// What the person actually asked for, verbatim.
    assignment: String,
    max_bytes: usize,
    /// Entity refs the reviewer removed before starting the run.
    #[serde(default)]
    excluded_entity_refs: Vec<String>,
}

/// The tools an agent is offered, as the pack states them. Mutating tools are
/// listed here but filtered by the role's ceiling before the pack is built, so
/// a preview can never promise a tool the broker would refuse.
fn offered_tools(adapter: AdapterKind) -> Vec<ToolGrant> {
    let mut tools = vec![
        ToolGrant::read_only(
            "read files",
            "Read, search and list files inside the task worktree",
        ),
        ToolGrant::read_only(
            "mars graph",
            "search, find_symbol, references, callers, callees, impact, related, file_deps, ask, \
             where_is and explain over this repository's index; use them before grepping",
        ),
        ToolGrant::read_only(
            "mars memory",
            "memory and decisions: what people approved for this project",
        ),
        ToolGrant::mutating(
            "mars proposals",
            "record_decision and remember, which wait for a person's approval",
        ),
    ];
    match adapter {
        AdapterKind::Claude => {
            tools.push(ToolGrant::mutating(
                "edit files",
                "Edit and write files; inside the declared scope without asking, elsewhere in the \
                 worktree after a person allows it",
            ));
            tools.push(ToolGrant::mutating(
                "run commands",
                "Read-only commands run at once; anything that changes state asks first",
            ));
            tools.push(ToolGrant::mutating(
                "web",
                "Fetch a page or search the web, after a person allows the host",
            ));
        }
        AdapterKind::Codex => {
            tools.push(ToolGrant::mutating(
                "edit files and run commands",
                "Inside Codex's sandbox, confined to this worktree with the network off",
            ));
        }
    }
    tools
}

fn run_pack_request(
    state: &AppState,
    input: RunPackInput,
) -> Result<(RunContextPackBuilder, RunPackRequest), CommandFailure> {
    let plan = state.planner.current(&input.task_id)?;
    let grants = state.broker.grants(Some(&input.task_id))?;
    Ok((
        RunContextPackBuilder::new(state.storage.clone()).with_grants(grants),
        RunPackRequest {
            task_id: input.task_id,
            agent_id: input.agent_id,
            adapter: adapter_name(input.adapter).to_owned(),
            role: input.role.as_str().to_owned(),
            ceiling: input.role.ceiling(),
            declared_scope: plan.expected_scope,
            tools: offered_tools(input.adapter),
            assignment: input.assignment,
            max_bytes: input.max_bytes,
            excluded_entity_refs: input.excluded_entity_refs,
        },
    ))
}

/// Exactly what an agent will see and may do, assembled before the run starts.
///
/// The declared scope is not a parameter: it is read from the worktree plan the
/// person already approved, so the capability review and the plan approval
/// cannot disagree about where the agent may write.
#[tauri::command]
async fn build_run_context_pack(
    request: RunPackInput,
    state: State<'_, AppState>,
) -> Result<RunContextPack, CommandFailure> {
    let (builder, pack_request) = run_pack_request(&state, request)?;
    builder.build(pack_request).map_err(Into::into)
}

/// Start the first run of a task from the pack a person reviewed.
///
/// The pack is rebuilt and its digest compared with the one approved; a pack
/// that moved in between is refused and must be looked at again (ADR-042).
#[tauri::command]
async fn start_run(
    request: runs::StartRunInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<runs::LaunchedRun, CommandFailure> {
    let task = state.tasks.get(&request.task_id)?;
    let plan = state.planner.current(&request.task_id)?;
    let role = request.role;
    let (builder, pack_request) = run_pack_request(
        &state,
        RunPackInput {
            task_id: request.task_id.clone(),
            agent_id: request.agent_id.clone(),
            adapter: request.adapter,
            role,
            assignment: request.assignment,
            max_bytes: request.max_bytes,
            excluded_entity_refs: request.excluded_entity_refs,
        },
    )?;
    let pack = builder.revalidate(pack_request, &request.digest)?;
    let launched = runs::launch(
        launch_context(&app, &state, request.adapter, &request.task_id)?,
        runs::LaunchPlan {
            agent_id: request.agent_id,
            role: match role {
                RunRoleInput::Implementer => RunRole::Implementer,
                RunRoleInput::Reviewer => RunRole::Reviewer,
            },
            digest: pack.digest.clone(),
            prompt: pack.prompt.clone(),
            resume: None,
            resumes_run_id: None,
            declared_scope: plan.expected_scope,
            fixed_tools: None,
            allow_write: role == RunRoleInput::Implementer,
            capabilities: json!({
                "ceiling": pack.permissions.ceiling,
                "declaredScope": pack.permissions.declared_scope,
                "asksThroughMars": request.adapter == AdapterKind::Claude,
            }),
            adapter: request.adapter,
            task,
        },
    )?;
    // Recorded only now that the pack was sent (ADR-036).
    state.memory.record_uses(
        &pack.memory_injections(),
        &InjectionContext {
            task_id: Some(request.task_id),
            session_id: Some(launched.session_id.clone()),
            surface: "context_pack".into(),
        },
    )?;
    Ok(launched)
}

/// The next thing the person said, sent to the session the task already has.
///
/// The message is written to the conversation first, then the provider's own
/// session is resumed with exactly that text: the agent already holds the pack
/// it started with, so it is not sent again (ADR-045).
#[tauri::command]
async fn continue_run(
    request: runs::ContinueRunInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<runs::LaunchedRun, CommandFailure> {
    let text = request.text.trim().to_owned();
    if text.is_empty() {
        return Err(CommandFailure::new("run.empty", "there is nothing to send"));
    }
    if state.live_runs.for_task(&request.task_id).is_some() {
        return Err(CommandFailure::new(
            "agent.busy",
            "an agent is already working in this task; your message will go when it finishes",
        ));
    }
    let previous = state
        .runs
        .latest_resumable(&request.task_id)?
        .ok_or_else(|| {
            CommandFailure::new(
                "run.nothing_to_resume",
                "no agent has worked in this task yet; start one from its context pack first",
            )
        })?;
    let adapter = parse_adapter(&previous.adapter_kind)?;
    let task = state.tasks.get(&request.task_id)?;
    let plan = state.planner.current(&request.task_id)?;
    state.activity.append(
        ActivityEntry::new(
            &request.task_id,
            ActivityKind::Message,
            Actor::Human,
            local_actor(),
            text.lines().next().unwrap_or_default(),
        )
        .with_payload(json!({ "text": text })),
    )?;
    runs::launch(
        launch_context(&app, &state, adapter, &request.task_id)?,
        runs::LaunchPlan {
            agent_id: previous.agent_id.clone(),
            role: previous.role,
            digest: runs::digest_of(&text),
            prompt: text,
            resume: previous.provider_session_id.clone(),
            resumes_run_id: Some(previous.id.clone()),
            declared_scope: plan.expected_scope,
            fixed_tools: None,
            allow_write: previous.role == RunRole::Implementer,
            capabilities: previous.capabilities.clone(),
            adapter,
            task,
        },
    )
}

/// Stop a run, by the run's own id.
#[tauri::command]
async fn stop_run(run_id: String, state: State<'_, AppState>) -> Result<(), CommandFailure> {
    runs::stop(
        &run_id,
        &state.runs,
        &state.agents,
        &state.broker,
        &state.activity,
        &state.live_runs,
    )
}

/// Every run still going, across every task: what the agents overview shows.
#[tauri::command]
async fn active_runs(state: State<'_, AppState>) -> Result<Vec<runs::RunOverview>, CommandFailure> {
    let unfinished = state.runs.unfinished()?;
    runs::overview(&state.storage, unfinished, &state.live_runs)
}

/// The most recent runs in a repository, newest first, finished or not.
#[tauri::command]
async fn repository_runs(
    repository_id: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<runs::RunOverview>, CommandFailure> {
    let recent = state
        .runs
        .recent_for_repository(&repository_id, limit.unwrap_or(50).clamp(1, 500))?;
    runs::overview(&state.storage, recent, &state.live_runs)
}

/// Where each agent CLI is, and how Mars found it.
#[tauri::command]
fn agent_executables(state: State<'_, AppState>) -> Vec<runs::ExecutableView> {
    [AdapterKind::Claude, AdapterKind::Codex]
        .into_iter()
        .map(|kind| runs::executable_view(&state.executables, kind))
        .collect()
}

/// Choose an agent CLI's executable, or clear the choice to search again.
#[tauri::command]
fn set_agent_executable(
    adapter: AdapterKind,
    path: Option<PathBuf>,
    state: State<'_, AppState>,
) -> Result<runs::ExecutableView, CommandFailure> {
    if let Some(path) = &path
        && !path.is_file()
    {
        return Err(CommandFailure::new(
            "agent.executable_missing",
            format!("{} is not a file", path.display()),
        ));
    }
    state
        .executables
        .set(adapter, path)
        .map_err(|error| CommandFailure::new("agent.preference", error))?;
    Ok(runs::executable_view(&state.executables, adapter))
}

#[tauri::command]
async fn provision_task(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<WorktreePlan, CommandFailure> {
    state.tasks.provision(&task_id).map_err(Into::into)
}

#[tauri::command]
async fn remove_task_worktree(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<WorktreePlan, CommandFailure> {
    state.tasks.remove(&task_id).map_err(Into::into)
}

#[tauri::command]
async fn open_task_worktree(
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
async fn probe_agent(
    adapter: AdapterKind,
    state: State<'_, AppState>,
) -> Result<AdapterProbe, CommandFailure> {
    let executable = state.executables.resolve(adapter)?;
    adapter_for(adapter)
        .probe(&executable.path)
        .map_err(Into::into)
}

/// What the agent's CLI is signed in with, and what a run would bill.
#[tauri::command]
async fn agent_account(
    adapter: AdapterKind,
    state: State<'_, AppState>,
) -> Result<AgentAccount, CommandFailure> {
    let executable = state.executables.resolve(adapter)?.path;
    account_off_thread(adapter, executable, state.sign_in.uses_sign_in(adapter)).await
}

/// Records whether runs of `adapter` should use the CLI's own sign-in rather
/// than an API key variable that outranks it, and answers with the result.
#[tauri::command]
async fn set_agent_sign_in_preference(
    adapter: AdapterKind,
    use_sign_in: bool,
    state: State<'_, AppState>,
) -> Result<AgentAccount, CommandFailure> {
    state
        .sign_in
        .set(adapter, use_sign_in)
        .map_err(|error| CommandFailure::new("agent.preference", error.to_string()))?;
    let executable = state.executables.resolve(adapter)?.path;
    account_off_thread(adapter, executable, use_sign_in).await
}

/// Asks the CLI on a blocking thread. `claude auth status` takes a noticeable
/// fraction of a second, and the shell asks every agent when it opens.
async fn account_off_thread(
    adapter: AdapterKind,
    executable: PathBuf,
    use_sign_in: bool,
) -> Result<AgentAccount, CommandFailure> {
    tauri::async_runtime::spawn_blocking(move || {
        read_account(adapter_for(adapter).as_ref(), &executable, use_sign_in)
    })
    .await
    .map_err(|error| CommandFailure::new("agent.account", error.to_string()))?
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
    let prompt = render_context_pack(&verified_pack, &prompt);
    let executable = state.executables.resolve(adapter)?.path;
    let tools = authorize_agent(&state.permissions, &task, adapter, permissions, &executable)?;
    let resumes_run_id = match &resume_session_id {
        Some(session) => state
            .storage
            .with_connection(|connection| {
                use rusqlite::OptionalExtension;
                connection
                    .query_row(
                        "SELECT id FROM agent_runs WHERE provider_session_id = ?1
                         ORDER BY started_at DESC LIMIT 1",
                        [session],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
            })
            .map_err(|error| CommandFailure::new("agent.resume", error.to_string()))?,
        None => None,
    };
    let launched = runs::launch(
        launch_context(&app, &state, adapter, &task_id)?,
        runs::LaunchPlan {
            agent_id: adapter_name.to_owned(),
            role: RunRole::Implementer,
            digest: runs::digest_of(&prompt),
            prompt,
            resume: resume_session_id,
            resumes_run_id,
            declared_scope: Vec::new(),
            fixed_tools: Some(tools),
            allow_write: permissions.write_files,
            capabilities: json!({
                "writeFiles": permissions.write_files,
                "runCommands": permissions.run_commands,
                "network": permissions.network,
            }),
            adapter,
            task,
        },
    )?;
    // The pack is now the one the agent actually received, so this is the
    // moment its memories count as used -- not when it was previewed, and not
    // when it was rebuilt for the comparison above, either of which would
    // record influence the memory never had.
    state.memory.record_uses(
        &injections_from_pack(&verified_pack),
        &InjectionContext {
            task_id: Some(task_id.clone()),
            session_id: Some(launched.session_id.clone()),
            surface: "context_pack".into(),
        },
    )?;
    Ok(AgentLaunchOutcome {
        session_id: launched.session_id,
        adapter,
        version: launched.version,
    })
}

/// What a launch reaches, borrowed from the app state.
///
/// For Claude this also assembles the run's skills plugin: every skill that
/// applies to the task, at its pinned version.
fn launch_context<'a>(
    app: &AppHandle,
    state: &'a AppState,
    adapter: AdapterKind,
    task_id: &str,
) -> Result<runs::LaunchContext<'a>, CommandFailure> {
    let plugin_dirs = match adapter {
        AdapterKind::Claude => {
            let installations = state
                .skills
                .applicable(task_id, mars_skills::AdapterPolicy::Claude)?;
            runs::skill_plugin(&state.data_dir, &installations)?
                .into_iter()
                .collect()
        }
        AdapterKind::Codex => Vec::new(),
    };
    Ok(runs::LaunchContext {
        app: app.clone(),
        database: state.database.clone(),
        activity: &state.activity,
        runs: &state.runs,
        supervisor: &state.agents,
        broker: &state.broker,
        live: state.live_runs.clone(),
        executables: &state.executables,
        uses_sign_in: state.sign_in.uses_sign_in(adapter),
        plugin_dirs,
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

fn account_for(state: &AppState, adapter: AdapterKind) -> Result<AgentAccount, CommandFailure> {
    let executable = state.executables.resolve(adapter)?;
    read_account(
        adapter_for(adapter).as_ref(),
        &executable.path,
        state.sign_in.uses_sign_in(adapter),
    )
    .map_err(Into::into)
}

/// The variables a review or extraction run should not inherit.
///
/// Only asks the CLI when the person chose their sign-in over a key, so the
/// common case costs nothing. Any failure here leaves the environment alone:
/// the run that follows reports a missing or signed-out CLI in its own words.
fn sign_in_environment(state: &AppState, adapter: AdapterKind) -> Vec<OsString> {
    if !state.sign_in.uses_sign_in(adapter) {
        return Vec::new();
    }
    account_for(state, adapter)
        .map(|account| account.removed_environment())
        .unwrap_or_default()
}

fn signed_out(adapter: AdapterKind, account: &AgentAccount) -> CommandFailure {
    let name = match adapter {
        AdapterKind::Claude => "Claude Code",
        AdapterKind::Codex => "Codex CLI",
    };
    let how = account.sign_in_command.as_deref().map_or_else(
        || " Sign in to it from a terminal.".to_owned(),
        |command| format!(" Run `{command}` in a terminal to sign in with your own account."),
    );
    CommandFailure::new(
        "agent.signed_out",
        format!("{name} is installed but not signed in.{how}"),
    )
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

fn main() {
    // An agent CLI starts this executable as its MCP server (ADR-046). That is
    // a different program from the app: it serves one task over stdio and
    // never opens a window.
    let mut arguments = std::env::args().skip(1);
    if arguments.next().as_deref() == Some("mcp") {
        std::process::exit(mars_mcp::run_from_args(arguments));
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            fs::create_dir_all(&data_dir)?;
            let database = data_dir.join("mars.sqlite3");
            let storage = Arc::new(Storage::open(&database)?);
            let live_runs = Arc::new(runs::LiveRuns::default());
            runs::clear_skill_plugins(&data_dir);
            runs::watch_activity(app.handle().clone(), storage.clone(), live_runs.clone());
            let agents = ProcessSupervisor::new(storage.clone());
            let broker = PermissionBroker::new(storage.clone());
            let payload_root = data_dir.join("activity-payloads");
            agents.recover_interrupted()?;
            let forge = ForgeWorkflow::new(storage.clone());
            forge.recover_interrupted()?;
            let skill_registry = Arc::new(SkillsShRegistry::new()?);
            app.manage(AppState {
                activity: ActivityRecord::new(storage.clone(), &payload_root),
                agents,
                brain: BrainService::new(storage.clone()),
                broker,
                data_dir: data_dir.clone(),
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
                index_runs: Arc::new(Mutex::new(HashMap::new())),
                knowledge: KnowledgeService::new(storage.clone()),
                overlaps: TaskOverlapService::new(storage.clone()),
                planner: WorktreePlanner::new(storage.clone(), data_dir.join("worktrees"))
                    .recording_to(ActivityRecord::new(storage.clone(), &payload_root)),
                projects: ProjectService::new(storage.clone()),
                recovery: RecoveryService::new(storage.clone(), &payload_root),
                reviews: ReviewService::new(storage.clone()),
                runs: RunStore::new(storage.clone()),
                permissions: PermissionAudit::new(storage.clone()),
                sign_in: SignInPreferences::load(data_dir.join("agent-sign-in.json")),
                skills: SkillService::new(storage.clone(), skill_registry, data_dir.join("skills")),
                tasks: TaskWorktreeService::new(storage.clone(), data_dir.join("worktrees")),
                terminals: TerminalService::new(storage.clone()),
                why: WhyService::new(storage.clone()),
                storage,
                database,
                live_runs,
                executables: runs::AgentExecutables::load(data_dir.join("agent-executables.json")),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core_info,
            open_repository,
            preview_repository,
            list_files,
            read_file,
            write_file,
            spawn_terminal,
            write_terminal,
            resize_terminal,
            cancel_terminal,
            index_repository,
            cancel_index,
            index_status,
            blob_freshness,
            starting_points,
            find_symbols,
            search_code,
            search_everything,
            permission_grants,
            revoke_permission_grant,
            task_base_drift,
            resolve_task_base_drift,
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
            record_message,
            task_diff,
            task_worktree_file,
            task_activity,
            task_activity_window,
            task_audit,
            write_task_audit,
            task_projection,
            activity_payload,
            task_runs,
            pending_permissions,
            decide_permission,
            interrupted_runs,
            note_interrupted_run,
            resolve_interrupted_run,
            create_task,
            task_overlap_warnings,
            connect_github,
            import_github_issue,
            create_task_from_issue,
            preview_pull_request,
            task_commits,
            planned_checks,
            get_task_pull_request,
            list_task_forge_mutations,
            create_github_pull_request,
            refresh_github_pull_request,
            plan_task,
            task_plan,
            amend_task_plan,
            approve_task_plan,
            provision_approved_task,
            build_run_context_pack,
            provision_task,
            remove_task_worktree,
            open_task_worktree,
            probe_agent,
            agent_account,
            set_agent_sign_in_preference,
            launch_agent,
            cancel_agent,
            agent_session,
            start_run,
            continue_run,
            stop_run,
            active_runs,
            repository_runs,
            agent_executables,
            set_agent_executable,
            repository_skills,
            set_skill_enabled,
            set_skill_adapter,
            check_skill_update,
            block_skill,
            local_skills,
            scm_commands::git_status,
            scm_commands::git_diff,
            scm_commands::git_stage,
            scm_commands::git_unstage,
            scm_commands::git_discard,
            scm_commands::git_commit,
            scm_commands::git_log,
            scm_commands::git_show,
            scm_commands::git_branches,
            scm_commands::git_switch,
            scm_commands::git_create_branch,
            scm_commands::git_remotes,
            scm_commands::git_fetch,
            scm_commands::git_pull,
            scm_commands::git_push,
            scm_commands::git_stashes,
            scm_commands::git_stash_push,
            scm_commands::git_stash_pop,
            scm_commands::integration_preview,
            scm_commands::integration_apply,
            scm_commands::worktree_cleanup_plan,
            scm_commands::worktree_cleanup,
            scm_commands::worktree_restore,
            scm_commands::worktree_last_cleanup,
            forge_commands::forge_status,
            forge_commands::connect_github_cli,
            forge_commands::github_issues,
            forge_commands::github_pull_requests,
            forge_commands::github_issue,
            forge_commands::github_pull_request,
            forge_commands::github_comment,
            forge_commands::github_merge,
            forge_commands::start_task_from_issue,
            forge_commands::open_external_url,
            knowledge_commands::project_knowledge,
            knowledge_commands::resolve_knowledge
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Mars desktop");
}
