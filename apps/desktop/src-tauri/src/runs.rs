//! Agent runs from the conversation: start one from a reviewed pack, follow it
//! up, stop it, and watch every run in the project at once.
//!
//! A run is one process of an agent CLI, working in a task's worktree. The
//! first run of a task is launched from the context pack a person reviewed;
//! its digest is checked again here, so what was approved is what is sent
//! (ADR-042). Each later message in the same conversation resumes the
//! provider's own session rather than sending the pack again: the agent
//! already has it, and resending it would pay for it twice (ADR-045).
//!
//! Every run is given Mars's MCP server, scoped to its task, so the agent can
//! ask the project graph instead of searching files (ADR-046). For Claude Code
//! the same server answers its permission prompts, which is what makes a
//! pending request stop the agent before it acts (ADR-047).

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mars_activity::{
    ActivityEntry, ActivityKind, ActivityRecord, Actor, Attachment, NewRun, RunRecord, RunRole,
    RunStatus, RunStore,
};
use mars_agents::account::{Billing, read_account};
use mars_agents::adapter::{
    AdapterKind, AgentLaunchRequest, JsonLineDecoder, McpServer, NormalizedAgentEvent,
};
use mars_agents::executables::{self, ExecutableSource, ResolvedExecutable};
use mars_agents::process::{ProcessSpec, ProcessSupervisor};
use mars_agents::pump::{RunObserver, pump};
use mars_git::WorktreePlan;
use mars_security::PermissionBroker;
use mars_storage::Storage;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

use crate::CommandFailure;

// --- executables --------------------------------------------------------

/// Paths a person chose for each CLI in Settings, remembered across restarts.
pub struct AgentExecutables {
    file: PathBuf,
    chosen: Mutex<HashMap<String, PathBuf>>,
}

impl AgentExecutables {
    pub fn load(file: PathBuf) -> Self {
        let chosen = fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self {
            file,
            chosen: Mutex::new(chosen),
        }
    }

    pub fn chosen(&self, kind: AdapterKind) -> Option<PathBuf> {
        self.chosen
            .lock()
            .ok()
            .and_then(|chosen| chosen.get(executables::command_name(kind)).cloned())
    }

    pub fn set(&self, kind: AdapterKind, path: Option<PathBuf>) -> Result<(), String> {
        let mut chosen = self
            .chosen
            .lock()
            .map_err(|_| "preferences are unavailable")?;
        match path {
            Some(path) => {
                chosen.insert(executables::command_name(kind).to_owned(), path);
            }
            None => {
                chosen.remove(executables::command_name(kind));
            }
        }
        let text = serde_json::to_string_pretty(&*chosen).map_err(|error| error.to_string())?;
        if let Some(parent) = self.file.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&self.file, text).map_err(|error| error.to_string())
    }

    /// The CLI to run for `kind`, or a failure that says where Mars looked.
    pub fn resolve(&self, kind: AdapterKind) -> Result<ResolvedExecutable, CommandFailure> {
        executables::resolve(kind, self.chosen(kind).as_deref()).map_err(|missing| {
            let name = match kind {
                AdapterKind::Claude => "Claude Code",
                AdapterKind::Codex => "Codex CLI",
            };
            CommandFailure::new(
                "agent.not_installed",
                format!(
                    "{name} is not installed, or Mars cannot find it. Install it, or choose its \
                     executable in Settings. Looked in {} place(s), including PATH.",
                    missing.searched.len()
                ),
            )
        })
    }
}

/// What Settings shows for one CLI.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutableView {
    pub adapter: AdapterKind,
    pub path: Option<PathBuf>,
    pub source: Option<ExecutableSource>,
    pub chosen: Option<PathBuf>,
    pub searched: Vec<String>,
}

pub fn executable_view(executables: &AgentExecutables, kind: AdapterKind) -> ExecutableView {
    let chosen = executables.chosen(kind);
    match executables::resolve(kind, chosen.as_deref()) {
        Ok(found) => ExecutableView {
            adapter: kind,
            path: Some(found.path),
            source: Some(found.source),
            chosen,
            searched: Vec::new(),
        },
        Err(missing) => ExecutableView {
            adapter: kind,
            path: None,
            source: None,
            chosen,
            searched: missing.searched,
        },
    }
}

