use std::ffi::OsString;

use serde_json::{Value, json};

use crate::account::Billing;
use crate::adapter::{
    AdapterCommand, AdapterError, AdapterKind, AgentAdapter, AgentLaunchRequest,
    NormalizedAgentEvent, RunUsage, bounded_output, event_type, is_test_command, validate_request,
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
        if request.resume_session_id.is_some() {
            arguments.push(OsString::from("resume"));
        }
        arguments.extend([
            OsString::from("--json"),
            OsString::from("--skip-git-repo-check"),
        ]);
        // Set through configuration rather than `--sandbox`, which `exec
        // resume` does not accept: a follow-up that dropped the flag would run
        // under whatever the person's own config says instead of this grant.
        let sandbox = if request.allow_write {
            "workspace-write"
        } else {
            "read-only"
        };
        config(&mut arguments, "sandbox_mode", &toml_string(sandbox));
        // `codex exec` has nobody to ask, so nothing waits on an approval: the
        // sandbox is the whole of the grant.
        config(&mut arguments, "approval_policy", &toml_string("never"));
        if request.allow_write {
            config(
                &mut arguments,
                "sandbox_workspace_write.network_access",
                if request.allow_network {
                    "true"
                } else {
                    "false"
                },
            );
        }
        for server in &request.mcp_servers {
            let key = format!("mcp_servers.{}", server.name);
            config(
                &mut arguments,
                &format!("{key}.command"),
                &toml_string(&server.command.to_string_lossy()),
            );
            config(
                &mut arguments,
                &format!("{key}.args"),
                &format!(
                    "[{}]",
                    server
                        .arguments
                        .iter()
                        .map(|argument| toml_string(argument))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
            if !server.environment.is_empty() {
                config(
                    &mut arguments,
                    &format!("{key}.env"),
                    &format!(
                        "{{ {} }}",
                        server
                            .environment
                            .iter()
                            .map(|(name, value)| format!(
                                "{} = {}",
                                toml_string(name),
                                toml_string(value)
                            ))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
            }
            config(&mut arguments, &format!("{key}.tool_timeout_sec"), "300");
        }
        if request.resume_session_id.is_none() {
            arguments.extend([
                OsString::from("--color"),
                OsString::from("never"),
                OsString::from("--cd"),
                request.cwd.clone().into_os_string(),
            ]);
        }
        if let Some(session_id) = &request.resume_session_id {
            arguments.push(OsString::from(session_id));
        }
        // `-` reads the prompt from stdin.
        arguments.push(OsString::from("-"));
        Ok(AdapterCommand {
            program: request.executable.clone(),
            arguments,
            cwd: request.cwd.clone(),
            stdin: Some(request.prompt.clone()),
            environment: Vec::new(),
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
            "item.started" => started_item_events(&value),
            "item.completed" => completed_item_events(&value),
            // A turn starting, and to-do lists being revised, are progress
            // the transcript already shows through the items themselves.
            "turn.started" | "item.updated" => Vec::new(),
            "error" => vec![NormalizedAgentEvent::Warning {
                message: value
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex reported an error and is retrying")
                    .to_owned(),
            }],
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
                usage: Some(turn_usage(&value)),
            }],
            "turn.failed" => vec![NormalizedAgentEvent::End {
                success: false,
                message: value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                usage: None,
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

/// Add one `-c key=value` override. The value is TOML.
fn config(arguments: &mut Vec<OsString>, key: &str, value: &str) {
    arguments.push(OsString::from("-c"));
    arguments.push(OsString::from(format!("{key}={value}")));
}

/// A TOML basic string. Windows paths are full of backslashes, which a bare
/// `"C:\Users"` would read as a malformed `\U` escape.
fn toml_string(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            control if control.is_control() => {
                quoted.push_str(&format!("\\u{:04X}", u32::from(control)));
            }
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

fn item_of(value: &Value) -> &Value {
    value.get("item").unwrap_or(value)
}

fn item_id(item: &Value) -> Option<String> {
    item.get("id").and_then(Value::as_str).map(str::to_owned)
}

fn mcp_tool_name(item: &Value) -> String {
    let tool = item
        .get("tool")
        .or_else(|| item.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    match item.get("server").and_then(Value::as_str) {
        Some(server) => format!("mcp__{server}__{tool}"),
        None => tool.to_owned(),
    }
}

/// An item beginning: a command or a tool call is reported as it starts, so
/// the transcript shows what the agent is doing while it does it.
fn started_item_events(value: &Value) -> Vec<NormalizedAgentEvent> {
    let item = item_of(value);
    match item.get("type").and_then(Value::as_str) {
        Some("command_execution") => {
            let command = item
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let call_id = item_id(item);
            let status = "started".to_owned();
            vec![if is_test_command(&command) {
                NormalizedAgentEvent::Test {
                    command,
                    status,
                    call_id,
                }
            } else {
                NormalizedAgentEvent::Command {
                    command,
                    status,
                    call_id,
                }
            }]
        }
        Some("mcp_tool_call") => vec![NormalizedAgentEvent::Tool {
            name: mcp_tool_name(item),
            input: item.get("arguments").cloned().unwrap_or_else(|| json!({})),
            call_id: item_id(item),
        }],
        Some("web_search") => vec![NormalizedAgentEvent::Tool {
            name: "web_search".into(),
            input: json!({ "query": item.get("query").cloned().unwrap_or(Value::Null) }),
            call_id: item_id(item),
        }],
        // Messages and file changes are reported once, complete.
        Some(_) | None => Vec::new(),
    }
}

/// An item finishing. A command's completion carries its output and exit
/// code; a message or a file change is reported only here.
fn completed_item_events(value: &Value) -> Vec<NormalizedAgentEvent> {
    let item = item_of(value);
    let failed = item.get("status").and_then(Value::as_str) == Some("failed");
    match item.get("type").and_then(Value::as_str) {
        Some("agent_message") => item
            .get("text")
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .map(|text| NormalizedAgentEvent::Text {
                text: text.to_owned(),
            })
            .into_iter()
            .collect(),
        Some("command_execution") => {
            let exit_code = item
                .get("exit_code")
                .and_then(Value::as_i64)
                .and_then(|code| i32::try_from(code).ok());
            match item_id(item) {
                Some(call_id) => vec![NormalizedAgentEvent::ToolResult {
                    call_id,
                    output: bounded_output(
                        item.get("aggregated_output")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    ),
                    is_error: failed || exit_code.is_some_and(|code| code != 0),
                    exit_code,
                }],
                // A completion with nothing to pair it with is still a command
                // that ran.
                None => {
                    let command = item
                        .get("command")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let status = if failed { "failed" } else { "completed" }.to_owned();
                    vec![if is_test_command(&command) {
                        NormalizedAgentEvent::Test {
                            command,
                            status,
                            call_id: None,
                        }
                    } else {
                        NormalizedAgentEvent::Command {
                            command,
                            status,
                            call_id: None,
                        }
                    }]
                }
            }
        }
        Some("file_change") => file_change_events(item),
        Some("mcp_tool_call") => match item_id(item) {
            Some(call_id) => vec![NormalizedAgentEvent::ToolResult {
                call_id,
                output: bounded_output(&mcp_result_text(item)),
                is_error: failed || item.get("error").is_some_and(|error| !error.is_null()),
                exit_code: None,
            }],
            None => Vec::new(),
        },
        // Reasoning is neither requested nor kept (Architecture §17.7). To-do
        // lists and searches are progress the next message reports.
        Some("reasoning" | "todo_list" | "web_search") => Vec::new(),
        Some("error") => vec![NormalizedAgentEvent::Warning {
            message: item
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Codex reported an error")
                .to_owned(),
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

fn mcp_result_text(item: &Value) -> String {
    if let Some(message) = item.pointer("/error/message").and_then(Value::as_str) {
        return message.to_owned();
    }
    item.pointer("/result/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

fn turn_usage(value: &Value) -> RunUsage {
    let number = |pointer: &str| value.pointer(pointer).and_then(Value::as_u64);
    RunUsage {
        input_tokens: number("/usage/input_tokens"),
        output_tokens: number("/usage/output_tokens"),
        cached_input_tokens: number("/usage/cached_input_tokens"),
        cost_usd: None,
        turns: Some(1),
    }
}

fn file_change_events(item: &Value) -> Vec<NormalizedAgentEvent> {
    let call_id = item_id(item);
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
                call_id: call_id.clone(),
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
        call_id,
    }]
}
