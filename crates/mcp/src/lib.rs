//! Mars as an MCP server: the project graph, memory and decisions, offered to
//! the agent CLIs as tools.
//!
//! Without this an agent learns a repository the way a stranger would — by
//! listing directories and grepping for names, again on every run, and paying
//! for every line it reads. Mars has already indexed the repository: its
//! symbols, their references and call edges, import graph, chunks with
//! embeddings, history, and the decisions and memories people approved. This
//! server hands that to the agent directly, scoped to one task, in a compact
//! text form written to be cheap to read.
//!
//! It speaks MCP over stdio (newline-delimited JSON-RPC 2.0). Every run gets
//! its own process, started by the agent CLI from the arguments Mars put in its
//! configuration, reading the same SQLite database the app writes. It writes
//! only where an agent is allowed to: proposals that wait for a person
//! (memories, decisions) and permission requests that wait for a person too.
//!
//! ```text
//! mars-mcp --database <path> --repository <id> [--task <id>] [--run <id>]
//!          [--worktree <path>] [--role implementer|reviewer] [--scope <glob>]...
//!          [--agent <label>] [--readable <dir>]... [--permission-tool]
//! ```

mod permission;
mod render;
mod tools;

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

pub use tools::ToolContext;

/// The MCP revisions this server can speak, newest first.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// What one server process is scoped to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerConfig {
    pub database: PathBuf,
    pub repository_id: String,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    /// The task's worktree, where the agent works. Paths it reports are
    /// resolved against this.
    pub worktree: Option<PathBuf>,
    /// `implementer` or `reviewer`; a reviewer is read-only by construction.
    pub role: String,
    /// The file globs the approved worktree plan declared.
    pub scope: Vec<String>,
    /// Who the agent is, for the record: `claude (implementer)`.
    pub agent: String,
    /// Directories outside the worktree the agent may read, such as the
    /// skills Mars loaded for this run.
    pub readable: Vec<PathBuf>,
    /// Offer `permission_prompt`, the tool Claude Code asks before acting.
    pub permission_tool: bool,
}

impl ServerConfig {
    /// Parse the command line, without the program name or the `mcp` verb.
    pub fn from_args(arguments: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut config = Self {
            role: "implementer".into(),
            agent: "agent".into(),
            ..Self::default()
        };
        let mut arguments = arguments.into_iter();
        while let Some(flag) = arguments.next() {
            let mut value = |name: &str| {
                arguments
                    .next()
                    .ok_or_else(|| format!("{name} needs a value"))
            };
            match flag.as_str() {
                "--database" => config.database = PathBuf::from(value("--database")?),
                "--repository" => config.repository_id = value("--repository")?,
                "--task" => config.task_id = Some(value("--task")?),
                "--run" => config.run_id = Some(value("--run")?),
                "--worktree" => config.worktree = Some(PathBuf::from(value("--worktree")?)),
                "--role" => config.role = value("--role")?,
                "--scope" => config.scope.push(value("--scope")?),
                "--agent" => config.agent = value("--agent")?,
                "--readable" => config.readable.push(PathBuf::from(value("--readable")?)),
                "--permission-tool" => config.permission_tool = true,
                other => return Err(format!("unknown argument `{other}`")),
            }
        }
        if config.database.as_os_str().is_empty() {
            return Err("--database is required".into());
        }
        if config.repository_id.is_empty() {
            return Err("--repository is required".into());
        }
        if !matches!(config.role.as_str(), "implementer" | "reviewer") {
            return Err(format!(
                "--role must be implementer or reviewer, not `{}`",
                config.role
            ));
        }
        Ok(config)
    }

    /// The arguments that reproduce this configuration, for an agent CLI's
    /// MCP settings.
    #[must_use]
    pub fn to_args(&self) -> Vec<String> {
        let mut arguments = vec![
            "--database".to_owned(),
            self.database.to_string_lossy().into_owned(),
            "--repository".to_owned(),
            self.repository_id.clone(),
            "--role".to_owned(),
            self.role.clone(),
            "--agent".to_owned(),
            self.agent.clone(),
        ];
        if let Some(task_id) = &self.task_id {
            arguments.extend(["--task".to_owned(), task_id.clone()]);
        }
        if let Some(run_id) = &self.run_id {
            arguments.extend(["--run".to_owned(), run_id.clone()]);
        }
        if let Some(worktree) = &self.worktree {
            arguments.extend([
                "--worktree".to_owned(),
                worktree.to_string_lossy().into_owned(),
            ]);
        }
        for glob in &self.scope {
            arguments.extend(["--scope".to_owned(), glob.clone()]);
        }
        for directory in &self.readable {
            arguments.extend([
                "--readable".to_owned(),
                directory.to_string_lossy().into_owned(),
            ]);
        }
        if self.permission_tool {
            arguments.push("--permission-tool".to_owned());
        }
        arguments
    }
}