// --- skills, loaded the way the CLI loads its own -----------------------

/// A Claude Code plugin holding the skills one run is given.
///
/// Claude Code loads a plugin directory passed with `--plugin-dir` for that
/// session only, and the skills inside it load exactly as the person's own do:
/// listed by description, read in full when used, their scripts and
/// references beside them. The plugin is a copy of each installation's pinned
/// files, so what the agent reads is the version a person trusted, and it
/// lives under the app's data folder, never in the worktree, so nothing
/// appears in the task's diff.
pub fn skill_plugin(
    data_dir: &Path,
    installations: &[mars_skills::SkillInstallation],
) -> Result<Option<PathBuf>, CommandFailure> {
    if installations.is_empty() {
        return Ok(None);
    }
    let failure = |error: std::io::Error| CommandFailure::new("skills.plugin", error.to_string());
    let root = data_dir
        .join("run-skills")
        .join(uuid_like())
        .join("mars-skills");
    fs::create_dir_all(root.join(".claude-plugin")).map_err(failure)?;
    fs::write(
        root.join(".claude-plugin").join("plugin.json"),
        serde_json::to_string_pretty(&json!({
            "name": "mars-skills",
            "description": "Skills a person trusted in Mars for this task",
            "version": "1.0.0",
        }))
        .unwrap_or_default(),
    )
    .map_err(failure)?;
    let mut used = std::collections::HashSet::new();
    for installation in installations {
        let mut name = folder_name(&installation.name);
        // Two installations with the same name keep both, distinctly.
        while !used.insert(name.clone()) {
            name.push('-');
            name.push_str(&installation.pinned_hash[..6.min(installation.pinned_hash.len())]);
        }
        copy_tree(&installation.install_path, &root.join("skills").join(name)).map_err(failure)?;
    }
    Ok(Some(root))
}

/// Remove the plugins of runs that no longer exist. Runs do not survive a
/// restart, so at startup every one of them is stale.
pub fn clear_skill_plugins(data_dir: &Path) {
    let _ = fs::remove_dir_all(data_dir.join("run-skills"));
}

