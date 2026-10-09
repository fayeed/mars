//! The MCP server over a real indexed repository: the protocol, the graph
//! tools, the writes an agent may make, and the permission prompt.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mars_activity::{ActivityEntry, ActivityKind, ActivityRecord, Actor};
use mars_core::projects::{OpenRepositoryOutcome, ProjectService};
use mars_git::TaskWorktreeService;
use mars_indexer::Indexer;
use mars_mcp::{ServerConfig, ToolContext, serve};
use mars_security::{BrokerDecision, PermissionBroker, TrustDecision};
use mars_storage::Storage;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

struct Fixture {
    // Drop order: everything holding the database closes before the
    // directory is removed.
    storage: Arc<Storage>,
    database: PathBuf,
    repository_id: String,
    task_id: String,
    worktree: PathBuf,
    _directory: TestDirectory,
}

impl Fixture {
    fn new(indexed: bool) -> Self {
        let directory = TestDirectory::new();
        let root = directory.path().join("repo");
        copy_directory(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/repositories/typescript-graph"),
            &root,
        );
        git(&root, &["init", "--quiet", "--initial-branch=main"]);
        git(&root, &["add", "-A"]);
        git(
            &root,
            &[
                "-c",
                "user.name=Mars Test",
                "-c",
                "user.email=mars@example.test",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            ],
        );
        let database = directory.path().join("mars.sqlite3");
        let storage = Arc::new(Storage::open(&database).expect("storage"));
        let OpenRepositoryOutcome::Opened(project) = ProjectService::new(storage.clone())
            .open_repository(&root, TrustDecision::Trust)
            .expect("repository opens")
        else {
            panic!("trusted repository opens");
        };
        if indexed {
            Indexer::new(storage.clone())
                .index_repository(&project.repository_id, &CancellationToken::new())
                .expect("fixture indexes");
        }
        let task = TaskWorktreeService::new(storage.clone(), directory.path().join("worktrees"))
            .create_task(
                &project.repository_id,
                "Tune the calculator",
                "make add faster",
                "HEAD",
            )
            .expect("task plans");
        Self {
            storage,
            database,
            repository_id: project.repository_id,
            task_id: task.task_id,
            worktree: task.worktree_path,
            _directory: directory,
        }
    }

    fn config(&self, role: &str) -> ServerConfig {
        ServerConfig {
            database: self.database.clone(),
            repository_id: self.repository_id.clone(),
            task_id: Some(self.task_id.clone()),
            run_id: None,
            worktree: Some(self.worktree.clone()),
            role: role.into(),
            scope: vec!["src/**".into()],
            agent: "claude (implementer)".into(),
            readable: Vec::new(),
            permission_tool: true,
        }
    }

    fn context(&self, role: &str) -> ToolContext {
        ToolContext::open(self.config(role)).expect("context opens")
    }
}

fn call(context: &ToolContext, name: &str, arguments: Value) -> (String, bool) {
    let outcome = context.call(name, &arguments, &AtomicBool::new(false));
    (outcome.text, outcome.is_error)
}

