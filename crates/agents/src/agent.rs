//! One `Agent` interface over every backend.
//!
//! Adapters already normalize provider frames into a shared vocabulary. This
//! layer turns that into something the rest of Mars can hold: an agent selected
//! per task *and per role*, whose capabilities are decided when it is built
//! rather than requested in its prompt.
//!
//! The role is the load-bearing part. A reviewer is constructed with a
//! read-only [`CapabilityCeiling`], and that ceiling is handed to the permission
//! broker with every request the agent makes. A reviewing agent cannot write,
//! not because it was told not to, but because there is no answer a human can
//! give that would let it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use mars_activity::{
    ActivityEntry, ActivityKind, ActivityRecord, Actor, Attachment, NewRun, RunRole, RunStatus,
    RunStore,
};
use mars_security::{
    BrokerDecision, BrokerOutcome, BrokerRequest, CapabilityCeiling, PermissionBroker, SideEffect,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::sync::mpsc;

use crate::adapter::{
    AdapterError, AdapterKind, AgentAdapter, AgentLaunchRequest, JsonLineDecoder,
    NormalizedAgentEvent,
};
use crate::process::{ProcessError, ProcessEventKind, ProcessSpec, ProcessSupervisor};

/// What an agent has been brought in to do.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    /// Does the work: may write, run commands, and ask about anything else.
    Implementer,
    /// Judges the work: reads and reports, and can do nothing else.
    Reviewer,
}

impl AgentRole {
    #[must_use]
    pub const fn ceiling(self) -> CapabilityCeiling {
        match self {
            Self::Implementer => CapabilityCeiling::implementer(),
            Self::Reviewer => CapabilityCeiling::reviewer(),
        }
    }

    const fn as_run_role(self) -> RunRole {
        match self {
            Self::Implementer => RunRole::Implementer,
            Self::Reviewer => RunRole::Reviewer,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Implementer => "implementer",
            Self::Reviewer => "reviewer",
        }
    }
}

/// What this agent can do, as a fact about how it was constructed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub adapter: AdapterKind,
    pub role: AgentRole,
    pub ceiling: CapabilityCeiling,
    pub supports_resume: bool,
    pub supports_interrupt: bool,
}

impl AgentCapabilities {
    #[must_use]
    pub const fn new(adapter: AdapterKind, role: AgentRole, supports_resume: bool) -> Self {
        Self {
            adapter,
            role,
            ceiling: role.ceiling(),
            supports_resume: supports_resume && matches!(role, AgentRole::Implementer),
            supports_interrupt: true,
        }
    }

    /// Whether this agent may edit the worktree at all. Used when the launch
    /// command is built, so the ceiling reaches the process itself and not only
    /// the broker.
    #[must_use]
    pub const fn may_write(&self) -> bool {
        self.ceiling.write_files
    }
}

/// The task an agent is being pointed at.
#[derive(Clone, Debug)]
pub struct TaskSpec {
    pub task_id: String,
    pub title: String,
    pub worktree_path: PathBuf,
    /// The globs the approved worktree plan declared. A write outside them is a
    /// permission request, not an edit.
    pub expected_scope: Vec<String>,
}

/// Exactly what the agent will see. Built by the context-pack builder and shown
/// to the human verbatim before the run starts.
#[derive(Clone, Debug)]
pub struct AgentContext {
    pub prompt: String,
    pub pack_digest: Option<String>,
    pub resume_session_id: Option<String>,
    pub allowed_tools: Vec<String>,
}

/// The normalized event vocabulary every backend is mapped onto.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum AgentEvent {
    Message {
        text: String,
    },
    Thought {
        text: String,
    },
    ToolCall {
        name: String,
        input: Value,
    },
    Command {
        command: String,
        status: String,
    },
    FileEdit {
        path: String,
        change: String,
    },
    TestRun {
        command: String,
        status: String,
    },
    PermissionRequest {
        request_id: String,
        summary: String,
        classification: String,
    },
    Error {
        message: String,
    },
    Done {
        success: bool,
        message: Option<String>,
    },
    /// A provider resume point. Kept out of the nine because it is machinery,
    /// not something that happened.
    Checkpoint {
        session_id: String,
    },
    /// A frame no adapter recognised, preserved rather than dropped (ADR-006).
    Unknown {
        provider_type: String,
        raw: Value,
    },
}

