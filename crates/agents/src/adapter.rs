use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::account::Billing;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterKind {
    Claude,
    Codex,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterProbe {
    pub kind: AdapterKind,
    pub executable: PathBuf,
    pub version: String,
    pub supports_resume: bool,
}

/// An MCP server an agent is given for one run.
///
/// Mars hands every run its own server, scoped to that run's task, so the agent
/// can ask the project graph instead of searching files over and over.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpServer {
    /// The name the agent sees its tools under, as in `mcp__<name>__<tool>`.
    pub name: String,
    pub command: PathBuf,
    pub arguments: Vec<String>,
    pub environment: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgentLaunchRequest {
    pub executable: PathBuf,
    pub cwd: PathBuf,
    /// Delivered on standard input, never as an argument: a context pack is
    /// larger than a Windows command line can carry, and an argument is
    /// visible to every process on the machine that lists command lines.
    pub prompt: String,
    pub resume_session_id: Option<String>,
    pub allowed_tools: Vec<String>,
    pub allow_write: bool,
    /// Network for the backends whose sandbox enforces it themselves.
    pub allow_network: bool,
    pub mcp_servers: Vec<McpServer>,
    /// An MCP tool the CLI asks before anything its allow-list does not
    /// cover. With one, a refusal is an answer the agent can act on rather
    /// than a reason to stop the run.
    pub permission_prompt_tool: Option<String>,
    /// Plugin directories loaded for this session only. The skills inside
    /// them load the way the CLI loads its own.
    pub plugin_dirs: Vec<PathBuf>,
}

impl AgentLaunchRequest {
    /// A read-only request with nothing extra attached.
    #[must_use]
    pub fn new(
        executable: impl Into<PathBuf>,
        cwd: impl Into<PathBuf>,
        prompt: impl Into<String>,
    ) -> Self {
        Self {
            executable: executable.into(),
            cwd: cwd.into(),
            prompt: prompt.into(),
            resume_session_id: None,
            allowed_tools: Vec::new(),
            allow_write: false,
            allow_network: false,
            mcp_servers: Vec::new(),
            permission_prompt_tool: None,
            plugin_dirs: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterCommand {
    pub program: PathBuf,
    pub arguments: Vec<OsString>,
    pub cwd: PathBuf,
    /// Written to the child's standard input, which is then closed.
    pub stdin: Option<String>,
    /// Variables set for the child on top of what it inherits.
    pub environment: Vec<(OsString, OsString)>,
}

impl AdapterCommand {
    /// Run to completion and collect the output, writing [`Self::stdin`] to
    /// the child first. For one-shot work whose answer is a document — a
    /// review, an extraction — rather than a conversation to watch.
    pub fn output(
        &self,
        removed_environment: &[OsString],
    ) -> std::io::Result<std::process::Output> {
        use std::io::Write;
        use std::process::Stdio;

        let mut process = mars_core::process::command(&self.program);
        process
            .args(&self.arguments)
            .current_dir(&self.cwd)
            .envs(self.environment.iter().cloned())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for variable in removed_environment {
            process.env_remove(variable);
        }
        let mut child = process.spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            let input = self.stdin.clone().unwrap_or_default();
            // Written from its own thread so a child that fills its output
            // pipe before reading all of its input cannot deadlock with us.
            std::thread::spawn(move || {
                let _ = stdin.write_all(input.as_bytes());
            });
        }
        child.wait_with_output()
    }
}

/// What a run cost, as the provider reported it at the end.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turns: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum NormalizedAgentEvent {
    Checkpoint {
        session_id: String,
    },
    Text {
        text: String,
    },
    Tool {
        name: String,
        input: Value,
        /// The provider's id for this call, which its result will carry.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
    },
    Command {
        command: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
    },
    FileChange {
        path: String,
        change: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
    },
    Test {
        command: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
    },
    /// What a call came back with. Output is bounded: head and tail are kept,
    /// because a test runner says what failed at the end.
    ToolResult {
        call_id: String,
        output: String,
        is_error: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
    },
    Permission {
        tool: String,
        description: String,
    },
    /// Something the provider wants said that is not the end of the run: a
    /// retry, a transient error, a rate limit it is waiting out.
    Warning {
        message: String,
    },
    Unknown {
        provider_type: String,
        raw: Value,
    },
    End {
        success: bool,
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<RunUsage>,
    },
}

/// The most of one tool result kept on an event.
pub const TOOL_OUTPUT_LIMIT: usize = 64 * 1024;

/// Bound a tool's output to [`TOOL_OUTPUT_LIMIT`], keeping its head and its
/// tail and saying how much was left out between them.
#[must_use]
pub fn bounded_output(output: &str) -> String {
    if output.len() <= TOOL_OUTPUT_LIMIT {
        return output.to_owned();
    }
    let head_budget = TOOL_OUTPUT_LIMIT / 8;
    let tail_budget = TOOL_OUTPUT_LIMIT - head_budget;
    let head_end = floor_char_boundary(output, head_budget);
    let tail_start = ceil_char_boundary(output, output.len() - tail_budget);
    format!(
        "{}\n… {} bytes not kept …\n{}",
        &output[..head_end],
        tail_start - head_end,
        &output[tail_start..]
    )
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("failed to probe {path}: {source}")]
    ProbeLaunch {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("adapter probe failed for {path}: {stderr}")]
    ProbeFailed { path: PathBuf, stderr: String },
    #[error("agent prompt cannot be empty")]
    EmptyPrompt,
}

pub trait AgentAdapter: Send + Sync {
    fn kind(&self) -> AdapterKind;
    fn command(&self, request: &AgentLaunchRequest) -> Result<AdapterCommand, AdapterError>;
    fn parse_value(&self, value: Value) -> Vec<NormalizedAgentEvent>;

    fn probe(&self, executable: &Path) -> Result<AdapterProbe, AdapterError> {
        let output = mars_core::process::command(executable)
            .arg("--version")
            .output()
            .map_err(|source| AdapterError::ProbeLaunch {
                path: executable.to_owned(),
                source,
            })?;
        if !output.status.success() {
            return Err(AdapterError::ProbeFailed {
                path: executable.to_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(AdapterProbe {
            kind: self.kind(),
            executable: executable.to_owned(),
            version: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            supports_resume: true,
        })
    }

    /// The arguments that make this CLI say what it is signed in with, or
    /// `None` for a backend that has no way to.
    fn sign_in_status_arguments(&self) -> Option<&'static [&'static str]> {
        None
    }

    /// Reads that answer. Both streams and the exit status are passed because
    /// the CLIs disagree about where they write it and what failure means.
    fn parse_sign_in(&self, _success: bool, _stdout: &[u8], _stderr: &[u8]) -> Billing {
        Billing::Unknown {
            detail: "this agent does not report what it is signed in with".into(),
        }
    }

    /// Variables that outrank the sign-in in a non-interactive run, highest
    /// precedence first.
    fn overriding_variables(&self) -> &'static [&'static str] {
        &[]
    }

    /// What a person runs in a terminal to sign in.
    fn sign_in_command(&self) -> Option<&'static str> {
        None
    }

    fn on_exit(&self, exit_code: Option<i32>) -> NormalizedAgentEvent {
        NormalizedAgentEvent::End {
            success: exit_code == Some(0),
            message: (exit_code != Some(0)).then(|| match exit_code {
                Some(code) => format!("the agent process exited with code {code}"),
                None => "the agent process was stopped before it exited".to_owned(),
            }),
            usage: None,
        }
    }
}

#[derive(Default)]
pub struct JsonLineDecoder {
    pending: Vec<u8>,
}

impl JsonLineDecoder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, chunk: &[u8], adapter: &dyn AgentAdapter) -> Vec<NormalizedAgentEvent> {
        self.pending.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(position) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<_> = self.pending.drain(..=position).collect();
            let line = &line[..line.len() - 1];
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            match serde_json::from_slice::<Value>(line) {
                Ok(value) => events.extend(adapter.parse_value(value)),
                Err(_) => events.push(NormalizedAgentEvent::Unknown {
                    provider_type: "malformed_json".into(),
                    raw: Value::String(String::from_utf8_lossy(line).into_owned()),
                }),
            }
        }
        events
    }

    pub fn finish(&mut self, adapter: &dyn AgentAdapter) -> Vec<NormalizedAgentEvent> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        self.pending.push(b'\n');
        self.push(&[], adapter)
    }
}

