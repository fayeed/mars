use std::ffi::OsString;

use serde_json::{Value, json};

use crate::account::Billing;
use crate::adapter::{
    AdapterCommand, AdapterError, AdapterKind, AgentAdapter, AgentLaunchRequest,
    NormalizedAgentEvent, RunUsage, bounded_output, event_type, is_test_command, validate_request,
};

/// How long Claude Code waits on one MCP tool call, in milliseconds, when a
/// permission prompt is routed through one. A person deciding is not a
/// timeout; the default would deny a request because nobody answered fast.
const PERMISSION_WAIT_MS: &str = "86400000";

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
            // With a prompt tool, anything outside the allow-list is asked
            // about through it. Without one there is nobody to ask, so it is
            // refused outright rather than silently allowed.
            OsString::from(if request.permission_prompt_tool.is_some() {
                "default"
            } else {
                "dontAsk"
            }),
        ];
        let mut allowed_tools = request.allowed_tools.clone();
        // A server Mars gives the run is one it means the agent to use; in
        // `dontAsk` mode an unlisted server would be configured and unusable.
        for server in &request.mcp_servers {
            let rule = format!("mcp__{}", server.name);
            if !allowed_tools.contains(&rule) {
                allowed_tools.push(rule);
            }
        }
        if !allowed_tools.is_empty() {
            arguments.push(OsString::from("--allowedTools"));
            arguments.push(OsString::from(allowed_tools.join(",")));
        }
        if !request.mcp_servers.is_empty() {
            arguments.push(OsString::from("--mcp-config"));
            arguments.push(OsString::from(mcp_config(request).to_string()));
        }
        if let Some(tool) = &request.permission_prompt_tool {
            arguments.push(OsString::from("--permission-prompt-tool"));
            arguments.push(OsString::from(tool));
        }
        for directory in &request.plugin_dirs {
            arguments.push(OsString::from("--plugin-dir"));
            arguments.push(directory.clone().into_os_string());
        }
        if let Some(session_id) = &request.resume_session_id {
            arguments.push(OsString::from("--resume"));
            arguments.push(OsString::from(session_id));
        }
        let mut environment = Vec::new();
        if request.permission_prompt_tool.is_some() {
            environment.push((
                OsString::from("MCP_TOOL_TIMEOUT"),
                OsString::from(PERMISSION_WAIT_MS),
            ));
        }
        Ok(AdapterCommand {
            program: request.executable.clone(),
            arguments,
            cwd: request.cwd.clone(),
            // `claude -p` with no prompt argument reads the prompt from stdin.
            stdin: Some(request.prompt.clone()),
            environment,
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
            // Status, compaction and hook notices: the CLI's own bookkeeping,
            // kept in the raw frames and not something a reader acts on.
            "system" | "stream_event" => Vec::new(),
            "rate_limit_event" => rate_limit_events(&value),
            "assistant" => assistant_events(&value),
            "user" => user_events(&value),
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
                usage: Some(result_usage(&value)),
            }],
            provider_type => vec![NormalizedAgentEvent::Unknown {
                provider_type: provider_type.to_owned(),
                raw: value,
            }],
        }
    }

    fn sign_in_status_arguments(&self) -> Option<&'static [&'static str]> {
        Some(&["auth", "status", "--json"])
    }

    /// Reads `claude auth status --json`, which answers in JSON whether or not
    /// anyone is signed in; only the exit status differs.
    fn parse_sign_in(&self, _success: bool, stdout: &[u8], _stderr: &[u8]) -> Billing {
        let Ok(status) = serde_json::from_slice::<Value>(stdout) else {
            return Billing::Unknown {
                detail: "this Claude Code does not report its sign-in; updating it lets Mars check"
                    .into(),
            };
        };
        let text = |key: &str| status.get(key).and_then(Value::as_str);
        if status.get("loggedIn").and_then(Value::as_bool) != Some(true) {
            return Billing::SignedOut;
        }
        if let Some(provider) = text("apiProvider").filter(|provider| *provider != "firstParty") {
            return Billing::CloudProvider {
                provider: provider.to_owned(),
            };
        }
        // An `apiKeyHelper` or a key saved by a Console login outranks a
        // claude.ai sign-in, and the CLI says so here rather than in
        // `authMethod`, which keeps naming the sign-in.
        if let Some(source) = text("apiKeySource") {
            return Billing::ApiKey {
                source: source.to_owned(),
            };
        }
        match text("authMethod") {
            Some("claude.ai" | "oauth_token") => Billing::Plan {
                plan: text("subscriptionType").map(str::to_owned),
            },
            Some("api_key") => Billing::ApiKey {
                source: "Claude Console".into(),
            },
            Some("none") | None => Billing::SignedOut,
            Some(other) => Billing::Unknown {
                detail: format!("signed in with {other}"),
            },
        }
    }

    /// `claude -p` uses these over a claude.ai sign-in whenever they are set:
    /// the bearer token first, then the API key.
    fn overriding_variables(&self) -> &'static [&'static str] {
        &["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"]
    }

    fn sign_in_command(&self) -> Option<&'static str> {
        Some("claude auth login")
    }
}

