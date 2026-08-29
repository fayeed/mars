use std::ffi::OsString;

use serde_json::{Value, json};

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
        Some("file_change") => vec![NormalizedAgentEvent::FileChange {
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
        }],
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
