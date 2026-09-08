//! The `Agent` interface: roles, capabilities, streaming, blocking, and status.
//!
//! These drive a real `CliAgent` over a scripted process rather than a mock
//! agent, so the adapter, the supervisor, the broker, and the activity record
//! are all exercised together. The script is a small program that writes adapter
//! JSONL to stdout, which is exactly what a real CLI does.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mars_activity::{ActivityKind, ActivityRecord, RunStatus, RunStore};
use mars_agents::adapter::{
    AdapterCommand, AdapterError, AdapterKind, AgentAdapter, AgentLaunchRequest,
    NormalizedAgentEvent,
};
use mars_agents::agent::{
    Agent, AgentContext, AgentEvent, AgentRole, AgentRuntime, CliAgent, TaskSpec,
};
use mars_agents::process::ProcessSupervisor;
use mars_security::{BrokerDecision, PermissionBroker};
use mars_storage::Storage;
use serde_json::{Value, json};

/// An adapter that replays a scripted JSONL transcript instead of driving a
/// real CLI. The script lives in a file so no shell quoting sits between the
/// test and the bytes the decoder sees.
struct ScriptedAdapter {
    script_path: PathBuf,
    /// Every `allow_write` the adapter was asked to build a command for, so a
    /// test can assert that the ceiling reached the command line.
    write_requests: Arc<Mutex<Vec<bool>>>,
}

impl ScriptedAdapter {
    fn new(script_path: PathBuf, write_requests: Arc<Mutex<Vec<bool>>>) -> Self {
        Self {
            script_path,
            write_requests,
        }
    }
}

impl AgentAdapter for ScriptedAdapter {
    fn kind(&self) -> AdapterKind {
        AdapterKind::Claude
    }

    fn command(&self, request: &AgentLaunchRequest) -> Result<AdapterCommand, AdapterError> {
        if request.prompt.trim().is_empty() {
            return Err(AdapterError::EmptyPrompt);
        }
        self.write_requests
            .lock()
            .expect("write requests should lock")
            .push(request.allow_write);
        let (program, arguments) = replay_command(&self.script_path);
        Ok(AdapterCommand {
            program,
            arguments,
            cwd: request.cwd.clone(),
        })
    }

    fn parse_value(&self, value: Value) -> Vec<NormalizedAgentEvent> {
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("missing");
        let text = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        match event_type {
            "text" => vec![NormalizedAgentEvent::Text { text: text("text") }],
            "command" => vec![NormalizedAgentEvent::Command {
                command: text("command"),
                status: text("status"),
            }],
            "file" => vec![NormalizedAgentEvent::FileChange {
                path: text("path"),
                change: text("change"),
            }],
            "test" => vec![NormalizedAgentEvent::Test {
                command: text("command"),
                status: text("status"),
            }],
            "end" => vec![NormalizedAgentEvent::End {
                success: value
                    .get("success")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                message: None,
            }],
            other => vec![NormalizedAgentEvent::Unknown {
                provider_type: other.to_owned(),
                raw: value,
            }],
        }
    }
}

/// A process that writes the script to stdout and exits, which is the shape of
/// every CLI adapter Mars supports.
fn replay_command(script: &Path) -> (PathBuf, Vec<std::ffi::OsString>) {
    if cfg!(windows) {
        (
            PathBuf::from("cmd"),
            vec!["/C".into(), "type".into(), script.as_os_str().to_owned()],
        )
    } else {
        (PathBuf::from("cat"), vec![script.as_os_str().to_owned()])
    }
}

struct AgentFixture {
    storage: Arc<Storage>,
    task_id: String,
    worktree: PathBuf,
    payload_root: PathBuf,
    _directory: TestDirectory,
}