/// The `--mcp-config` document for the servers a run is given.
fn mcp_config(request: &AgentLaunchRequest) -> Value {
    let servers: serde_json::Map<String, Value> = request
        .mcp_servers
        .iter()
        .map(|server| {
            let environment: serde_json::Map<String, Value> = server
                .environment
                .iter()
                .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                .collect();
            (
                server.name.clone(),
                json!({
                    "type": "stdio",
                    "command": server.command.to_string_lossy(),
                    "args": server.arguments,
                    "env": environment,
                }),
            )
        })
        .collect();
    json!({ "mcpServers": servers })
}

fn assistant_events(value: &Value) -> Vec<NormalizedAgentEvent> {
    value
        .pointer("/message/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(
            |content| match content.get("type").and_then(Value::as_str) {
                Some("text") => content
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|text| !text.trim().is_empty())
                    .map(|text| NormalizedAgentEvent::Text {
                        text: text.to_owned(),
                    }),
                Some("tool_use") => Some(tool_event(content)),
                // Reasoning is neither requested nor kept (Architecture §17.7).
                // It is dropped here so it never reaches the activity record,
                // the transcript, or a handoff.
                Some("thinking" | "redacted_thinking") => None,
                _ => Some(NormalizedAgentEvent::Unknown {
                    provider_type: "assistant_content".into(),
                    raw: content.clone(),
                }),
            },
        )
        .collect()
}

/// The CLI reports each tool's result as a `user` turn holding `tool_result`
/// blocks. Anything else in a user turn is the prompt echoed back.
fn user_events(value: &Value) -> Vec<NormalizedAgentEvent> {
    value
        .pointer("/message/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|content| content.get("type").and_then(Value::as_str) == Some("tool_result"))
        .filter_map(|content| {
            let call_id = content.get("tool_use_id").and_then(Value::as_str)?;
            Some(NormalizedAgentEvent::ToolResult {
                call_id: call_id.to_owned(),
                output: bounded_output(&tool_result_text(content.get("content"))),
                is_error: content
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                exit_code: None,
            })
        })
        .collect()
}

/// A tool result's content is either a string or a list of blocks; only the
/// text blocks are kept.
fn tool_result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// A rate limit is worth a line only when the CLI is actually held by it.
fn rate_limit_events(value: &Value) -> Vec<NormalizedAgentEvent> {
    let status = value
        .pointer("/rate_limit_info/status")
        .and_then(Value::as_str)
        .unwrap_or("allowed");
    if status == "allowed" {
        return Vec::new();
    }
    vec![NormalizedAgentEvent::Warning {
        message: format!("Claude is rate limited ({status}) and is waiting before it continues"),
    }]
}

fn result_usage(value: &Value) -> RunUsage {
    let number = |pointer: &str| value.pointer(pointer).and_then(Value::as_u64);
    RunUsage {
        input_tokens: number("/usage/input_tokens"),
        output_tokens: number("/usage/output_tokens"),
        cached_input_tokens: number("/usage/cache_read_input_tokens"),
        cost_usd: value.get("total_cost_usd").and_then(Value::as_f64),
        turns: number("/num_turns"),
    }
}

fn tool_event(content: &Value) -> NormalizedAgentEvent {
    let name = content
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let call_id = content.get("id").and_then(Value::as_str).map(str::to_owned);
    let input = content.get("input").cloned().unwrap_or_else(|| json!({}));
    if matches!(name, "Bash" | "PowerShell") {
        let command = input
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        return if is_test_command(&command) {
            NormalizedAgentEvent::Test {
                command,
                status: "started".into(),
                call_id,
            }
        } else {
            NormalizedAgentEvent::Command {
                command,
                status: "started".into(),
                call_id,
            }
        };
    }
    if matches!(name, "Edit" | "MultiEdit" | "Write" | "NotebookEdit") {
        let path = input
            .get("file_path")
            .or_else(|| input.get("notebook_path"))
            .or_else(|| input.get("path"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        return NormalizedAgentEvent::FileChange {
            path,
            change: name.to_ascii_lowercase(),
            call_id,
        };
    }
    NormalizedAgentEvent::Tool {
        name: name.to_owned(),
        input,
        call_id,
    }
}