fn folder_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "skill".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn uuid_like() -> String {
    format!(
        "{:x}-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default(),
        std::process::id()
    )
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

// --- the runs this process is driving -----------------------------------

#[derive(Clone, Debug)]
struct LiveRun {
    task_id: String,
    session_id: String,
}

/// Runs whose processes this app started and is still reading.
#[derive(Default)]
pub struct LiveRuns {
    runs: Mutex<HashMap<String, LiveRun>>,
}

impl LiveRuns {
    fn insert(&self, run_id: &str, run: LiveRun) {
        if let Ok(mut runs) = self.runs.lock() {
            runs.insert(run_id.to_owned(), run);
        }
    }

    fn remove(&self, run_id: &str) {
        if let Ok(mut runs) = self.runs.lock() {
            runs.remove(run_id);
        }
    }

    fn get(&self, run_id: &str) -> Option<LiveRun> {
        self.runs
            .lock()
            .ok()
            .and_then(|runs| runs.get(run_id).cloned())
    }

    /// The run currently working in this task, if any.
    pub fn for_task(&self, task_id: &str) -> Option<String> {
        self.runs.lock().ok().and_then(|runs| {
            runs.iter()
                .find(|(_, run)| run.task_id == task_id)
                .map(|(run_id, _)| run_id.clone())
        })
    }

    fn tasks(&self) -> Vec<String> {
        self.runs
            .lock()
            .map(|runs| {
                let mut tasks: Vec<String> = runs.values().map(|run| run.task_id.clone()).collect();
                tasks.sort();
                tasks.dedup();
                tasks
            })
            .unwrap_or_default()
    }
}

// --- launching ----------------------------------------------------------

/// Everything one launch needs, whichever path asked for it.
pub struct LaunchPlan {
    pub task: WorktreePlan,
    pub adapter: AdapterKind,
    pub role: RunRole,
    pub agent_id: String,
    /// Exactly what the agent is sent.
    pub prompt: String,
    /// SHA-256 of `prompt`.
    pub digest: String,
    /// The provider session to resume.
    pub resume: Option<String>,
    /// The run that left that session, where Mars knows it.
    pub resumes_run_id: Option<String>,
    pub declared_scope: Vec<String>,
    /// Older launch path: the tools a person ticked, with nobody to ask about
    /// anything else. `None` routes everything else through the permission
    /// prompt instead.
    pub fixed_tools: Option<Vec<String>>,
    pub allow_write: bool,
    pub capabilities: Value,
}

/// The dependencies a launch reaches. Borrowed from the app state so this
/// module does not need to know its shape.
pub struct LaunchContext<'a> {
    pub app: AppHandle,
    pub database: PathBuf,
    pub activity: &'a ActivityRecord,
    pub runs: &'a RunStore,
    pub supervisor: &'a ProcessSupervisor,
    pub broker: &'a PermissionBroker,
    pub live: Arc<LiveRuns>,
    pub executables: &'a AgentExecutables,
    pub uses_sign_in: bool,
    /// Skill folders loaded for this run, which the agent may read.
    pub plugin_dirs: Vec<PathBuf>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchedRun {
    pub run_id: String,
    pub session_id: String,
    pub adapter: AdapterKind,
    pub version: String,
    pub billing: Billing,
}

pub fn launch(context: LaunchContext<'_>, plan: LaunchPlan) -> Result<LaunchedRun, CommandFailure> {
    if plan.task.state != mars_git::WorktreeState::Ready {
        return Err(CommandFailure::new(
            "agent.worktree_not_ready",
            "approve and provision the task's worktree before starting an agent in it",
        ));
    }
    // One agent changes a worktree at a time (ADR-005): two writing to the same
    // files at once would each be working from a tree the other is changing.
    if context.live.for_task(&plan.task.task_id).is_some() {
        return Err(CommandFailure::new(
            "agent.busy",
            "an agent is already working in this task; wait for it to finish or stop it first",
        ));
    }
    let adapter = crate::adapter_for(plan.adapter);
    let executable = context.executables.resolve(plan.adapter)?;
    let probe = adapter.probe(&executable.path)?;
    // Asked before anything is recorded: a CLI nobody is signed in to would
    // start, fail on its first request, and leave a run behind that never did
    // anything.
    let account = read_account(adapter.as_ref(), &executable.path, context.uses_sign_in)?;
    if account.billing == Billing::SignedOut {
        return Err(crate::signed_out(plan.adapter, &account));
    }

    let adapter_name = crate::adapter_name(plan.adapter);
    let run = context.runs.start(NewRun {
        task_id: plan.task.task_id.clone(),
        agent_id: plan.agent_id.clone(),
        adapter_kind: adapter_name.to_owned(),
        role: plan.role,
        capabilities: plan.capabilities.clone(),
        context_pack_digest: Some(plan.digest.clone()),
        resumes_run_id: plan.resumes_run_id.clone(),
    })?;

    let agent_label = format!("{adapter_name} ({})", plan.role.as_str());
    let mcp = mcp_server(&context, &plan, &run.id, &agent_label);
    let mut request = AgentLaunchRequest::new(
        executable.path.clone(),
        plan.task.worktree_path.clone(),
        plan.prompt.clone(),
    );
    request.resume_session_id = plan.resume.clone();
    request.allow_write = plan.allow_write;
    request.mcp_servers = vec![mcp];
    request.plugin_dirs = context.plugin_dirs.clone();
    match &plan.fixed_tools {
        Some(tools) => request.allowed_tools = tools.clone(),
        None => {
            // Reading, and organising its own work, needs nobody's answer.
            // Everything else reaches the permission prompt.
            request.allowed_tools = [
                "Read",
                "Glob",
                "Grep",
                "LS",
                "NotebookRead",
                "TodoWrite",
                "Task",
                "Skill",
            ]
            .map(str::to_owned)
            .to_vec();
            if plan.adapter == AdapterKind::Claude {
                request.permission_prompt_tool = Some("mcp__mars__permission_prompt".into());
            }
        }
    }
    let command = adapter.command(&request).inspect_err(|_| {
        let _ = context
            .runs
            .fail(&run.id, "the launch command could not be built");
    })?;
    let mut spec = ProcessSpec::from_command(adapter_name, command);
    spec.task_id = Some(plan.task.task_id.clone());
    spec.removed_environment = account.removed_environment();

    let launched = match context.supervisor.launch(spec) {
        Ok(launched) => launched,
        Err(error) => {
            let _ = context.runs.fail(&run.id, &error.to_string());
            return Err(error.into());
        }
    };
    let session_id = launched.session_id.clone();
    let _ = context.runs.attach_session(&run.id, &session_id);
    context.live.insert(
        &run.id,
        LiveRun {
            task_id: plan.task.task_id.clone(),
            session_id: session_id.clone(),
        },
    );
    let _ = context.activity.append(
        ActivityEntry::new(
            &plan.task.task_id,
            ActivityKind::RunStarted,
            Actor::System,
            "mars",
            match &plan.resume {
                Some(_) => format!("Continued with {adapter_name}"),
                None => format!("Started {adapter_name} as {}", plan.role.as_str()),
            },
        )
        .in_run(&run.id)
        .with_payload(json!({
            "adapter": adapter_name,
            "role": plan.role.as_str(),
            "billing": account.billing,
            "contextPackDigest": plan.digest,
            "resumesRunId": plan.resumes_run_id,
            "executable": executable.path,
            "executableSource": executable.source,
            "version": probe.version,
        }))
        // The prompt is the whole of what the agent was told, so the log
        // keeps it verbatim (ADR-042).
        .with_attachment(Attachment::text("text/plain", plan.prompt.clone())),
    );

    let sink = RunSink {
        app: context.app.clone(),
        activity: context.activity.clone(),
        runs: context.runs.clone(),
        broker: context.broker.clone(),
        live: context.live.clone(),
        task_id: plan.task.task_id.clone(),
        run_id: run.id.clone(),
        session_id: session_id.clone(),
        agent_label,
        worktree: plan.task.worktree_path.clone(),
        calls: HashMap::new(),
        running: false,
    };
    tauri::async_runtime::spawn(pump(context.supervisor.clone(), adapter, launched, sink));
    let _ = context.app.emit(
        "run-started",
        json!({ "taskId": plan.task.task_id, "runId": run.id }),
    );

    Ok(LaunchedRun {
        run_id: run.id,
        session_id,
        adapter: plan.adapter,
        version: probe.version,
        billing: account.billing,
    })
}

/// The MCP server one run is given: this executable, serving `mcp` for the
/// run's task.
fn mcp_server(
    context: &LaunchContext<'_>,
    plan: &LaunchPlan,
    run_id: &str,
    agent_label: &str,
) -> McpServer {
    let config = mars_mcp::ServerConfig {
        database: context.database.clone(),
        repository_id: plan.task.repository_id.clone(),
        task_id: Some(plan.task.task_id.clone()),
        run_id: Some(run_id.to_owned()),
        worktree: Some(plan.task.worktree_path.clone()),
        role: plan.role.as_str().to_owned(),
        scope: plan.declared_scope.clone(),
        agent: agent_label.to_owned(),
        readable: context.plugin_dirs.clone(),
        permission_tool: plan.adapter == AdapterKind::Claude && plan.fixed_tools.is_none(),
    };
    let mut arguments = vec!["mcp".to_owned()];
    arguments.extend(config.to_args());
    McpServer {
        name: "mars".into(),
        command: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("mars-desktop")),
        arguments,
        environment: Vec::new(),
    }
}