impl AgentEvent {
    const fn activity_kind(&self) -> Option<ActivityKind> {
        match self {
            Self::Message { .. } => Some(ActivityKind::Message),
            Self::Thought { .. } => Some(ActivityKind::Thought),
            Self::ToolCall { .. } => Some(ActivityKind::ToolCall),
            Self::Command { .. } => Some(ActivityKind::Command),
            Self::FileEdit { .. } => Some(ActivityKind::FileEdit),
            Self::TestRun { .. } => Some(ActivityKind::TestRun),
            Self::PermissionRequest { .. } => Some(ActivityKind::PermissionRequest),
            Self::Error { .. } => Some(ActivityKind::Error),
            Self::Done { .. } => Some(ActivityKind::Done),
            // Checkpoints and unrecognised frames are provider machinery. They
            // are kept on the session record, not in the task's account of what
            // happened.
            Self::Checkpoint { .. } | Self::Unknown { .. } => None,
        }
    }

    fn summary(&self) -> String {
        match self {
            Self::Message { text } | Self::Thought { text } => truncate(text, 200),
            Self::ToolCall { name, .. } => format!("called {name}"),
            Self::Command { command, status } => format!("{command} ({status})"),
            Self::FileEdit { path, change } => format!("{change} {path}"),
            Self::TestRun { command, status } => format!("{command}: {status}"),
            Self::PermissionRequest { summary, .. } => summary.clone(),
            Self::Error { message } => truncate(message, 200),
            Self::Done { success, message } => match message {
                Some(message) => truncate(message, 200),
                None if *success => "finished".into(),
                None => "ended without success".into(),
            },
            Self::Checkpoint { session_id } => format!("checkpoint {session_id}"),
            Self::Unknown { provider_type, .. } => format!("unrecognised frame {provider_type}"),
        }
    }
}

/// Where a run has got to.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub run_id: String,
    pub session_id: Option<String>,
    pub state: RunStatus,
    pub last_sequence: i64,
    /// Set while the agent is stopped waiting for a permission decision.
    pub blocked_on_request: Option<String>,
}

