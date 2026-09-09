//! A task's record as a document a person can read.
//!
//! [`export`](crate::record::ActivityRecord::export) already writes the log as
//! newline-delimited JSON, which is the right shape for another program and the
//! wrong one for a human: what actually happened is spread across a hundred
//! objects, in the order they were written rather than the order anyone would
//! want to read them, with payloads inlined as escaped strings.
//!
//! This produces the same evidence as prose. Two constraints shape it:
//!
//! * **The log is the only source.** Nothing here queries tasks, worktrees,
//!   reviews, memory or the forge. If a fact is not in the record it does not
//!   appear, and the document says which facts those are rather than filling
//!   them in from a table that could disagree with the transcript.
//! * **Nothing is dropped silently.** A payload too large to inline is cut at a
//!   stated byte count with the cut named on the page, and an event of a kind
//!   this module has no special rendering for still gets a line. An audit that
//!   quietly omits things is worse than no audit.

use std::fmt::Write as _;

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::ActivityError;
use crate::record::{ActivityEvent, ActivityKind, Actor, ActivityRecord};
use crate::replay::project;

/// How much of one payload the document carries before saying it cut it.
///
/// Generous, because the point of an audit is to be sufficient on its own: a
/// test log or a diff that has to be fetched from somewhere else to be read is
/// not evidence, it is a pointer to evidence.
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;

/// How many events one export walks before refusing.
///
/// A ceiling rather than a page: an audit is a whole document or it is not an
/// audit, so this is the point at which the export fails loudly instead of
/// producing something that looks complete and is not.
pub const MAX_EVENTS: usize = 50_000;

fn field(payload: &serde_json::Value, name: &str) -> Option<String> {
    payload.get(name).and_then(|value| match value {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    })
}

/// A fence long enough that the content cannot close it early.
fn fenced(out: &mut String, language: &str, body: &str) {
    let longest = body
        .lines()
        .filter(|line| line.trim_start().starts_with("```"))
        .map(|line| line.trim_start().chars().take_while(|c| *c == '`').count())
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    let _ = writeln!(out, "{fence}{language}");
    out.push_str(body);
    if !body.ends_with('\n') {
        out.push('\n');
    }
    let _ = writeln!(out, "{fence}");
}

/// The payload as it was written, whether or not it fitted on the row.
///
/// A spilled event keeps only `{spilled, digest, byteSize}` inline — the
/// command, its status and its exit code all move into the blob. An audit that
/// read only the row would report a large `cargo test` with no command and no
/// exit code, which is precisely the run whose evidence matters most.
struct Resolved {
    payload: serde_json::Value,
    /// The evidence to print: the spilled bytes, or the inline output field.
    body: Option<String>,
    digest: Option<String>,
}

fn actor_label(event: &ActivityEvent) -> String {
    match event.actor {
        Actor::Human => "You".to_owned(),
        _ => event.actor_label.clone(),
    }
}

