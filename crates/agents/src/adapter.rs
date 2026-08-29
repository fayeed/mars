use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentLaunchRequest {
    pub executable: PathBuf,
    pub cwd: PathBuf,
    pub prompt: String,
    pub resume_session_id: Option<String>,
    pub allowed_tools: Vec<String>,
    pub allow_write: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterCommand {
    pub program: PathBuf,
    pub arguments: Vec<OsString>,
    pub cwd: PathBuf,
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
    },
    Command {
        command: String,
        status: String,
    },
    FileChange {
        path: String,
        change: String,
    },
    Test {
        command: String,
        status: String,
    },
    Permission {
        tool: String,
        description: String,
    },
    Unknown {
        provider_type: String,
        raw: Value,
    },
    End {
        success: bool,
        message: Option<String>,
    },
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
        let output = Command::new(executable)
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

    fn on_exit(&self, exit_code: Option<i32>) -> NormalizedAgentEvent {
        NormalizedAgentEvent::End {
            success: exit_code == Some(0),
            message: (exit_code != Some(0)).then(|| format!("process exited with {exit_code:?}")),
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

pub(crate) fn is_test_command(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    [" test", "test ", "pytest", "cargo test", "vitest", "jest"]
        .iter()
        .any(|needle| lower.contains(needle))
}
