use std::ffi::OsString;

use serde_json::{Value, json};

use crate::adapter::{
    AdapterCommand, AdapterError, AdapterKind, AgentAdapter, AgentLaunchRequest,
    NormalizedAgentEvent, event_type, is_test_command, validate_request,
};

#[derive(Default)]
pub struct ClaudeAdapter;

impl AgentAdapter for ClaudeAdapter {
    fn kind(&self) -> AdapterKind {
        AdapterKind::Claude
    }

    fn command(&self, request: &AgentLaunchRequest) -> Result<AdapterCommand, AdapterError> {
        validate_request(request)?;
        let mut arguments = vec![
            OsString::from("-p"),
            OsString::from("--output-format"),
            OsString::from("stream-json"),
            OsString::from("--verbose"),
            OsString::from("--permission-mode"),
            OsString::from("dontAsk"),
        ];
        if !request.allowed_tools.is_empty() {
            arguments.push(OsString::from("--allowedTools"));
            arguments.push(OsString::from(request.allowed_tools.join(",")));
        }
        if let Some(session_id) = &request.resume_session_id {
            arguments.push(OsString::from("--resume"));
            arguments.push(OsString::from(session_id));
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
            "system" if value.get("subtype").and_then(Value::as_str) == Some("init") => value
                .get("session_id")
                .and_then(Value::as_str)
                .map(|session_id| NormalizedAgentEvent::Checkpoint {
                    session_id: session_id.to_owned(),
                })
                .into_iter()
                .collect(),
            "assistant" => assistant_events(&value),
            "permission_request" => vec![NormalizedAgentEvent::Permission {
                tool: value
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned(),
                description: value
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("Claude requested permission")
                    .to_owned(),
            }],
            "result" => vec![NormalizedAgentEvent::End {
                success: !value
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                message: value
                    .get("result")
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

fn assistant_events(value: &Value) -> Vec<NormalizedAgentEvent> {
    value
        .pointer("/message/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(
            |content| match content.get("type").and_then(Value::as_str) {
                Some("text") => content.get("text").and_then(Value::as_str).map(|text| {
                    NormalizedAgentEvent::Text {
                        text: text.to_owned(),
                    }
                }),
                Some("tool_use") => Some(tool_event(content)),
                _ => Some(NormalizedAgentEvent::Unknown {
                    provider_type: "assistant_content".into(),
                    raw: content.clone(),
                }),
            },
        )
        .collect()
}

fn tool_event(content: &Value) -> NormalizedAgentEvent {
    let name = content
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let input = content.get("input").cloned().unwrap_or_else(|| json!({}));
    if name == "Bash" {
        let command = input
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        return if is_test_command(&command) {
            NormalizedAgentEvent::Test {
                command,
                status: "started".into(),
            }
        } else {
            NormalizedAgentEvent::Command {
                command,
                status: "started".into(),
            }
        };
    }
    if matches!(name, "Edit" | "Write" | "NotebookEdit") {
        let path = input
            .get("file_path")
            .or_else(|| input.get("path"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        return NormalizedAgentEvent::FileChange {
            path,
            change: name.to_ascii_lowercase(),
        };
    }
    NormalizedAgentEvent::Tool {
        name: name.to_owned(),
        input,
    }
}