impl ActivityRecord {
    /// The whole of a task's record as Markdown, from the log alone.
    ///
    /// `generated_at` is passed in rather than read from the clock so the same
    /// log always produces the same document — an audit that differs between
    /// two exports of unchanged evidence cannot be compared against itself.
    pub fn audit_markdown(
        &self,
        task_id: &str,
        generated_at: &str,
    ) -> Result<String, ActivityError> {
        let mut events: Vec<ActivityEvent> = Vec::new();
        let mut after = 0;
        loop {
            let page = self.read(task_id, after, 1_000)?;
            if page.is_empty() {
                break;
            }
            after = page[page.len() - 1].sequence;
            events.extend(page);
            if events.len() > MAX_EVENTS {
                return Err(ActivityError::TooLargeToRender(format!(
                    "task {task_id} has more than {MAX_EVENTS} events; export it as JSONL instead \
                     of a document"
                )));
            }
        }

        let projection = project(task_id, &events);
        let mut out = String::new();

        let _ = writeln!(out, "# Activity record for task {task_id}");
        out.push('\n');
        let _ = writeln!(
            out,
            "Exported {generated_at}. Every line below is derived from the append-only activity \
             log and nothing else — no status column, no cached summary. Anything the log does not \
             record is absent here rather than filled in from elsewhere."
        );
        out.push('\n');

        // ---- What the log says the shape of the work was.
        let _ = writeln!(out, "## Summary");
        out.push('\n');
        let _ = writeln!(out, "- Events: {}", projection.event_count);
        let _ = writeln!(out, "- Last sequence: {}", projection.last_sequence);
        if let Some(first) = events.first() {
            let _ = writeln!(out, "- First event: {}", first.occurred_at);
        }
        if let Some(last) = events.last() {
            let _ = writeln!(out, "- Last event: {}", last.occurred_at);
        }
        match &projection.worktree {
            Some(worktree) => {
                let _ = writeln!(
                    out,
                    "- Worktree: {} on branch {} from {} ({})",
                    worktree.path.as_deref().unwrap_or("(path not recorded)"),
                    worktree.branch_name.as_deref().unwrap_or("(unnamed)"),
                    worktree.base_ref.as_deref().unwrap_or("(base not recorded)"),
                    worktree.state,
                );
                if let Some(oid) = &worktree.base_oid {
                    let _ = writeln!(out, "- Base commit: `{oid}`");
                }
            }
            None => {
                let _ = writeln!(
                    out,
                    "- Worktree: the log records none, so no branch was ever provisioned for this \
                     task."
                );
            }
        }
        let _ = writeln!(out, "- Runs: {}", projection.runs.len());
        let _ = writeln!(out, "- Commands: {}", projection.commands.len());
        let _ = writeln!(out, "- Files touched: {}", projection.files_touched.len());
        let _ = writeln!(out, "- Tests recorded: {}", projection.tests.len());
        let _ = writeln!(
            out,
            "- Permission decisions: {} ({} still unanswered)",
            projection.decided_permissions.len(),
            projection.pending_permissions.len(),
        );
        out.push('\n');

        // ---- Decisions, gathered, because this is what an audit is usually for.
        let _ = writeln!(out, "## Decisions");
        out.push('\n');
        if projection.decided_permissions.is_empty() && projection.pending_permissions.is_empty() {
            let _ = writeln!(out, "No permission was ever requested for this task.");
        } else {
            let _ = writeln!(out, "| Sequence | Request | Decision | By | When |");
            let _ = writeln!(out, "| --- | --- | --- | --- | --- |");
            for decision in &projection.decided_permissions {
                let _ = writeln!(
                    out,
                    "| {} | `{}` | {} | {} | {} |",
                    decision.sequence,
                    decision.request_id,
                    decision.decision,
                    decision.decided_by,
                    decision.decided_at,
                );
            }
            for pending in &projection.pending_permissions {
                // Named rather than omitted: a request nobody answered is a
                // fact about the run, and often the most important one.
                let _ = writeln!(
                    out,
                    "| {} | `{}` | **never answered** | — | {} |",
                    pending.sequence, pending.request_id, pending.requested_at,
                );
            }
        }
        out.push('\n');

        if !projection.files_touched.is_empty() {
            let _ = writeln!(out, "## Files the log says were written");
            out.push('\n');
            for file in &projection.files_touched {
                let _ = writeln!(
                    out,
                    "- `{}` — {} ({} edit(s), last at sequence {})",
                    file.path, file.change, file.edits, file.last_sequence,
                );
            }
            out.push('\n');
        }

        // ---- The record itself, in order, with its evidence inline.
        let _ = writeln!(out, "## Transcript");
        out.push('\n');
        if events.is_empty() {
            let _ = writeln!(out, "The log for this task is empty.");
            return Ok(out);
        }

        for event in &events {
            self.write_event(&mut out, event)?;
        }

        Ok(out)
    }

    fn write_event(&self, out: &mut String, event: &ActivityEvent) -> Result<(), ActivityError> {
        let sequence = event.sequence;
        let when = &event.occurred_at;
        // Read through the spill: a large command's own name and exit code live
        // in the blob, not on the row.
        let resolved = self.resolve(event)?;
        let payload = &resolved.payload;

        match event.kind {
            ActivityKind::Message => {
                let _ = writeln!(out, "### {} · {} · seq {}", actor_label(event), when, sequence);
                out.push('\n');
                let text = field(payload, "text").unwrap_or_else(|| event.summary.clone());
                for line in text.lines() {
                    let _ = writeln!(out, "> {line}");
                }
                out.push('\n');
            }
            ActivityKind::Command => {
                let command =
                    field(payload, "command").unwrap_or_else(|| event.summary.clone());
                let status = field(payload, "status").unwrap_or_else(|| "—".into());
                let exit = payload
                    .get("exitCode")
                    .and_then(serde_json::Value::as_i64);
                let _ = writeln!(out, "### Command · {when} · seq {sequence}");
                out.push('\n');
                fenced(out, "sh", &command);
                let _ = writeln!(
                    out,
                    "Status: {status}{}{}",
                    exit.map(|code| format!(" · exit {code}")).unwrap_or_default(),
                    event
                        .duration_ms
                        .map(|ms| format!(" · {ms}ms"))
                        .unwrap_or_default(),
                );
                out.push('\n');
                self.write_payload(out, &resolved, "Output", "");
            }
            ActivityKind::FileEdit => {
                let path = field(payload, "path").unwrap_or_else(|| event.summary.clone());
                let change = field(payload, "change").unwrap_or_else(|| "changed".into());
                let added = payload.get("added").and_then(serde_json::Value::as_i64);
                let removed = payload.get("removed").and_then(serde_json::Value::as_i64);
                let _ = writeln!(out, "### File · {when} · seq {sequence}");
                out.push('\n');
                let _ = writeln!(
                    out,
                    "`{path}` — {change}{}",
                    match (added, removed) {
                        (Some(a), Some(r)) => format!(" (+{a} −{r})"),
                        _ => String::new(),
                    },
                );
                out.push('\n');
                self.write_payload(out, &resolved, "Diff", "diff");
            }
            ActivityKind::TestRun => {
                let command =
                    field(payload, "command").unwrap_or_else(|| event.summary.clone());
                let status = field(payload, "status").unwrap_or_else(|| "—".into());
                let _ = writeln!(out, "### Test · {when} · seq {sequence}");
                out.push('\n');
                let _ = writeln!(out, "`{command}` — {status}");
                out.push('\n');
                self.write_payload(out, &resolved, "Result", "");
            }
            ActivityKind::PermissionRequest => {
                let _ = writeln!(out, "### Permission requested · {when} · seq {sequence}");
                out.push('\n');
                let _ = writeln!(out, "{}", event.summary);
                if let Some(id) = field(payload, "requestId") {
                    let _ = writeln!(out, "\nRequest `{id}`.");
                }
                if let Some(classification) = field(payload, "classification") {
                    let _ = writeln!(out, "\nClassified as {classification}.");
                }
                out.push('\n');
            }
            ActivityKind::PermissionDecision => {
                let decision = field(payload, "decision").unwrap_or_else(|| "—".into());
                let _ = writeln!(out, "### Permission {decision} · {when} · seq {sequence}");
                out.push('\n');
                let _ = writeln!(
                    out,
                    "{} — decided by {}.",
                    event.summary,
                    field(payload, "decidedBy").unwrap_or_else(|| actor_label(event)),
                );
                out.push('\n');
            }
            ActivityKind::Error => {
                let _ = writeln!(out, "### Error · {when} · seq {sequence}");
                out.push('\n');
                let _ = writeln!(out, "{}", event.summary);
                out.push('\n');
                self.write_payload(out, &resolved, "Detail", "");
            }
            _ => {
                // No special rendering, but still a line. An audit that shows
                // only the kinds someone thought to format is not a record.
                let _ = writeln!(
                    out,
                    "### {} · {} · seq {}",
                    event.kind.as_str(),
                    when,
                    sequence
                );
                out.push('\n');
                let _ = writeln!(out, "{}", event.summary);
                out.push('\n');
                self.write_payload(out, &resolved, "Detail", "");
            }
        }

        Ok(())
    }

