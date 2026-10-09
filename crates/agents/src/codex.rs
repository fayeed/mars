use std::ffi::OsString;

use serde_json::{Value, json};

use crate::account::Billing;
use crate::adapter::{
    AdapterCommand, AdapterError, AdapterKind, AgentAdapter, AgentLaunchRequest,
    NormalizedAgentEvent, event_type, is_test_command, validate_request,
};

#[derive(Default)]
pub struct CodexAdapter;

impl AgentAdapter for CodexAdapter {
    fn kind(&self) -> AdapterKind {
        AdapterKind::Codex
    }

    fn command(&self, request: &AgentLaunchRequest) -> Result<AdapterCommand, AdapterError> {
        validate_request(request)?;
        let mut arguments = vec![OsString::from("exec")];
        if let Some(session_id) = &request.resume_session_id {
            arguments.extend([
                OsString::from("resume"),
                OsString::from("--json"),
                OsString::from(session_id),
            ]);
        } else {
            arguments.extend([
                OsString::from("--json"),
                OsString::from("--color"),
                OsString::from("never"),
                OsString::from("--sandbox"),
                OsString::from(if request.allow_write {
                    "workspace-write"
                } else {
                    "read-only"
                }),
                OsString::from("--cd"),
                request.cwd.clone().into_os_string(),
            ]);
        }
        arguments.push(OsString::from("--"));
        arguments.push(OsString::from(&request.prompt));
        Ok(AdapterCommand {
            program: request.executable.clone(),
            arguments,
            cwd: request.cwd.clone(),
        })
    }

    fn parse_value(&self, value: Value) -> Vec<NormalizedAgentEvent> {
        match event_type(&value).as_str() {
            "thread.started" => value
                .get("thread_id")
                .and_then(Value::as_str)
                .map(|session_id| NormalizedAgentEvent::Checkpoint {
                    session_id: session_id.to_owned(),
                })
                .into_iter()
                .collect(),
            "item.started" | "item.completed" => item_events(&value),
            "approval.requested" => vec![NormalizedAgentEvent::Permission {
                tool: value
                    .get("tool")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned(),
                description: value
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex requested approval")
                    .to_owned(),
            }],
            "turn.completed" => vec![NormalizedAgentEvent::End {
                success: true,
                message: None,
            }],
            "turn.failed" => vec![NormalizedAgentEvent::End {
                success: false,
                message: value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            }],
            provider_type => vec![NormalizedAgentEvent::Unknown {
                provider_type: provider_type.to_owned(),
                raw: value,
            }],
        }
    }

    fn sign_in_status_arguments(&self) -> Option<&'static [&'static str]> {
        Some(&["login", "status"])
    }

    /// Reads `codex login status`, which writes one sentence to stderr and
    /// exits non-zero when nobody is signed in.
    fn parse_sign_in(&self, success: bool, stdout: &[u8], stderr: &[u8]) -> Billing {
        let output = format!(
            "{}\n{}",
            String::from_utf8_lossy(stderr),
            String::from_utf8_lossy(stdout)
        );
        if output.contains("Logged in using ChatGPT") {
            return Billing::Plan { plan: None };
        }
        if output.contains("Logged in using an API key") {
            return Billing::ApiKey {
                source: "saved API key".into(),
            };
        }
        if output.contains("Not logged in") {
            return Billing::SignedOut;
        }
        // Codex prints warnings ahead of the answer, so the last line is the
        // one most likely to be it.
        let last = output
            .lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .unwrap_or_default();
        Billing::Unknown {
            detail: if success || last.is_empty() {
                "Codex did not say what it is signed in with".into()
            } else {
                last.to_owned()
            },
        }
    }

    /// `codex exec` uses this over a saved ChatGPT sign-in for the run.
    fn overriding_variables(&self) -> &'static [&'static str] {
        &["CODEX_API_KEY"]
    }

    fn sign_in_command(&self) -> Option<&'static str> {
        Some("codex login")
    }
}

fn item_events(value: &Value) -> Vec<NormalizedAgentEvent> {
    let item = value.get("item").unwrap_or(value);
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed")
        .to_owned();
    match item.get("type").and_then(Value::as_str) {
        Some("agent_message") => item
            .get("text")
            .and_then(Value::as_str)
            .map(|text| NormalizedAgentEvent::Text {
                text: text.to_owned(),
            })
            .into_iter()
            .collect(),
        Some("command_execution") => {
            let command = item
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            vec![if is_test_command(&command) {
                NormalizedAgentEvent::Test { command, status }
            } else {
                NormalizedAgentEvent::Command { command, status }
            }]
        }
        Some("file_change") => file_change_events(item),
        Some("mcp_tool_call" | "web_search") => vec![NormalizedAgentEvent::Tool {
            name: item
                .get("name")
                .or_else(|| item.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            input: item.get("arguments").cloned().unwrap_or_else(|| json!({})),
        }],
        Some(provider_type) => vec![NormalizedAgentEvent::Unknown {
            provider_type: provider_type.to_owned(),
            raw: item.clone(),
        }],
        None => vec![NormalizedAgentEvent::Unknown {
            provider_type: "missing_item_type".into(),
            raw: item.clone(),
        }],
    }
}

fn file_change_events(item: &Value) -> Vec<NormalizedAgentEvent> {
    let changes = item
        .get("changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|change| {
            Some(NormalizedAgentEvent::FileChange {
                path: change.get("path")?.as_str()?.to_owned(),
                change: change
                    .get("kind")
                    .or_else(|| change.get("change"))
                    .and_then(Value::as_str)
                    .unwrap_or("modified")
                    .to_owned(),
            })
        })
        .collect::<Vec<_>>();
    if !changes.is_empty() {
        return changes;
    }
    vec![NormalizedAgentEvent::FileChange {
        path: item
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned(),
        change: item
            .get("change")
            .and_then(Value::as_str)
            .unwrap_or("modified")
            .to_owned(),
    }]
}