/// A started run: its identity, and its stream of normalized events.
pub struct AgentHandle {
    pub run_id: String,
    pub session_id: String,
    pub events: mpsc::Receiver<AgentEvent>,
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error(transparent)]
    Adapter(#[from] AdapterError),
    #[error(transparent)]
    Process(#[from] ProcessError),
    #[error("activity record failed: {0}")]
    Activity(String),
    #[error("permission broker failed: {0}")]
    Permission(String),
    #[error("run `{0}` is not active")]
    RunNotActive(String),
    #[error("agent state lock was poisoned")]
    StatePoisoned,
}

impl From<mars_activity::ActivityError> for AgentError {
    fn from(error: mars_activity::ActivityError) -> Self {
        Self::Activity(error.to_string())
    }
}

impl From<mars_security::permissions::PermissionError> for AgentError {
    fn from(error: mars_security::permissions::PermissionError) -> Self {
        Self::Permission(error.to_string())
    }
}

/// The interface every backend is reached through.
pub trait Agent: Send + Sync {
    /// A stable identity for this agent as configured, e.g. `claude:reviewer`.
    fn id(&self) -> &str;

    fn kind(&self) -> AdapterKind;

    /// What this agent can do. Decided at construction; never widened by a run.
    fn capabilities(&self) -> AgentCapabilities;

    /// Begin work. The returned handle streams normalized events; the same
    /// events are written to the task's activity record first, so the log is
    /// never behind what a UI has seen.
    fn start(&self, task: &TaskSpec, context: &AgentContext) -> Result<AgentHandle, AgentError>;

    /// Answer a permission request this agent is blocked on.
    fn respond_to_permission(
        &self,
        request_id: &str,
        decision: BrokerDecision,
        decided_by: &str,
    ) -> Result<(), AgentError>;

    /// Ask the agent to stop. Escalation to a kill is the supervisor's job.
    fn interrupt(&self, run_id: &str) -> Result<(), AgentError>;

    fn status(&self, run_id: &str) -> Result<AgentStatus, AgentError>;
}

/// The shared services every agent writes through: the process supervisor, the
/// activity record, the run store, and the permission broker. Grouped because an
/// agent is never usefully built with only some of them.
#[derive(Clone)]
pub struct AgentRuntime {
    pub supervisor: ProcessSupervisor,
    pub activity: ActivityRecord,
    pub runs: RunStore,
    pub broker: PermissionBroker,
}

/// An [`Agent`] backed by an installed CLI.
pub struct CliAgent {
    id: String,
    adapter: Arc<dyn AgentAdapter>,
    executable: PathBuf,
    capabilities: AgentCapabilities,
    supervisor: ProcessSupervisor,
    activity: ActivityRecord,
    runs: RunStore,
    broker: PermissionBroker,
    active: Arc<Mutex<Vec<ActiveRun>>>,
}

#[derive(Clone)]
struct ActiveRun {
    run_id: String,
    session_id: String,
}

impl CliAgent {
    #[must_use]
    pub fn new(
        adapter: Arc<dyn AgentAdapter>,
        executable: impl Into<PathBuf>,
        role: AgentRole,
        supports_resume: bool,
        runtime: AgentRuntime,
    ) -> Self {
        let kind = adapter.kind();
        Self {
            id: format!("{}:{}", adapter_name(kind), role.label()),
            adapter,
            executable: executable.into(),
            capabilities: AgentCapabilities::new(kind, role, supports_resume),
            supervisor: runtime.supervisor,
            activity: runtime.activity,
            runs: runtime.runs,
            broker: runtime.broker,
            active: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn active(&self) -> Result<MutexGuard<'_, Vec<ActiveRun>>, AgentError> {
        self.active.lock().map_err(|_| AgentError::StatePoisoned)
    }

    fn session_for(&self, run_id: &str) -> Result<String, AgentError> {
        self.active()?
            .iter()
            .find(|run| run.run_id == run_id)
            .map(|run| run.session_id.clone())
            .ok_or_else(|| AgentError::RunNotActive(run_id.to_owned()))
    }
}

impl Agent for CliAgent {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> AdapterKind {
        self.adapter.kind()
    }

    fn capabilities(&self) -> AgentCapabilities {
        self.capabilities
    }

    fn start(&self, task: &TaskSpec, context: &AgentContext) -> Result<AgentHandle, AgentError> {
        // The ceiling reaches the command line, not only the broker: a reviewer
        // is launched without write tools in the first place.
        let launch = AgentLaunchRequest {
            executable: self.executable.clone(),
            cwd: task.worktree_path.clone(),
            prompt: context.prompt.clone(),
            resume_session_id: context
                .resume_session_id
                .clone()
                .filter(|_| self.capabilities.supports_resume),
            allowed_tools: context.allowed_tools.clone(),
            allow_write: self.capabilities.may_write(),
        };
        let command = self.adapter.command(&launch)?;

        let run = self.runs.start(NewRun {
            task_id: task.task_id.clone(),
            agent_id: self.id.clone(),
            adapter_kind: adapter_name(self.adapter.kind()).to_owned(),
            role: self.capabilities.role.as_run_role(),
            capabilities: serde_json::to_value(self.capabilities).unwrap_or(Value::Null),
            context_pack_digest: context.pack_digest.clone(),
        })?;

        self.activity.append(
            ActivityEntry::new(
                &task.task_id,
                ActivityKind::RunStarted,
                Actor::System,
                "mars",
                format!(
                    "{} started as {} in {}",
                    self.id,
                    self.capabilities.role.label(),
                    task.worktree_path.display()
                ),
            )
            .in_run(&run.id)
            .with_payload(json!({
                "agentId": self.id,
                "role": self.capabilities.role,
                "capabilities": self.capabilities,
                "contextPackDigest": context.pack_digest,
                "expectedScope": task.expected_scope,
                "worktreePath": task.worktree_path,
            }))
            // The prompt is the whole of what the agent was told, so it is kept
            // verbatim rather than summarised.
            .with_attachment(Attachment::text("text/plain", context.prompt.clone())),
        )?;

        let mut spec = ProcessSpec::new(
            adapter_name(self.adapter.kind()),
            &command.program,
            command.arguments.clone(),
            &command.cwd,
        );
        spec.task_id = Some(task.task_id.clone());

        let launched = self.supervisor.launch(spec)?;
        self.runs.attach_session(&run.id, &launched.session_id)?;
        self.runs.set_status(&run.id, RunStatus::Running)?;
        self.active()?.push(ActiveRun {
            run_id: run.id.clone(),
            session_id: launched.session_id.clone(),
        });

        let (events_tx, events_rx) = mpsc::channel(256);
        tokio::spawn(pump_events(
            PumpContext {
                adapter: self.adapter.clone(),
                activity: self.activity.clone(),
                runs: self.runs.clone(),
                broker: self.broker.clone(),
                supervisor: self.supervisor.clone(),
                task_id: task.task_id.clone(),
                run_id: run.id.clone(),
                session_id: launched.session_id.clone(),
                agent_label: self.id.clone(),
                ceiling: self.capabilities.ceiling,
                worktree_path: task.worktree_path.clone(),
                expected_scope: task.expected_scope.clone(),
            },
            launched.events,
            events_tx,
        ));

        Ok(AgentHandle {
            run_id: run.id,
            session_id: launched.session_id,
            events: events_rx,
        })
    }

    fn respond_to_permission(
        &self,
        request_id: &str,
        decision: BrokerDecision,
        decided_by: &str,
    ) -> Result<(), AgentError> {
        self.broker.decide(request_id, decision, decided_by)?;
        Ok(())
    }

    fn interrupt(&self, run_id: &str) -> Result<(), AgentError> {
        let session_id = self.session_for(run_id)?;
        self.runs.set_status(run_id, RunStatus::Interrupting)?;
        self.supervisor.cancel(&session_id)?;
        Ok(())
    }

    fn status(&self, run_id: &str) -> Result<AgentStatus, AgentError> {
        let run = self.runs.get(run_id)?;
        let blocked_on_request = if run.status == RunStatus::Blocked {
            self.broker
                .pending(&run.task_id)?
                .into_iter()
                .find(|pending| pending.run_id.as_deref() == Some(run_id))
                .map(|pending| pending.id)
        } else {
            None
        };
        Ok(AgentStatus {
            run_id: run.id,
            session_id: run.session_id,
            state: run.status,
            last_sequence: run.last_sequence,
            blocked_on_request,
        })
    }
}

struct PumpContext {
    adapter: Arc<dyn AgentAdapter>,
    activity: ActivityRecord,
    runs: RunStore,
    broker: PermissionBroker,
    supervisor: ProcessSupervisor,
    task_id: String,
    run_id: String,
    session_id: String,
    agent_label: String,
    ceiling: CapabilityCeiling,
    worktree_path: PathBuf,
    expected_scope: Vec<String>,
}

/// Decode the process stream into normalized events, record each one, and stop
/// the whole pump on a permission request until a person answers it.
async fn pump_events(
    context: PumpContext,
    mut process_events: mpsc::Receiver<crate::process::ProcessEvent>,
    events: mpsc::Sender<AgentEvent>,
) {
    let mut decoder = JsonLineDecoder::new();
    let mut ended = false;

    while let Some(process_event) = process_events.recv().await {
        let decoded = match &process_event.kind {
            ProcessEventKind::Stdout { text } => {
                decoder.push(text.as_bytes(), context.adapter.as_ref())
            }
            ProcessEventKind::Stderr { text } => vec![NormalizedAgentEvent::Unknown {
                provider_type: "stderr".into(),
                raw: Value::String(text.clone()),
            }],
            ProcessEventKind::Exit { code, success } => {
                ended = true;
                let mut tail = decoder.finish(context.adapter.as_ref());
                tail.push(NormalizedAgentEvent::End {
                    success: *success,
                    message: (!*success).then(|| format!("process exited with {code:?}")),
                });
                tail
            }
            ProcessEventKind::CancelRequested => vec![],
            ProcessEventKind::KillEscalated => vec![NormalizedAgentEvent::End {
                success: false,
                message: Some("the process was killed after it did not stop".into()),
            }],
            ProcessEventKind::InterruptedOnRestart => vec![NormalizedAgentEvent::End {
                success: false,
                message: Some("Mars restarted while this process was running".into()),
            }],
            ProcessEventKind::DeliveryLagged => vec![NormalizedAgentEvent::Unknown {
                provider_type: "delivery_lagged".into(),
                raw: Value::Bool(true),
            }],
        };

        for normalized in decoded {
            let _ = context
                .supervisor
                .record_normalized(&context.session_id, &normalized);
            let event = to_agent_event(normalized);
            match handle_event(&context, event, &events).await {
                Ok(Flow::Continue) => {}
                Ok(Flow::Stop) => {
                    // A refused effect ends the run. The process cannot be told
                    // "no" through its own stream, so it is stopped instead of
                    // being left to carry on as though it had been allowed.
                    // The run is settled before the ending is announced, so a
                    // consumer reacting to `Done` never reads a stale status.
                    let _ = context.supervisor.cancel(&context.session_id);
                    let _ = context
                        .runs
                        .set_status(&context.run_id, RunStatus::Cancelled);
                    let ending = AgentEvent::Done {
                        success: false,
                        message: Some("stopped after a refused request".into()),
                    };
                    record(&context, &ending);
                    let _ = events.send(ending).await;
                    return;
                }
                Err(()) => return,
            }
        }
    }

    if !ended {
        let _ = context.runs.set_status(&context.run_id, RunStatus::Failed);
    }
    drop(events);
}

/// Whether the pump should keep reading the agent's stream.
enum Flow {
    Continue,
    Stop,
}

/// Record one event, then act on it. Returns `Err` when the consumer is gone.
async fn handle_event(
    context: &PumpContext,
    event: AgentEvent,
    events: &mpsc::Sender<AgentEvent>,
) -> Result<Flow, ()> {
    // A command, or a write outside the declared scope, is a side effect: it
    // goes through the broker before it is reported as having happened.
    let effect = match &event {
        AgentEvent::Command { command, .. } => Some(SideEffect::Shell {
            command: command.clone(),
            cwd: context.worktree_path.clone(),
        }),
        AgentEvent::FileEdit { path, .. } if !within_scope(path, &context.expected_scope) => {
            Some(SideEffect::FileWrite {
                path: context.worktree_path.join(path),
                declared_scope: context.expected_scope.clone(),
            })
        }
        _ => None,
    };
    if let Some(effect) = effect
        && !ask_permission(context, effect, events).await
    {
        return Ok(Flow::Stop);
    }

    record(context, &event);

    if let AgentEvent::Done { success, .. } = &event {
        let _ = context.runs.set_status(
            &context.run_id,
            if *success {
                RunStatus::Completed
            } else {
                RunStatus::Failed
            },
        );
    }

    events.send(event).await.map_err(|_| ())?;
    Ok(Flow::Continue)
}

/// Put the effect to the broker. Returns whether the agent may proceed; a
/// blocked agent waits here rather than continuing to read its stream.
async fn ask_permission(
    context: &PumpContext,
    effect: SideEffect,
    events: &mpsc::Sender<AgentEvent>,
) -> bool {
    let request = BrokerRequest {
        task_id: context.task_id.clone(),
        run_id: Some(context.run_id.clone()),
        agent_label: context.agent_label.clone(),
        ceiling: context.ceiling,
        effect,
    };
    let Ok((outcome, ticket)) = context.broker.request(&request) else {
        return false;
    };

    match outcome {
        BrokerOutcome::Allowed { .. } => true,
        BrokerOutcome::Denied { reason, .. } => {
            refuse(context, &reason, events).await;
            false
        }
        BrokerOutcome::Pending { request_id } => {
            let _ = context.runs.set_status(&context.run_id, RunStatus::Blocked);
            let asked = AgentEvent::PermissionRequest {
                request_id: request_id.clone(),
                summary: request.effect.summary(),
                classification: request.effect.classification().as_str().to_owned(),
            };
            let _ = events.send(asked).await;

            let Some(ticket) = ticket else {
                refuse(context, "the request was abandoned", events).await;
                return false;
            };
            match ticket.resolve().await {
                Some(BrokerOutcome::Allowed { .. }) => {
                    let _ = context.runs.set_status(&context.run_id, RunStatus::Running);
                    true
                }
                Some(BrokerOutcome::Denied { reason, .. }) => {
                    refuse(context, &reason, events).await;
                    false
                }
                _ => {
                    refuse(context, "no decision was recorded", events).await;
                    false
                }
            }
        }
    }
}

/// Tell the stream, and the log, why the agent is not allowed to proceed.
async fn refuse(context: &PumpContext, reason: &str, events: &mpsc::Sender<AgentEvent>) {
    let denial = AgentEvent::Error {
        message: format!("refused: {reason}"),
    };
    record(context, &denial);
    let _ = events.send(denial).await;
}

fn record(context: &PumpContext, event: &AgentEvent) {
    let Some(kind) = event.activity_kind() else {
        return;
    };
    // The broker writes its own permission events; recording them again here
    // would double every request in the transcript.
    if kind == ActivityKind::PermissionRequest {
        return;
    }
    let payload = serde_json::to_value(event).unwrap_or(Value::Null);
    let entry = ActivityEntry::new(
        &context.task_id,
        kind,
        Actor::Agent,
        &context.agent_label,
        event.summary(),
    )
    .in_run(&context.run_id)
    .with_payload(payload);
    let _ = context.activity.append(entry);
}

/// Whether a path is inside the file scope the approved plan declared. An empty
/// scope declares nothing, so nothing is inside it.
fn within_scope(path: &str, scope: &[String]) -> bool {
    let normalized = path.replace('\\', "/");
    scope
        .iter()
        .any(|pattern| mars_security::glob_match(pattern, &normalized))
}

fn to_agent_event(event: NormalizedAgentEvent) -> AgentEvent {
    match event {
        NormalizedAgentEvent::Text { text } => AgentEvent::Message { text },
        NormalizedAgentEvent::Tool { name, input } => AgentEvent::ToolCall { name, input },
        NormalizedAgentEvent::Command { command, status } => {
            AgentEvent::Command { command, status }
        }
        NormalizedAgentEvent::FileChange { path, change } => AgentEvent::FileEdit { path, change },
        NormalizedAgentEvent::Test { command, status } => AgentEvent::TestRun { command, status },
        NormalizedAgentEvent::Permission { tool, description } => AgentEvent::PermissionRequest {
            request_id: String::new(),
            summary: format!("{tool}: {description}"),
            classification: "mutating".into(),
        },
        NormalizedAgentEvent::Checkpoint { session_id } => AgentEvent::Checkpoint { session_id },
        NormalizedAgentEvent::End { success, message } => AgentEvent::Done { success, message },
        NormalizedAgentEvent::Unknown { provider_type, raw } => {
            AgentEvent::Unknown { provider_type, raw }
        }
    }
}

const fn adapter_name(kind: AdapterKind) -> &'static str {
    match kind {
        AdapterKind::Claude => "claude",
        AdapterKind::Codex => "codex",
    }
}

fn truncate(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_owned();
    }
    let mut shortened: String = trimmed.chars().take(limit).collect();
    shortened.push('…');
    shortened
}