    /// The same document, stamped with the current time.
    ///
    /// Separate from [`audit_markdown`](Self::audit_markdown) so the renderer
    /// stays a pure function of the log: a test can compare two exports of
    /// unchanged evidence and expect them to be identical.
    pub fn audit_markdown_now(&self, task_id: &str) -> Result<String, ActivityError> {
        let generated_at = OffsetDateTime::now_utc().format(&Rfc3339)?;
        self.audit_markdown(task_id, &generated_at)
    }

    /// Read back whatever the row could not hold.
    fn resolve(&self, event: &ActivityEvent) -> Result<Resolved, ActivityError> {
        let spilled = event
            .payload
            .get("spilled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        let Some(digest) = event.payload_digest.as_deref() else {
            let body = field(&event.payload, "output").or_else(|| field(&event.payload, "detail"));
            return Ok(Resolved {
                payload: event.payload.clone(),
                body,
                digest: None,
            });
        };

        let bytes = self.payload_bytes(digest)?;
        let text = String::from_utf8_lossy(&bytes).into_owned();

        if !spilled {
            // An attachment: the row's own payload is intact, and the stored
            // bytes are the thing that was attached.
            let body = field(&event.payload, "output")
                .or_else(|| field(&event.payload, "detail"))
                .or(Some(text));
            return Ok(Resolved {
                payload: event.payload.clone(),
                body,
                digest: Some(digest.to_owned()),
            });
        }

        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(payload) => {
                let body = field(&payload, "output")
                    .or_else(|| field(&payload, "detail"))
                    // A spilled payload with no named evidence field still gets
                    // printed; showing the object beats showing nothing.
                    .or_else(|| serde_json::to_string_pretty(&payload).ok());
                Ok(Resolved {
                    payload,
                    body,
                    digest: Some(digest.to_owned()),
                })
            }
            // Not JSON after all. The bytes are the evidence.
            Err(_) => Ok(Resolved {
                payload: event.payload.clone(),
                body: Some(text),
                digest: Some(digest.to_owned()),
            }),
        }
    }

    /// Print an event's evidence, whole or explicitly cut.
    fn write_payload(&self, out: &mut String, resolved: &Resolved, label: &str, language: &str) {
        let Some(body) = resolved.body.as_deref() else {
            return;
        };

        match &resolved.digest {
            Some(digest) => {
                let _ = writeln!(out, "{label} (payload `{}`):", &digest[..12.min(digest.len())]);
            }
            None => {
                let _ = writeln!(out, "{label}:");
            }
        }
        out.push('\n');

        if body.len() > MAX_PAYLOAD_BYTES {
            // Cut on a character boundary, and say so on the page. A silent
            // truncation in an audit is a lie of omission.
            let mut end = MAX_PAYLOAD_BYTES;
            while end > 0 && !body.is_char_boundary(end) {
                end -= 1;
            }
            fenced(out, language, &body[..end]);
            let _ = writeln!(
                out,
                "\n*Cut after {MAX_PAYLOAD_BYTES} bytes of {}. The whole payload is in the \
                 record under its digest.*",
                body.len()
            );
        } else {
            fenced(out, language, body);
        }
        out.push('\n');
    }
}