/// Run the server on this process's stdio until the client hangs up. Returns
/// a process exit code.
#[must_use]
pub fn run_from_args(arguments: impl IntoIterator<Item = String>) -> i32 {
    let config = match ServerConfig::from_args(arguments) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("mars mcp: {error}");
            return 2;
        }
    };
    let context = match ToolContext::open(config) {
        Ok(context) => Arc::new(context),
        Err(error) => {
            eprintln!("mars mcp: {error}");
            return 1;
        }
    };
    let stdin = std::io::stdin();
    serve(context, stdin.lock(), std::io::stdout());
    0
}

type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// Serve requests read from `input`, writing responses to `output`.
///
/// Tool calls run on their own threads: a permission prompt can wait minutes
/// for a person, and the client must still get answers to everything else it
/// asks meanwhile — pings, other tools, cancellations.
pub fn serve(context: Arc<ToolContext>, input: impl BufRead, output: impl Write + Send + 'static) {
    let writer: SharedWriter = Arc::new(Mutex::new(Box::new(output)));
    // Per request: `stop` tells a waiting tool to give up, and `withdrawn`
    // says the client cancelled it, so no answer is owed.
    let cancelled: Arc<Mutex<HashMap<String, InFlight>>> = Arc::default();
    let mut workers = Vec::new();

    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(&line) {
            Ok(message) => message,
            Err(error) => {
                write_message(
                    &writer,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": Value::Null,
                        "error": { "code": -32700, "message": format!("parse error: {error}") }
                    }),
                );
                continue;
            }
        };
        let id = message.get("id").cloned();
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        match (method.as_str(), id) {
            ("initialize", Some(id)) => {
                let requested = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(PROTOCOL_VERSIONS[0]);
                let version = PROTOCOL_VERSIONS
                    .iter()
                    .find(|version| **version == requested)
                    .copied()
                    .unwrap_or(PROTOCOL_VERSIONS[0]);
                respond(
                    &writer,
                    id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": { "tools": { "listChanged": false } },
                        "serverInfo": { "name": "mars", "version": env!("CARGO_PKG_VERSION") },
                        "instructions": tools::INSTRUCTIONS,
                    }),
                );
            }
            ("ping", Some(id)) => respond(&writer, id, json!({})),
            ("tools/list", Some(id)) => {
                respond(&writer, id, json!({ "tools": context.definitions() }));
            }
            ("tools/call", Some(id)) => {
                workers.retain(|worker: &std::thread::JoinHandle<()>| !worker.is_finished());
                let in_flight = InFlight::default();
                let flag = in_flight.stop.clone();
                let withdrawn = in_flight.withdrawn.clone();
                if let Ok(mut map) = cancelled.lock() {
                    map.insert(id.to_string(), in_flight);
                }
                let context = context.clone();
                let writer = writer.clone();
                let cancelled = cancelled.clone();
                workers.push(std::thread::spawn(move || {
                    let name = params
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                    let result = context.call(&name, &arguments, &flag);
                    if let Ok(mut map) = cancelled.lock() {
                        map.remove(&id.to_string());
                    }
                    // A request the client cancelled gets no response: it has
                    // already forgotten it.
                    if !withdrawn.load(Ordering::SeqCst) {
                        respond(&writer, id, result.into_json());
                    }
                }));
            }
            ("notifications/cancelled", None) => {
                if let Some(request) = params.get("requestId")
                    && let Ok(map) = cancelled.lock()
                    && let Some(in_flight) = map.get(&request.to_string())
                {
                    in_flight.withdrawn.store(true, Ordering::SeqCst);
                    in_flight.stop.store(true, Ordering::SeqCst);
                }
            }
            // Other notifications need no answer.
            (_, None) => {}
            (other, Some(id)) => write_message(
                &writer,
                &json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("method `{other}` is not supported") }
                }),
            ),
        }
    }

    // The client stopped sending. Work already under way finishes and is
    // answered, but anything waiting on a person stops waiting: there is no
    // longer anyone to hand the answer to.
    if let Ok(map) = cancelled.lock() {
        for in_flight in map.values() {
            in_flight.stop.store(true, Ordering::SeqCst);
        }
    }
    for worker in workers {
        let _ = worker.join();
    }
}

#[derive(Default)]
struct InFlight {
    stop: Arc<AtomicBool>,
    withdrawn: Arc<AtomicBool>,
}

fn respond(writer: &SharedWriter, id: Value, result: Value) {
    write_message(
        writer,
        &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
    );
}

fn write_message(writer: &SharedWriter, message: &Value) {
    if let Ok(mut writer) = writer.lock() {
        // One message per line: serde_json never emits a raw newline inside
        // a string, so the framing holds.
        let _ = writeln!(writer, "{message}");
        let _ = writer.flush();
    }
}

/// What a tool call produced.
pub struct ToolOutcome {
    pub text: String,
    pub is_error: bool,
}

impl ToolOutcome {
    #[must_use]
    pub fn ok(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
        }
    }

    #[must_use]
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
        }
    }

    fn into_json(self) -> Value {
        let mut result = json!({ "content": [{ "type": "text", "text": self.text }] });
        if self.is_error {
            result["isError"] = Value::Bool(true);
        }
        result
    }
}