#[test]
fn the_protocol_initialises_lists_tools_and_answers_over_stdio() {
    let fixture = Fixture::new(true);
    let context = Arc::new(fixture.context("implementer"));
    let input = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"find_symbol","arguments":{"name":"add"}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"ping"}),
        json!({"jsonrpc":"2.0","id":5,"method":"resources/list"}),
    ]
    .iter()
    .map(Value::to_string)
    .collect::<Vec<_>>()
    .join("\n");
    let output = SharedBuffer::default();
    serve(context, Cursor::new(input), output.clone());

    let responses: Vec<Value> = output
        .text()
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is one JSON message"))
        .collect();
    let by_id = |id: i64| {
        responses
            .iter()
            .find(|response| response["id"] == json!(id))
            .unwrap_or_else(|| panic!("response {id} missing: {responses:?}"))
    };
    assert_eq!(by_id(1)["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(by_id(1)["result"]["serverInfo"]["name"], "mars");
    let tools: Vec<&str> = by_id(2)["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    for expected in [
        "search",
        "find_symbol",
        "references",
        "callers",
        "impact",
        "ask",
        "memory",
        "record_decision",
        "remember",
        "permission_prompt",
    ] {
        assert!(
            tools.contains(&expected),
            "{expected} is offered: {tools:?}"
        );
    }
    let found = by_id(3)["result"]["content"][0]["text"]
        .as_str()
        .expect("text");
    assert!(found.contains("src/math.ts"), "{found}");
    assert_eq!(by_id(4)["result"], json!({}));
    assert_eq!(by_id(5)["error"]["code"], -32601);
    // Nothing answers a notification.
    assert_eq!(responses.len(), 5);
}

#[test]
fn a_reviewer_is_offered_no_tool_that_writes() {
    let fixture = Fixture::new(true);
    let context = fixture.context("reviewer");
    let names: Vec<String> = context
        .definitions()
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
        .collect();
    assert!(!names.contains(&"record_decision".to_owned()));
    assert!(!names.contains(&"remember".to_owned()));
    let (text, error) = call(
        &context,
        "record_decision",
        json!({"title":"x","decision":"y","rationale":"z"}),
    );
    assert!(error, "{text}");
}

#[test]
fn the_graph_tools_answer_from_the_index() {
    let fixture = Fixture::new(true);
    let context = fixture.context("implementer");

    let (text, error) = call(
        &context,
        "search",
        json!({"query": "calculate numeric value"}),
    );
    assert!(!error, "{text}");
    assert!(text.contains("src/"), "{text}");

    let (text, error) = call(&context, "callers", json!({"symbol": "add"}));
    assert!(!error, "{text}");
    assert!(text.contains("calculate"), "callers of add: {text}");

    let (text, error) = call(&context, "impact", json!({"symbol": "add", "depth": 2}));
    assert!(!error, "{text}");

    let (text, error) = call(&context, "file_deps", json!({"path": "src/main.ts"}));
    assert!(!error, "{text}");
    assert!(text.contains("imports"), "{text}");

    let (text, error) = call(
        &context,
        "symbols_at",
        json!({"path": "src/math.ts", "line": 13}),
    );
    assert!(!error, "{text}");
    assert!(text.contains("calculate"), "{text}");

    let (text, error) = call(&context, "ask", json!({"question": "What does run do?"}));
    assert!(!error, "{text}");
    assert!(text.contains("source(s)"), "{text}");

    let (text, error) = call(&context, "callers", json!({"symbol": "doesNotExist"}));
    assert!(
        error,
        "an unknown name is an error the agent can read: {text}"
    );

    let (text, error) = call(&context, "index_status", json!({}));
    assert!(!error, "{text}");
    assert!(text.contains("file(s)"), "{text}");
}

#[test]
fn an_unindexed_repository_says_so_instead_of_returning_nothing() {
    let fixture = Fixture::new(false);
    let context = fixture.context("implementer");
    let (text, error) = call(&context, "search", json!({"query": "anything"}));
    assert!(error);
    assert!(text.contains("not been indexed"), "{text}");
}

#[test]
fn a_recorded_decision_waits_for_a_person_and_belongs_to_the_task() {
    let fixture = Fixture::new(true);
    let context = fixture.context("implementer");
    let (text, error) = call(
        &context,
        "record_decision",
        json!({
            "title": "Keep add pure",
            "decision": "add stays a pure function",
            "rationale": "callers rely on it having no side effects",
            "alternatives": ["memoise it"],
            "files": ["src/math.ts", "does/not/exist.ts"]
        }),
    );
    assert!(!error, "{text}");
    assert!(text.contains("Linked to 1 of 2"), "{text}");
    let records = mars_brain::KnowledgeService::new(fixture.storage.clone())
        .list_for_task(&fixture.task_id)
        .expect("records list");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].kind, mars_brain::KnowledgeKind::Decision);
    assert_eq!(records[0].status, mars_brain::KnowledgeStatus::Candidate);
    assert!(records[0].body.contains("memoise it"));

    let (text, error) = call(&context, "decisions", json!({}));
    assert!(!error);
    assert!(
        text.contains("Keep add pure") && text.contains("awaiting approval"),
        "{text}"
    );
}

#[test]
fn a_memory_proposal_is_verified_against_the_conversation_it_quotes() {
    let fixture = Fixture::new(true);
    let payloads = fixture
        .database
        .parent()
        .expect("dir")
        .join("activity-payloads");
    ActivityRecord::new(fixture.storage.clone(), &payloads)
        .append(
            ActivityEntry::new(
                &fixture.task_id,
                ActivityKind::Message,
                Actor::Human,
                "fayeed",
                "Always use pnpm",
            )
            .with_payload(json!({ "text": "Please always use pnpm, never npm, in this repo." })),
        )
        .expect("message records");
    let context = fixture.context("implementer");

    let (text, error) = call(
        &context,
        "remember",
        json!({"text": "Use pnpm, not npm.", "kind": "convention", "quote": "always use pnpm, never npm"}),
    );
    assert!(!error, "{text}");
    assert!(!text.contains("unverified"), "{text}");

    let (text, error) = call(
        &context,
        "remember",
        json!({"text": "Prefer tabs.", "kind": "preference", "quote": "words nobody said"}),
    );
    assert!(!error, "{text}");
    assert!(text.contains("unverified"), "{text}");

    let pending: (i64, i64) = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT COUNT(*), SUM(origin_verified) FROM memory_proposals WHERE status = 'pending'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .expect("proposals count");
    assert_eq!(pending, (2, 1));
    // Proposals are never retrieved before approval.
    let (text, _) = call(&context, "memory", json!({"query": "pnpm"}));
    assert!(text.contains("No approved memory"), "{text}");
}

