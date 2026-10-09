//! The permission prompt: Claude Code asks this before any action its
//! allow-list does not cover, and waits for the answer.
//!
//! This is what makes "anything else stops and asks a person" true rather than
//! a sentence in a prompt. The CLI asks *before* it acts, so a refusal is an
//! answer it can work with instead of something that has already happened.
//!
//! - Read-only commands, and writes inside the task's declared scope, are what
//!   the run was granted, so they are allowed without asking.
//! - Writes outside the worktree, or into Git's own directory, are refused:
//!   the worktree is the whole of where this task may change anything.
//! - Everything else is put to the permission broker. A standing grant answers
//!   at once; otherwise the request waits in the task's conversation until a
//!   person decides, and this tool waits with it.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mars_security::{BrokerOutcome, BrokerRequest, CommandClass, SideEffect, classify_command};
use serde_json::{Value, json};

use crate::ToolOutcome;
use crate::tools::ToolContext;

const POLL: Duration = Duration::from_millis(250);

/// Tools that only read, or only organise the agent's own work.
const HARMLESS: &[&str] = &[
    "Task",
    "Agent",
    "TodoWrite",
    "TodoRead",
    "Skill",
    "ExitPlanMode",
    "BashOutput",
    "KillShell",
    "KillBash",
    "ListMcpResourcesTool",
    "ReadMcpResourceTool",
];

enum Verdict {
    Allow,
    Deny(String),
    Ask(SideEffect),
}

pub fn decide(context: &ToolContext, arguments: &Value, cancelled: &AtomicBool) -> ToolOutcome {
    let tool = arguments
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let input = arguments.get("input").cloned().unwrap_or_else(|| json!({}));
    let verdict = match classify(context, tool, &input) {
        Ok(verdict) => verdict,
        Err(message) => Verdict::Deny(message),
    };
    match verdict {
        Verdict::Allow => allow(&input),
        Verdict::Deny(message) => deny(&message),
        Verdict::Ask(effect) => ask(context, effect, &input, cancelled),
    }
}

fn classify(context: &ToolContext, tool: &str, input: &Value) -> Result<Verdict, String> {
    let worktree =
        context.config.worktree.clone().ok_or(
            "this run has no worktree, so nothing outside the agent's own tools is allowed",
        )?;
    if tool.starts_with("mcp__mars__") || HARMLESS.contains(&tool) {
        return Ok(Verdict::Allow);
    }
    match tool {
        "Bash" | "PowerShell" => {
            let command = input
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if command.trim().is_empty() {
                return Ok(Verdict::Deny("an empty command was requested".into()));
            }
            if classify_command(command).class == CommandClass::ReadOnly {
                return Ok(Verdict::Allow);
            }
            Ok(Verdict::Ask(SideEffect::Shell {
                command: command.to_owned(),
                cwd: worktree,
            }))
        }
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => {
            let Some(given) = path_of(input) else {
                return Ok(Verdict::Deny("the edit named no file".into()));
            };
            let path = absolute(&worktree, given);
            let Some(relative) = inside(&path, &worktree) else {
                return Ok(Verdict::Deny(format!(
                    "{} is outside this task's worktree ({}); this task may only change files inside it",
                    path.display(),
                    worktree.display()
                )));
            };
            if relative == ".git" || relative.starts_with(".git/") {
                return Ok(Verdict::Deny(
                    "Git's own directory is not edited by hand; use git commands".into(),
                ));
            }
            // Inside the declared scope is what the plan was approved for —
            // unless the ceiling forbids writing at all, which the broker
            // records as a refusal by construction.
            if context.ceiling().write_files
                && context
                    .config
                    .scope
                    .iter()
                    .any(|glob| mars_security::glob_match(glob, &relative))
            {
                return Ok(Verdict::Allow);
            }
            Ok(Verdict::Ask(SideEffect::FileWrite {
                path,
                declared_scope: context.config.scope.clone(),
            }))
        }
        "Read" | "Glob" | "Grep" | "LS" | "NotebookRead" => {
            let Some(given) = path_of(input) else {
                return Ok(Verdict::Allow);
            };
            let path = absolute(&worktree, given);
            let readable = inside(&path, &worktree).is_some()
                || context
                    .config
                    .readable
                    .iter()
                    .any(|directory| inside(&path, directory).is_some());
            if readable {
                Ok(Verdict::Allow)
            } else {
                Ok(Verdict::Deny(format!(
                    "{} is outside this task's worktree. Ask the person to attach it as context if you need it.",
                    path.display()
                )))
            }
        }
        "WebFetch" => {
            let url = input.get("url").and_then(Value::as_str).unwrap_or_default();
            Ok(Verdict::Ask(SideEffect::Network {
                host: host_of(url),
                detail: url.to_owned(),
            }))
        }
        "WebSearch" => Ok(Verdict::Ask(SideEffect::Network {
            host: "web search".into(),
            detail: input
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        })),
        other => Ok(Verdict::Ask(SideEffect::Tool {
            name: other.to_owned(),
            detail: summarize(input),
            cwd: worktree,
        })),
    }
}