impl AgentFixture {
    fn new() -> Self {
        let directory = TestDirectory::new();
        let worktree = directory.path().join("worktree");
        fs::create_dir_all(&worktree).expect("worktree should exist");
        let storage = Arc::new(
            Storage::open(directory.path().join("mars-test.sqlite3")).expect("storage should open"),
        );
        storage
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO projects (id, name, created_at, updated_at)
                     VALUES ('project-1', 'fixture', '2026-09-08T00:00:00Z',
                             '2026-09-08T00:00:00Z')",
                    [],
                )?;
                connection.execute(
                    "INSERT INTO repositories
                         (id, project_id, root_path, git_common_dir, state, created_at, updated_at)
                     VALUES ('repository-1', 'project-1', ?1, ?2, 'active',
                             '2026-09-08T00:00:00Z', '2026-09-08T00:00:00Z')",
                    rusqlite::params![
                        worktree.to_string_lossy(),
                        worktree.join(".git").to_string_lossy()
                    ],
                )?;
                connection.execute(
                    "INSERT INTO tasks
                         (id, project_id, repository_id, title, intent, status, created_at,
                          updated_at)
                     VALUES ('task-1', 'project-1', 'repository-1', 'Fixture', 'do it', 'ready',
                             '2026-09-08T00:00:00Z', '2026-09-08T00:00:00Z')",
                    [],
                )?;
                Ok(())
            })
            .expect("fixture rows should insert");

        Self {
            storage,
            task_id: "task-1".into(),
            worktree,
            payload_root: directory.path().join("payloads"),
            _directory: directory,
        }
    }

    fn activity(&self) -> ActivityRecord {
        ActivityRecord::new(self.storage.clone(), &self.payload_root)
    }

    fn broker(&self) -> PermissionBroker {
        PermissionBroker::new(self.storage.clone()).recording_to(self.activity())
    }

    fn runtime(&self) -> AgentRuntime {
        AgentRuntime {
            supervisor: ProcessSupervisor::new(self.storage.clone()),
            activity: self.activity(),
            runs: RunStore::new(self.storage.clone()),
            broker: self.broker(),
        }
    }

    fn agent(&self, role: AgentRole, script: Vec<Value>) -> CliAgent {
        self.agent_watching_writes(role, script, Arc::new(Mutex::new(Vec::new())))
    }

    fn agent_watching_writes(
        &self,
        role: AgentRole,
        script: Vec<Value>,
        write_requests: Arc<Mutex<Vec<bool>>>,
    ) -> CliAgent {
        let script_path = self.worktree.join(format!("script-{}.jsonl", role.label()));
        let body = script
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(
                "
",
            );
        fs::write(
            &script_path,
            format!(
                "{body}
"
            ),
        )
        .expect("script should write");
        CliAgent::new(
            Arc::new(ScriptedAdapter::new(script_path, write_requests)),
            "replayed-by-the-adapter",
            role,
            true,
            self.runtime(),
        )
    }

    fn task(&self, scope: Vec<String>) -> TaskSpec {
        TaskSpec {
            task_id: self.task_id.clone(),
            title: "Fixture".into(),
            worktree_path: self.worktree.clone(),
            expected_scope: scope,
        }
    }
}

fn context() -> AgentContext {
    AgentContext {
        prompt: "Implement the guard.".into(),
        pack_digest: Some("pack-digest".into()),
        resume_session_id: None,
        allowed_tools: vec!["read".into()],
    }
}

fn runtime_for_test() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("test runtime should build")
}

async fn drain(handle: &mut mars_agents::agent::AgentHandle) -> Vec<AgentEvent> {
    let mut collected = Vec::new();
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_secs(10), handle.events.recv()).await
    {
        let done = matches!(event, AgentEvent::Done { .. });
        collected.push(event);
        if done {
            break;
        }
    }
    collected
}

#[test]
fn a_reviewer_is_read_only_by_construction_not_by_instruction() {
    let fixture = AgentFixture::new();
    let reviewer = fixture.agent(
        AgentRole::Reviewer,
        vec![json!({"type": "end", "success": true})],
    );
    let implementer = fixture.agent(
        AgentRole::Implementer,
        vec![json!({"type": "end", "success": true})],
    );

    let reviewer_capabilities = reviewer.capabilities();
    let implementer_capabilities = implementer.capabilities();

    assert_eq!(reviewer_capabilities.role, AgentRole::Reviewer);
    assert!(!reviewer_capabilities.may_write());
    assert!(!reviewer_capabilities.ceiling.write_files);
    assert!(!reviewer_capabilities.ceiling.network);
    assert!(!reviewer_capabilities.ceiling.git_push);
    assert!(reviewer_capabilities.ceiling.read_files);
    assert!(
        !reviewer_capabilities.supports_resume,
        "a reviewer has no session to resume into"
    );

    assert!(implementer_capabilities.may_write());
    assert!(implementer_capabilities.ceiling.git_push);
    assert_eq!(reviewer.id(), "claude:reviewer");
    assert_eq!(implementer.id(), "claude:implementer");
    assert_eq!(reviewer.kind(), AdapterKind::Claude);
}