#[must_use]
pub fn digest_of(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

// --- reading a run into the task's log ----------------------------------

#[derive(Clone)]
enum CallKind {
    Command(String),
    Test(String),
    File,
    Tool,
}

/// Where one run's events are written down: the task's activity record, the
/// run row, and the screen.
struct RunSink {
    app: AppHandle,
    activity: ActivityRecord,
    runs: RunStore,
    broker: PermissionBroker,
    live: Arc<LiveRuns>,
    task_id: String,
    run_id: String,
    session_id: String,
    agent_label: String,
    worktree: PathBuf,
    /// What each open call was, so its result can be recorded against it.
    calls: HashMap<String, CallKind>,
    running: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentEventPayload<'a> {
    task_id: &'a str,
    run_id: &'a str,
    session_id: &'a str,
    event: &'a NormalizedAgentEvent,
}

impl RunObserver for RunSink {
    fn event(&mut self, event: NormalizedAgentEvent, stopping: bool) {
        if !self.running {
            self.running = true;
            let _ = self.runs.set_status(&self.run_id, RunStatus::Running);
        }
        if let NormalizedAgentEvent::End { success, usage, .. } = &event {
            if let Some(usage) = usage
                && let Ok(value) = serde_json::to_value(usage)
            {
                let _ = self.runs.set_usage(&self.run_id, &value);
            }
            // Settled before the ending is recorded, so anything reacting to
            // the ending never reads a stale status (ADR-039).
            let status = if *success {
                RunStatus::Completed
            } else if stopping {
                RunStatus::Cancelled
            } else {
                RunStatus::Failed
            };
            let _ = self.runs.set_status(&self.run_id, status);
            let _ = self
                .broker
                .withdraw_for_run(&self.run_id, "the run ended before anyone answered");
            self.live.remove(&self.run_id);
        }
        if let Some(entry) = self.entry(&event, stopping) {
            let _ = self.activity.append(entry.in_run(&self.run_id));
        }
        let _ = self.app.emit(
            "agent-event",
            AgentEventPayload {
                task_id: &self.task_id,
                run_id: &self.run_id,
                session_id: &self.session_id,
                event: &event,
            },
        );
        if matches!(event, NormalizedAgentEvent::End { .. }) {
            let _ = self.app.emit(
                "run-finished",
                json!({ "taskId": self.task_id, "runId": self.run_id }),
            );
        }
    }
}

impl RunSink {
    fn new_entry(
        &self,
        kind: ActivityKind,
        actor: Actor,
        summary: impl Into<String>,
    ) -> ActivityEntry {
        let label = match actor {
            Actor::System => "mars",
            _ => self.agent_label.as_str(),
        };
        ActivityEntry::new(&self.task_id, kind, actor, label, summary)
    }

    /// A path as the transcript shows it: relative to the worktree.
    fn relative(&self, path: &str) -> String {
        let normalized = path.replace('\\', "/");
        let root = self.worktree.to_string_lossy().replace('\\', "/");
        let root = root.trim_end_matches('/');
        let lower = |text: &str| {
            if cfg!(windows) {
                text.to_ascii_lowercase()
            } else {
                text.to_owned()
            }
        };
        if lower(&normalized).starts_with(&format!("{}/", lower(root))) {
            return normalized[root.len() + 1..].to_owned();
        }
        normalized
    }

    fn entry(&mut self, event: &NormalizedAgentEvent, stopping: bool) -> Option<ActivityEntry> {
        Some(match event {
            NormalizedAgentEvent::Checkpoint { session_id } => {
                let _ = self.runs.set_provider_session(&self.run_id, session_id);
                return None;
            }
            NormalizedAgentEvent::Text { text } => self
                .new_entry(
                    ActivityKind::Message,
                    Actor::Agent,
                    text.lines().next().unwrap_or_default(),
                )
                .with_payload(json!({ "text": text })),
            NormalizedAgentEvent::Tool {
                name,
                input,
                call_id,
            } => {
                if let Some(call_id) = call_id {
                    self.calls.insert(call_id.clone(), CallKind::Tool);
                }
                self.new_entry(ActivityKind::ToolCall, Actor::Agent, name.clone())
                    .with_payload(json!({ "name": name, "input": input, "callId": call_id }))
            }
            NormalizedAgentEvent::Command {
                command,
                status,
                call_id,
            } => {
                if let Some(call_id) = call_id {
                    self.calls
                        .insert(call_id.clone(), CallKind::Command(command.clone()));
                }
                self.new_entry(ActivityKind::Command, Actor::Agent, command.clone())
                    .with_payload(
                        json!({ "command": command, "status": status, "callId": call_id }),
                    )
            }
            NormalizedAgentEvent::Test {
                command,
                status,
                call_id,
            } => {
                if let Some(call_id) = call_id {
                    self.calls
                        .insert(call_id.clone(), CallKind::Test(command.clone()));
                }
                self.new_entry(ActivityKind::TestRun, Actor::Agent, command.clone())
                    .with_payload(
                        json!({ "command": command, "status": status, "callId": call_id }),
                    )
            }
            NormalizedAgentEvent::FileChange {
                path,
                change,
                call_id,
            } => {
                if let Some(call_id) = call_id {
                    self.calls.insert(call_id.clone(), CallKind::File);
                }
                let path = self.relative(path);
                self.new_entry(
                    ActivityKind::FileEdit,
                    Actor::Agent,
                    format!("{change} {path}"),
                )
                .with_payload(json!({ "path": path, "change": change, "callId": call_id }))
            }
            NormalizedAgentEvent::ToolResult {
                call_id,
                output,
                is_error,
                exit_code,
            } => {
                let (kind, command, status) = match self.calls.remove(call_id)? {
                    CallKind::Command(command) => (
                        ActivityKind::Command,
                        command,
                        if *is_error { "failed" } else { "completed" },
                    ),
                    CallKind::Test(command) => (
                        ActivityKind::TestRun,
                        command,
                        if *is_error { "failed" } else { "passed" },
                    ),
                    // An edit or a read that came back is the agent's own
                    // business; what changed is already in the log.
                    CallKind::File | CallKind::Tool => return None,
                };
                let mut entry = self
                    .new_entry(kind, Actor::Agent, command.clone())
                    .with_payload(json!({
                        "command": command,
                        "status": status,
                        "callId": call_id,
                        "exitCode": exit_code,
                    }));
                if !output.is_empty() {
                    entry = entry.with_attachment(Attachment::text("text/plain", output.clone()));
                }
                entry
            }
            NormalizedAgentEvent::Permission { tool, description } => self
                .new_entry(ActivityKind::Message, Actor::System, description.clone())
                .with_payload(json!({ "text": description, "tool": tool, "notice": true })),
            NormalizedAgentEvent::Warning { message } => self
                .new_entry(ActivityKind::Message, Actor::System, message.clone())
                .with_payload(json!({ "text": message, "notice": true })),
            // Kept, because the log is the record of what happened including
            // what Mars could not read; marked, because it says nothing about
            // whether the run went well.
            NormalizedAgentEvent::Unknown { provider_type, raw } => self
                .new_entry(
                    ActivityKind::Error,
                    Actor::System,
                    format!("unrecognised {provider_type} output"),
                )
                .with_payload(json!({ "providerType": provider_type, "diagnostic": true }))
                .with_attachment(Attachment::text("application/json", raw.to_string())),
            NormalizedAgentEvent::End {
                success,
                message,
                usage,
            } => {
                let stopped = !success && stopping;
                let summary = match (success, stopped, message) {
                    (_, true, _) => "Stopped before it finished".to_owned(),
                    (true, _, _) => "Finished".to_owned(),
                    (false, _, Some(message)) => {
                        message.lines().next().unwrap_or("Failed").to_owned()
                    }
                    (false, _, None) => "Failed".to_owned(),
                };
                self.new_entry(ActivityKind::Done, Actor::Agent, summary)
                    .with_payload(json!({
                        "success": success,
                        "stopped": stopped,
                        "message": message,
                        "usage": usage,
                    }))
            }
        })
    }
}

// --- stopping -----------------------------------------------------------

/// Stop a run. A run this process is reading is asked to stop and settles
/// itself; one that was left behind is settled here.
pub fn stop(
    run_id: &str,
    runs: &RunStore,
    supervisor: &ProcessSupervisor,
    broker: &PermissionBroker,
    activity: &ActivityRecord,
    live: &LiveRuns,
) -> Result<(), CommandFailure> {
    let _ = broker.withdraw_for_run(run_id, "the run was stopped");
    if let Some(run) = live.get(run_id)
        && supervisor.cancel(&run.session_id).is_ok()
    {
        return Ok(());
    }
    let record = runs.get(run_id)?;
    if record.status.is_terminal() {
        return Ok(());
    }
    // Nothing is reading this run: its process is gone. Say so in the log
    // rather than leaving it looking alive.
    runs.set_status(run_id, RunStatus::Cancelled)?;
    activity.append(
        ActivityEntry::new(
            &record.task_id,
            ActivityKind::Done,
            Actor::System,
            "mars",
            "Stopped; its process was no longer running",
        )
        .in_run(run_id)
        .with_payload(json!({ "success": false, "stopped": true })),
    )?;
    Ok(())
}

// --- the overview -------------------------------------------------------

/// One run, with the task it belongs to, as the agents overview shows it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunOverview {
    #[serde(flatten)]
    pub run: RunRecord,
    pub task_title: String,
    pub branch_name: String,
    pub worktree_path: PathBuf,
    /// Whether this process is still reading it.
    pub live: bool,
    pub pending_permissions: usize,
    pub files_touched: usize,
}