fn decision(text: &str) -> Value {
    serde_json::from_str(text).expect("the permission tool answers in JSON")
}

#[test]
fn the_permission_prompt_allows_what_was_granted_and_refuses_escapes() {
    let fixture = Fixture::new(true);
    let context = fixture.context("implementer");
    let worktree = fixture.worktree.to_string_lossy().into_owned();

    let (text, _) = call(
        &context,
        "permission_prompt",
        json!({"tool_name": "Bash", "input": {"command": "git status"}}),
    );
    assert_eq!(decision(&text)["behavior"], "allow");
    assert_eq!(decision(&text)["updatedInput"]["command"], "git status");

    let (text, _) = call(
        &context,
        "permission_prompt",
        json!({"tool_name": "Edit", "input": {"file_path": format!("{worktree}/src/math.ts")}}),
    );
    assert_eq!(
        decision(&text)["behavior"],
        "allow",
        "inside the declared scope"
    );

    let (text, _) = call(
        &context,
        "permission_prompt",
        json!({"tool_name": "Write", "input": {"file_path": "../../escape.txt"}}),
    );
    assert_eq!(decision(&text)["behavior"], "deny");
    assert!(
        decision(&text)["message"]
            .as_str()
            .unwrap()
            .contains("outside")
    );

    let (text, _) = call(
        &context,
        "permission_prompt",
        json!({"tool_name": "Write", "input": {"file_path": ".git/config"}}),
    );
    assert_eq!(decision(&text)["behavior"], "deny");

    // A reviewer cannot be granted a write, whoever answers.
    let reviewer = fixture.context("reviewer");
    let (text, _) = call(
        &reviewer,
        "permission_prompt",
        json!({"tool_name": "Write", "input": {"file_path": "README.md"}}),
    );
    assert_eq!(decision(&text)["behavior"], "deny");
}

#[test]
fn the_permission_prompt_waits_for_the_person_and_carries_their_answer() {
    let fixture = Fixture::new(true);
    let context = Arc::new(fixture.context("implementer"));
    let asking = {
        let context = context.clone();
        std::thread::spawn(move || {
            call(
                &context,
                "permission_prompt",
                json!({"tool_name": "Bash", "input": {"command": "pnpm install left-pad"}}),
            )
        })
    };

    // The person sees it as pending in their own process's broker.
    let broker = PermissionBroker::new(fixture.storage.clone());
    let deadline = SystemTime::now() + Duration::from_secs(10);
    let pending = loop {
        let pending = broker.pending(&fixture.task_id).expect("pending loads");
        if let Some(request) = pending.into_iter().next() {
            break request;
        }
        assert!(SystemTime::now() < deadline, "the request never appeared");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(pending.summary.contains("pnpm install left-pad"));
    broker
        .decide(&pending.id, BrokerDecision::AllowOnce, "fayeed")
        .expect("decision records");

    let (text, _) = asking.join().expect("the tool returns");
    assert_eq!(decision(&text)["behavior"], "allow");

    // A refusal is carried back as an answer the agent can act on.
    let refusing = {
        let context = context.clone();
        std::thread::spawn(move || {
            call(
                &context,
                "permission_prompt",
                json!({"tool_name": "WebFetch", "input": {"url": "https://example.com/x"}}),
            )
        })
    };
    let pending = loop {
        let pending = broker.pending(&fixture.task_id).expect("pending loads");
        if let Some(request) = pending.into_iter().next() {
            break request;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    broker
        .decide(&pending.id, BrokerDecision::Deny, "fayeed")
        .expect("decision records");
    let (text, _) = refusing.join().expect("the tool returns");
    assert_eq!(decision(&text)["behavior"], "deny");
    assert!(
        decision(&text)["message"]
            .as_str()
            .unwrap()
            .contains("fayeed denied")
    );
}

#[derive(Clone, Default)]
struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl SharedBuffer {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("buffer").clone()).expect("utf-8")
    }
}

impl std::io::Write for SharedBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("buffer").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn copy_directory(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("destination");
    for entry in fs::read_dir(source).expect("source") {
        let entry = entry.expect("entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_directory(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy");
        }
    }
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mars-mcp-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).expect("temp");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        for _ in 0..50 {
            if fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