#[test]
fn a_run_streams_the_normalized_vocabulary_and_records_every_event() {
    let fixture = AgentFixture::new();
    let agent = fixture.agent(
        AgentRole::Implementer,
        vec![
            json!({"type": "text", "text": "I will add the guard"}),
            json!({"type": "file", "path": "src/math.ts", "change": "modified"}),
            json!({"type": "test", "command": "npm test", "status": "passed"}),
            json!({"type": "end", "success": true}),
        ],
    );

    runtime_for_test().block_on(async {
        let mut handle = agent
            .start(&fixture.task(vec!["src/**".into()]), &context())
            .expect("agent should start");
        let events = drain(&mut handle).await;

        assert!(matches!(events[0], AgentEvent::Message { .. }));
        assert!(matches!(events[1], AgentEvent::FileEdit { .. }));
        assert!(matches!(events[2], AgentEvent::TestRun { .. }));
        assert!(matches!(events[3], AgentEvent::Done { success: true, .. }));

        let logged = fixture
            .activity()
            .read(&fixture.task_id, 0, 50)
            .expect("log should read");
        let kinds = logged.iter().map(|event| event.kind).collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                ActivityKind::RunStarted,
                ActivityKind::Message,
                ActivityKind::FileEdit,
                ActivityKind::TestRun,
                ActivityKind::Done,
            ]
        );
        assert_eq!(logged[0].payload["contextPackDigest"], "pack-digest");

        let status = agent.status(&handle.run_id).expect("status should read");
        assert_eq!(status.state, RunStatus::Completed);
        assert!(status.blocked_on_request.is_none());
        assert!(status.last_sequence >= 5);
    });
}

#[test]
fn the_prompt_the_agent_was_given_is_kept_verbatim_in_the_log() {
    let fixture = AgentFixture::new();
    let agent = fixture.agent(
        AgentRole::Implementer,
        vec![json!({"type": "end", "success": true})],
    );

    runtime_for_test().block_on(async {
        let mut handle = agent
            .start(&fixture.task(vec!["src/**".into()]), &context())
            .expect("agent should start");
        drain(&mut handle).await;

        let record = fixture.activity();
        let started = record
            .read(&fixture.task_id, 0, 5)
            .expect("log should read")
            .into_iter()
            .next()
            .expect("a run should be recorded");
        let digest = started
            .payload_digest
            .expect("the prompt should be stored by digest");
        let bytes = record
            .payload_bytes(&digest)
            .expect("prompt should resolve");
        assert_eq!(String::from_utf8_lossy(&bytes), "Implement the guard.");
    });
}

#[test]
fn a_command_blocks_the_agent_until_a_person_answers() {
    let fixture = AgentFixture::new();
    let agent = fixture.agent(
        AgentRole::Implementer,
        vec![
            json!({"type": "command", "command": "npm test", "status": "started"}),
            json!({"type": "text", "text": "tests are green"}),
            json!({"type": "end", "success": true}),
        ],
    );
    let broker = fixture.broker();

    runtime_for_test().block_on(async {
        let mut handle = agent
            .start(&fixture.task(vec!["src/**".into()]), &context())
            .expect("agent should start");

        let asked = tokio::time::timeout(Duration::from_secs(10), handle.events.recv())
            .await
            .expect("a permission request should arrive")
            .expect("the stream should stay open");
        let AgentEvent::PermissionRequest {
            request_id,
            classification,
            ..
        } = asked
        else {
            panic!("the first event should be the permission request, got {asked:?}")
        };
        assert_eq!(classification, "mutating");

        // Nothing further arrives while the question is unanswered.
        assert!(
            tokio::time::timeout(Duration::from_millis(300), handle.events.recv())
                .await
                .is_err(),
            "a blocked agent must not keep streaming"
        );
        let status = agent.status(&handle.run_id).expect("status should read");
        assert_eq!(status.state, RunStatus::Blocked);
        assert_eq!(
            status.blocked_on_request.as_deref(),
            Some(request_id.as_str())
        );

        agent
            .respond_to_permission(&request_id, BrokerDecision::AllowTask, "fayeed")
            .expect("the decision should reach the agent");

        let rest = drain(&mut handle).await;
        assert!(
            rest.iter()
                .any(|event| matches!(event, AgentEvent::Command { .. })),
            "the command should be reported once it was allowed"
        );
        assert!(
            rest.iter()
                .any(|event| matches!(event, AgentEvent::Done { success: true, .. }))
        );

        let decisions = broker
            .grants(Some(&fixture.task_id))
            .expect("grants should read");
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].granted_by, "fayeed");
        assert!(decisions[0].active);
    });
}