pub fn overview(
    storage: &Storage,
    runs: Vec<RunRecord>,
    live: &LiveRuns,
) -> Result<Vec<RunOverview>, CommandFailure> {
    runs.into_iter()
        .map(|run| {
            let (task_title, branch_name, worktree_path): (String, String, String) = storage
                .with_connection(|connection| {
                    connection.query_row(
                        "SELECT t.title, COALESCE(b.name, ''), COALESCE(w.path, '')
                         FROM tasks t
                         LEFT JOIN worktrees w ON w.task_id = t.id
                         LEFT JOIN branches b ON b.id = w.branch_id
                         WHERE t.id = ?1",
                        [&run.task_id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                })
                .map_err(|error| CommandFailure::new("runs.overview", error.to_string()))?;
            let (pending, files): (i64, i64) = storage
                .with_connection(|connection| {
                    connection.query_row(
                        "SELECT
                           (SELECT COUNT(*) FROM permission_requests WHERE run_id = ?1 AND state = 'pending'),
                           (SELECT COUNT(DISTINCT json_extract(payload_json, '$.path'))
                              FROM activity_events WHERE run_id = ?1 AND kind = 'file_edit')",
                        [&run.id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                })
                .map_err(|error| CommandFailure::new("runs.overview", error.to_string()))?;
            Ok(RunOverview {
                live: live.get(&run.id).is_some(),
                pending_permissions: usize::try_from(pending).unwrap_or_default(),
                files_touched: usize::try_from(files).unwrap_or_default(),
                task_title,
                branch_name,
                worktree_path: PathBuf::from(worktree_path),
                run,
            })
        })
        .collect()
}

// --- watching -----------------------------------------------------------

/// Tell the screen when a running task's log grows from outside this process.
///
/// An agent's MCP server writes to the log from its own process — a permission
/// request, most importantly, which arrives while the agent's own stream is
/// silent because it is waiting on the answer. Nothing in this process would
/// otherwise notice, and a question nobody sees is never answered.
pub fn watch_activity(app: AppHandle, storage: Arc<Storage>, live: Arc<LiveRuns>) {
    std::thread::spawn(move || {
        let mut seen: HashMap<String, i64> = HashMap::new();
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let tasks = live.tasks();
            seen.retain(|task_id, _| tasks.contains(task_id));
            for task_id in tasks {
                let head: i64 = storage
                    .with_connection(|connection| {
                        connection.query_row(
                            "SELECT COALESCE(MAX(sequence), 0) FROM activity_events WHERE task_id = ?1",
                            [&task_id],
                            |row| row.get(0),
                        )
                    })
                    .unwrap_or(0);
                if seen.get(&task_id) != Some(&head) {
                    seen.insert(task_id.clone(), head);
                    let _ = app.emit(
                        "activity-changed",
                        json!({ "taskId": task_id, "lastSequence": head }),
                    );
                }
            }
        }
    });
}

// --- one-shot runs: reviews and memory extraction ------------------------

/// Run an agent to completion on one prompt and return its last message.
/// Used where the answer is a document — a review, an extraction — rather
/// than a conversation.
pub fn run_once(
    adapter_kind: AdapterKind,
    executable: &Path,
    cwd: &Path,
    prompt: &str,
    removed_environment: &[OsString],
) -> Result<String, String> {
    let adapter = crate::adapter_for(adapter_kind);
    let command = adapter
        .command(&AgentLaunchRequest::new(executable, cwd, prompt))
        .map_err(|error| error.to_string())?;
    let output = command
        .output(removed_environment)
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
            NormalizedAgentEvent::Text { text } => Some(crate::strip_json_fence(&text)),
            _ => None,
        })
        .next_back()
        .ok_or_else(|| "the agent returned no answer".into())
}

/// Input for `start_run`: the pack a person reviewed, by digest.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRunInput {
    pub task_id: String,
    pub agent_id: String,
    pub adapter: AdapterKind,
    pub role: crate::RunRoleInput,
    pub assignment: String,
    pub max_bytes: usize,
    #[serde(default)]
    pub excluded_entity_refs: Vec<String>,
    /// The digest of the pack the person approved.
    pub digest: String,
}

/// Input for `continue_run`: the next thing the person said.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinueRunInput {
    pub task_id: String,
    pub text: String,
}