pub(crate) fn validate_request(request: &AgentLaunchRequest) -> Result<(), AdapterError> {
    if request.prompt.trim().is_empty() {
        Err(AdapterError::EmptyPrompt)
    } else {
        Ok(())
    }
}

pub(crate) fn event_type(value: &Value) -> String {
    value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("missing_type")
        .to_owned()
}

/// Whether a shell command runs a test suite.
///
/// Matched on words rather than substrings, so `cat test.txt` and
/// `git commit -m "fix test"` are commands and `pnpm --filter app test` is a
/// test run. Wrappers such as `bash -lc '…'` are seen through because the
/// command is split on quotes as well as spaces.
#[must_use]
pub fn is_test_command(command: &str) -> bool {
    const RUNNERS: [&str; 9] = [
        "pytest",
        "vitest",
        "jest",
        "phpunit",
        "rspec",
        "ctest",
        "nextest",
        "mocha",
        "playwright",
    ];
    const TOOLS: [&str; 14] = [
        "cargo", "npm", "pnpm", "yarn", "bun", "deno", "go", "dotnet", "mvn", "gradle", "gradlew",
        "mix", "make", "swift",
    ];
    let lower = command.to_ascii_lowercase();
    let words: Vec<&str> = lower
        .split(|character: char| {
            character.is_whitespace()
                || matches!(character, '\'' | '"' | ';' | '&' | '|' | '(' | ')')
        })
        .filter(|word| !word.is_empty())
        .collect();
    let program_of = |word: &str| -> String {
        let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
        name.strip_suffix(".exe")
            .or_else(|| name.strip_suffix(".cmd"))
            .unwrap_or(name)
            .to_owned()
    };
    words.iter().enumerate().any(|(index, word)| {
        let program = program_of(word);
        if RUNNERS.contains(&program.as_str()) {
            return true;
        }
        if TOOLS.contains(&program.as_str()) {
            // `pnpm --filter app test`, `npm run test`, `go test ./...`: a
            // `test` word among the next few arguments of the same command.
            return words[index + 1..]
                .iter()
                .take(4)
                .any(|argument| *argument == "test" || argument.starts_with("test:"));
        }
        false
    })
}