fn ask(
    context: &ToolContext,
    effect: SideEffect,
    input: &Value,
    cancelled: &AtomicBool,
) -> ToolOutcome {
    let Some(task_id) = context.config.task_id.clone() else {
        return deny("this session belongs to no task, so there is nobody to ask");
    };
    let request = BrokerRequest {
        task_id,
        run_id: context.config.run_id.clone(),
        agent_label: context.config.agent.clone(),
        ceiling: context.ceiling(),
        effect,
    };
    let outcome = match context.broker.request(&request) {
        Ok((outcome, _ticket)) => outcome,
        Err(error) => return deny(&format!("the request could not be recorded: {error}")),
    };
    let request_id = match outcome {
        BrokerOutcome::Allowed { .. } => return allow(input),
        BrokerOutcome::Denied { reason, .. } => return deny(&reason),
        BrokerOutcome::Pending { request_id } => request_id,
    };
    // The person answers in Mars, in another process. Their answer reaches
    // this one through the database.
    loop {
        if cancelled.load(Ordering::SeqCst) {
            let _ = context
                .broker
                .withdraw(&request_id, "the agent stopped waiting for an answer");
            return deny("the request was withdrawn because the agent stopped waiting");
        }
        match context.broker.outcome(&request_id) {
            Ok(None) => std::thread::sleep(POLL),
            Ok(Some(BrokerOutcome::Allowed { .. })) => return allow(input),
            Ok(Some(BrokerOutcome::Denied { reason, .. })) => {
                return deny(&format!(
                    "{reason}. Do not retry this or work around it; continue another way or explain what you need."
                ));
            }
            Ok(Some(BrokerOutcome::Pending { .. })) => std::thread::sleep(POLL),
            Err(error) => return deny(&format!("the request could not be read back: {error}")),
        }
    }
}

fn allow(input: &Value) -> ToolOutcome {
    ToolOutcome::ok(json!({ "behavior": "allow", "updatedInput": input }).to_string())
}

fn deny(message: &str) -> ToolOutcome {
    ToolOutcome::ok(json!({ "behavior": "deny", "message": message }).to_string())
}

fn path_of(input: &Value) -> Option<&str> {
    ["file_path", "notebook_path", "path"]
        .iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))
        .filter(|path| !path.trim().is_empty())
}

/// `given` as an absolute path, with `.` and `..` resolved lexically so a
/// relative escape such as `../../etc` is seen for what it is.
fn absolute(worktree: &Path, given: &str) -> PathBuf {
    let candidate = Path::new(given);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        worktree.join(candidate)
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// `path` relative to `root` with forward slashes, or `None` when it is not
/// inside it. Case-insensitive on Windows, where the file system is.
fn inside(path: &Path, root: &Path) -> Option<String> {
    let normalize = |value: &Path| {
        let text = dunce_like(value).replace('\\', "/");
        let text = text.trim_end_matches('/').to_owned();
        // ASCII folding keeps byte offsets, so the relative path can be cut
        // from the original spelling at the same index.
        if cfg!(windows) {
            text.to_ascii_lowercase()
        } else {
            text
        }
    };
    let path_text = normalize(path);
    let root_text = normalize(root);
    let original = dunce_like(path).replace('\\', "/");
    if path_text == root_text {
        return Some(String::new());
    }
    let prefix = format!("{root_text}/");
    path_text
        .starts_with(&prefix)
        .then(|| original.trim_end_matches('/')[prefix.len()..].to_owned())
}

/// The path without Windows' verbatim `\\?\` prefix, so a root stored one way
/// and a path reported the other still compare.
fn dunce_like(path: &Path) -> String {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\")
        .filter(|rest| !rest.starts_with("UNC"))
        .map_or_else(|| text.to_string(), str::to_owned)
}

fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
    host.split(':').next().unwrap_or(host).to_ascii_lowercase()
}

fn summarize(input: &Value) -> String {
    let text = input.to_string();
    if text.chars().count() <= 120 {
        text
    } else {
        let mut short: String = text.chars().take(119).collect();
        short.push('…');
        short
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_read_from_urls() {
        assert_eq!(host_of("https://Docs.RS/serde/latest"), "docs.rs");
        assert_eq!(host_of("http://user:pw@example.com:8080/x"), "example.com");
        assert_eq!(host_of("example.org/path"), "example.org");
    }

    #[test]
    fn relative_escapes_are_resolved_before_containment() {
        let root = if cfg!(windows) {
            PathBuf::from(r"C:\work\task")
        } else {
            PathBuf::from("/work/task")
        };
        let inside_path = absolute(&root, "src/../src/app.ts");
        assert_eq!(inside(&inside_path, &root).as_deref(), Some("src/app.ts"));
        let escape = absolute(&root, "../other/secrets.txt");
        assert_eq!(inside(&escape, &root), None);
        // A sibling that shares a prefix is not inside.
        let sibling = absolute(&root, "../task-two/file");
        assert_eq!(inside(&sibling, &root), None);
    }
}