#[test]
fn a_denied_command_stops_the_agent_and_says_so_in_the_stream() {
    let fixture = AgentFixture::new();
    let agent = fixture.agent(
        AgentRole::Implementer,
        vec![
            json!({"type": "command", "command": "rm -rf build", "status": "started"}),
            json!({"type": "end", "success": true}),
        ],
    );

    runtime_for_test().block_on(async {
        let mut handle = agent
            .start(&fixture.task(vec!["src/**".into()]), &context())
            .expect("agent should start");

        let asked = tokio::time::timeout(Duration::from_secs(10), handle.events.recv())
            .await
            .expect("a permission request should arrive")
            .expect("the stream should stay open");
        let AgentEvent::PermissionRequest {
            request_id,
            classification,
            ..
        } = asked
        else {
            panic!("expected a permission request, got {asked:?}")
        };
        assert_eq!(classification, "destructive");

        agent
            .respond_to_permission(&request_id, BrokerDecision::Deny, "fayeed")
            .expect("the denial should reach the agent");

        let rest = drain(&mut handle).await;
        assert!(
            rest.iter().any(|event| matches!(
                event,
                AgentEvent::Error { message } if message.contains("refused")
            )),
            "a denial should be visible in the stream, got {rest:?}"
        );
        assert!(
            !rest
                .iter()
                .any(|event| matches!(event, AgentEvent::Command { .. })),
            "a denied command must never be reported as having run"
        );
        assert!(
            rest.iter()
                .any(|event| matches!(event, AgentEvent::Done { success: false, .. })),
            "a refusal should end the run rather than let it continue"
        );
        assert_eq!(
            agent
                .status(&handle.run_id)
                .expect("status should read")
                .state,
            RunStatus::Cancelled
        );
    });
}

#[test]
fn a_write_inside_the_declared_scope_needs_no_permission() {
    let fixture = AgentFixture::new();
    let agent = fixture.agent(
        AgentRole::Implementer,
        vec![
            json!({"type": "file", "path": "src/math.ts", "change": "modified"}),
            json!({"type": "end", "success": true}),
        ],
    );

    runtime_for_test().block_on(async {
        let mut handle = agent
            .start(&fixture.task(vec!["src/**".into()]), &context())
            .expect("agent should start");
        let events = drain(&mut handle).await;

        assert!(
            !events
                .iter()
                .any(|event| matches!(event, AgentEvent::PermissionRequest { .. })),
            "an in-scope write is the work, not a question"
        );
        assert!(matches!(events[0], AgentEvent::FileEdit { .. }));
    });
}

#[test]
fn a_write_outside_the_declared_scope_becomes_a_question() {
    let fixture = AgentFixture::new();
    let agent = fixture.agent(
        AgentRole::Implementer,
        vec![
            json!({"type": "file", "path": "config/secrets.yaml", "change": "modified"}),
            json!({"type": "end", "success": true}),
        ],
    );

    runtime_for_test().block_on(async {
        let mut handle = agent
            .start(&fixture.task(vec!["src/**".into()]), &context())
            .expect("agent should start");

        let asked = tokio::time::timeout(Duration::from_secs(10), handle.events.recv())
            .await
            .expect("a permission request should arrive")
            .expect("the stream should stay open");
        let AgentEvent::PermissionRequest { summary, .. } = asked else {
            panic!("expected a permission request, got {asked:?}")
        };
        assert!(summary.contains("outside the declared scope"));
    });
}

#[test]
fn a_reviewers_write_is_refused_without_ever_asking_a_person() {
    let fixture = AgentFixture::new();
    let agent = fixture.agent(
        AgentRole::Reviewer,
        vec![
            json!({"type": "file", "path": "config/secrets.yaml", "change": "modified"}),
            json!({"type": "end", "success": true}),
        ],
    );
    let broker = fixture.broker();

    runtime_for_test().block_on(async {
        let mut handle = agent
            .start(&fixture.task(vec!["src/**".into()]), &context())
            .expect("agent should start");
        let events = drain(&mut handle).await;

        assert!(
            events.iter().any(|event| matches!(
                event,
                AgentEvent::Error { message } if message.contains("read-only")
            )),
            "the reviewer should be told why, got {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, AgentEvent::PermissionRequest { .. })),
            "no human should ever be asked to let a reviewer write"
        );
        assert!(
            broker
                .pending(&fixture.task_id)
                .expect("pending should read")
                .is_empty()
        );
    });
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("mars-agent-fixture-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).expect("test directory should exist");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.0.exists() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
