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
use mars_brain::{BrainError, BrainService};
use mars_core::Core;
use mars_core::files::{FileDocument, FileError, FilePage, FileService};
use mars_core::projects::{OpenRepositoryOutcome, ProjectError, ProjectService};
use mars_core::protocol::{CommandRequest, CommandResponse};
use mars_core::terminal::{TerminalError, TerminalService, TerminalSessionInfo};
use mars_git::{TaskWorktreeError, TaskWorktreeService, WorktreePlan};
use mars_indexer::{IndexError, IndexSummary, Indexer};
use mars_security::TrustDecision;
use mars_security::permissions::{
    Capability, PermissionDecision, PermissionEngine, PermissionError, PermissionOutcome,
    PermissionRequest, PermissionScope,
};
use mars_storage::Storage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    agents: ProcessSupervisor,
    brain: BrainService,
    files: FileService,
    indexer: Indexer,
    projects: ProjectService,
    permissions: PermissionEngine,
    tasks: TaskWorktreeService,
    terminals: TerminalService,
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

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentPermissions {
    write_files: bool,
    run_commands: bool,
    network: bool,
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
    task_id: String,
    adapter: AdapterKind,
    prompt: String,
    resume_session_id: Option<String>,
    permissions: AgentPermissions,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AgentLaunchOutcome, CommandFailure> {
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
    stream_agent_events(app, adapter_impl, launched);
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

fn authorize_agent(
    engine: &PermissionEngine,
    task: &WorktreePlan,
    adapter: AdapterKind,
    permissions: AgentPermissions,
    executable: &Path,
) -> Result<Vec<String>, CommandFailure> {
    let mut tools = vec!["Read".to_owned()];
    record_permission(
        engine,
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
        engine,
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
        engine,
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
        engine,
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
    engine: &PermissionEngine,
    request: PermissionRequest,
    allowed: bool,
) -> Result<bool, CommandFailure> {
    engine.record_decision(
        &request,
        if allowed {
            PermissionDecision::AllowSession
        } else {
            PermissionDecision::Deny
        },
    )?;
    Ok(matches!(
        engine.evaluate(&request, None)?,
        PermissionOutcome::Allowed(_)
    ))
}

fn stream_agent_events(
    app: AppHandle,
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
            app.manage(AppState {
                agents,
                brain: BrainService::new(storage.clone()),
                files: FileService::new(storage.clone()),
                indexer: Indexer::new(storage.clone()),
                projects: ProjectService::new(storage.clone()),
                permissions: PermissionEngine::new(),
                tasks: TaskWorktreeService::new(storage.clone(), data_dir.join("worktrees")),
                terminals: TerminalService::new(storage),
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
            list_tasks,
            create_task,
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
