//! The activity record: an append-only, per-task event log.
//!
//! Everything the execution core does becomes an event here — agent output,
//! commands, file edits, test runs, permission requests and the decisions on
//! them, worktree lifecycle steps. The log is the single source of truth the UI
//! renders from, so it carries enough to reconstruct a task without consulting
//! any other table: a monotonic sequence, wall-clock time, an optional duration,
//! and either an inline payload or the digest of a content-addressed one.

use std::sync::Arc;
use std::time::Duration;

use mars_storage::Storage;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::ActivityError;
use crate::payloads::PayloadStore;

/// Payloads larger than this are spilled to the content-addressed store instead
/// of being inlined on the event row.
const INLINE_PAYLOAD_LIMIT: usize = 8 * 1024;

/// The normalized event vocabulary. Adapter-specific shapes are mapped onto
/// these by the agent layer; nothing downstream sees provider types.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    Message,
    Thought,
    ToolCall,
    Command,
    FileEdit,
    TestRun,
    PermissionRequest,
    PermissionDecision,
    Error,
    Done,
    RunStarted,
    WorktreePlanned,
    WorktreeCreated,
    WorktreeRemoved,
    WorktreeRestored,
    RecoveryNoted,
}

impl ActivityKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Thought => "thought",
            Self::ToolCall => "tool_call",
            Self::Command => "command",
            Self::FileEdit => "file_edit",
            Self::TestRun => "test_run",
            Self::PermissionRequest => "permission_request",
            Self::PermissionDecision => "permission_decision",
            Self::Error => "error",
            Self::Done => "done",
            Self::RunStarted => "run_started",
            Self::WorktreePlanned => "worktree_planned",
            Self::WorktreeCreated => "worktree_created",
            Self::WorktreeRemoved => "worktree_removed",
            Self::WorktreeRestored => "worktree_restored",
            Self::RecoveryNoted => "recovery_noted",
        }
    }

    fn parse(value: &str) -> Result<Self, ActivityError> {
        match value {
            "message" => Ok(Self::Message),
            "thought" => Ok(Self::Thought),
            "tool_call" => Ok(Self::ToolCall),
            "command" => Ok(Self::Command),
            "file_edit" => Ok(Self::FileEdit),
            "test_run" => Ok(Self::TestRun),
            "permission_request" => Ok(Self::PermissionRequest),
            "permission_decision" => Ok(Self::PermissionDecision),
            "error" => Ok(Self::Error),
            "done" => Ok(Self::Done),
            "run_started" => Ok(Self::RunStarted),
            "worktree_planned" => Ok(Self::WorktreePlanned),
            "worktree_created" => Ok(Self::WorktreeCreated),
            "worktree_removed" => Ok(Self::WorktreeRemoved),
            "worktree_restored" => Ok(Self::WorktreeRestored),
            "recovery_noted" => Ok(Self::RecoveryNoted),
            other => Err(ActivityError::InvalidStoredKind(other.to_owned())),
        }
    }
}

/// Who caused an event. The distinction matters for the transcript: a decision
/// recorded against `Human` names a person, one against `System` does not.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    Human,
    Agent,
    System,
}

impl Actor {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
            Self::System => "system",
        }
    }

    fn parse(value: &str) -> Result<Self, ActivityError> {
        match value {
            "human" => Ok(Self::Human),
            "agent" => Ok(Self::Agent),
            "system" => Ok(Self::System),
            other => Err(ActivityError::InvalidStoredActor(other.to_owned())),
        }
    }
}

/// Bytes to keep out of the event row and address by digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attachment {
    pub media_type: String,
    pub bytes: Vec<u8>,
}

impl Attachment {
    #[must_use]
    pub fn text(media_type: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            media_type: media_type.into(),
            bytes: text.into().into_bytes(),
        }
    }
}

/// What a writer supplies. The sequence, id and timestamp are assigned by the
/// record so no caller can choose its own ordering.
#[derive(Clone, Debug)]
pub struct ActivityEntry {
    pub task_id: String,
    pub run_id: Option<String>,
    pub kind: ActivityKind,
    pub actor: Actor,
    pub actor_label: String,
    pub summary: String,
    pub payload: Value,
    pub attachment: Option<Attachment>,
    pub duration: Option<Duration>,
}

impl ActivityEntry {
    #[must_use]
    pub fn new(
        task_id: impl Into<String>,
        kind: ActivityKind,
        actor: Actor,
        actor_label: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            run_id: None,
            kind,
            actor,
            actor_label: actor_label.into(),
            summary: summary.into(),
            payload: Value::Object(serde_json::Map::new()),
            attachment: None,
            duration: None,
        }
    }

    #[must_use]
    pub fn in_run(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    #[must_use]
    pub fn with_payload(mut self, payload: Value) -> Self {
        self.payload = payload;
        self
    }

    #[must_use]
    pub fn with_attachment(mut self, attachment: Attachment) -> Self {
        self.attachment = Some(attachment);
        self
    }

    #[must_use]
    pub fn lasting(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }
}

/// A durable event, exactly as the UI receives it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    pub id: String,
    pub task_id: String,
    pub sequence: i64,
    pub run_id: Option<String>,
    pub kind: ActivityKind,
    pub actor: Actor,
    pub actor_label: String,
    pub summary: String,
    pub payload: Value,
    pub payload_digest: Option<String>,
    pub occurred_at: String,
    pub duration_ms: Option<i64>,
}

/// A page of the record, with what sits before it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityWindow {
    pub events: Vec<ActivityEvent>,
    /// Events earlier than the first one here. Zero means this page reaches the
    /// beginning of the record.
    pub earlier: usize,
}

#[derive(Clone)]
pub struct ActivityRecord {
    storage: Arc<Storage>,
    payloads: PayloadStore,
}

impl ActivityRecord {
    #[must_use]
    pub fn new(storage: Arc<Storage>, payload_root: impl AsRef<std::path::Path>) -> Self {
        let payloads = PayloadStore::new(storage.clone(), payload_root);
        Self { storage, payloads }
    }

    #[must_use]
    pub fn payloads(&self) -> &PayloadStore {
        &self.payloads
    }

    /// Append one event, assigning it the next sequence for its task.
    pub fn append(&self, entry: ActivityEntry) -> Result<ActivityEvent, ActivityError> {
        let (payload, digest) = self.materialize_payload(&entry)?;
        let id = Uuid::now_v7().to_string();
        let occurred_at = OffsetDateTime::now_utc().format(&Rfc3339)?;
        let duration_ms = entry
            .duration
            .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX));
        let payload_text = serde_json::to_string(&payload)?;
        let scope_run = entry.run_id.clone();

        let sequence = self.storage.with_transaction(|transaction| {
            let next: i64 = transaction.query_row(
                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM activity_events WHERE task_id = ?1",
                [&entry.task_id],
                |row| row.get(0),
            )?;
            transaction.execute(
                "INSERT INTO activity_events
                     (task_id, sequence, id, run_id, kind, actor, actor_label, summary,
                      payload_json, payload_digest, occurred_at, duration_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    &entry.task_id,
                    next,
                    &id,
                    scope_run.as_deref(),
                    entry.kind.as_str(),
                    entry.actor.as_str(),
                    &entry.actor_label,
                    &entry.summary,
                    &payload_text,
                    digest.as_deref(),
                    &occurred_at,
                    duration_ms,
                ],
            )?;
            if let Some(run_id) = scope_run.as_deref() {
                transaction.execute(
                    "UPDATE agent_runs SET last_sequence = ?2 WHERE id = ?1",
                    params![run_id, next],
                )?;
            }
            Ok(next)
        })?;

        Ok(ActivityEvent {
            id,
            task_id: entry.task_id,
            sequence,
            run_id: entry.run_id,
            kind: entry.kind,
            actor: entry.actor,
            actor_label: entry.actor_label,
            summary: entry.summary,
            payload,
            payload_digest: digest,
            occurred_at,
            duration_ms,
        })
    }

    /// Read a page of the log in sequence order.
    pub fn read(
        &self,
        task_id: &str,
        after_sequence: i64,
        limit: usize,
    ) -> Result<Vec<ActivityEvent>, ActivityError> {
        let limit = i64::try_from(limit.clamp(1, 5_000)).unwrap_or(500);
        let rows = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT task_id, sequence, id, run_id, kind, actor, actor_label, summary,
                        payload_json, payload_digest, occurred_at, duration_ms
                 FROM activity_events
                 WHERE task_id = ?1 AND sequence > ?2
                 ORDER BY sequence
                 LIMIT ?3",
            )?;
            let mapped = statement.query_map(params![task_id, after_sequence, limit], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                ))
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()
        })?;

        rows.into_iter().map(decode_row).collect()
    }

    /// The newest page of the log, and how much of it is not in the page.
    ///
    /// `read` walks forward from the beginning, which is right for an export and
    /// wrong for a conversation: a task that outgrows one page would show its
    /// first thousand events forever and never its live tail. A reader arrives
    /// at the end of a transcript, so this starts there and works backwards,
    /// and `earlier` is what lets the screen say how much it is not showing
    /// instead of silently ending the record.
    ///
    /// `before_sequence` walks further back, one page at a time.
    pub fn read_window(
        &self,
        task_id: &str,
        before_sequence: Option<i64>,
        limit: usize,
    ) -> Result<ActivityWindow, ActivityError> {
        let limit = i64::try_from(limit.clamp(1, 5_000)).unwrap_or(500);
        let rows = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT task_id, sequence, id, run_id, kind, actor, actor_label, summary,
                        payload_json, payload_digest, occurred_at, duration_ms
                 FROM activity_events
                 WHERE task_id = ?1 AND (?2 IS NULL OR sequence < ?2)
                 ORDER BY sequence DESC
                 LIMIT ?3",
            )?;
            let mapped = statement.query_map(params![task_id, before_sequence, limit], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                ))
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()
        })?;

        let mut events: Vec<ActivityEvent> = rows
            .into_iter()
            .map(decode_row)
            .collect::<Result<Vec<_>, _>>()?;
        // Read newest-first for the LIMIT, rendered oldest-first for the reader.
        events.reverse();

        let earlier = match events.first() {
            None => 0,
            Some(first) => self.storage.with_connection(|connection| {
                connection.query_row(
                    "SELECT COUNT(*) FROM activity_events WHERE task_id = ?1 AND sequence < ?2",
                    params![task_id, first.sequence],
                    |row| row.get::<_, i64>(0),
                )
            })?,
        };

        Ok(ActivityWindow {
            events,
            earlier: usize::try_from(earlier).unwrap_or(0),
        })
    }

    /// The highest sequence written for a task, or zero when it has no log yet.
    pub fn last_sequence(&self, task_id: &str) -> Result<i64, ActivityError> {
        Ok(self.storage.with_connection(|connection| {
            connection.query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM activity_events WHERE task_id = ?1",
                [task_id],
                |row| row.get(0),
            )
        })?)
    }

    /// Resolve a spilled payload back to its bytes.
    pub fn payload_bytes(&self, digest: &str) -> Result<Vec<u8>, ActivityError> {
        self.payloads.get(digest)
    }

    /// The whole log for a task as newline-delimited JSON, one event per line,
    /// with spilled payloads resolved inline so an export is self-contained.
    pub fn export(&self, task_id: &str) -> Result<String, ActivityError> {
        let mut exported = String::new();
        let mut after = 0;
        loop {
            let page = self.read(task_id, after, 1_000)?;
            if page.is_empty() {
                break;
            }
            for event in &page {
                after = event.sequence;
                let mut line = serde_json::to_value(event)?;
                if let Some(digest) = event.payload_digest.as_deref() {
                    let bytes = self.payloads.get(digest)?;
                    let resolved = serde_json::from_slice::<Value>(&bytes).unwrap_or_else(|_| {
                        Value::String(String::from_utf8_lossy(&bytes).into_owned())
                    });
                    if let Some(object) = line.as_object_mut() {
                        object.insert("resolvedPayload".into(), resolved);
                    }
                }
                exported.push_str(&serde_json::to_string(&line)?);
                exported.push('\n');
            }
        }
        Ok(exported)
    }

    /// The tasks that have any activity, most recent first.
    pub fn tasks_with_activity(&self) -> Result<Vec<String>, ActivityError> {
        Ok(self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT task_id, MAX(sequence) AS last
                 FROM activity_events
                 GROUP BY task_id
                 ORDER BY last DESC",
            )?;
            let mapped = statement.query_map([], |row| row.get::<_, String>(0))?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()
        })?)
    }

    fn materialize_payload(
        &self,
        entry: &ActivityEntry,
    ) -> Result<(Value, Option<String>), ActivityError> {
        if let Some(attachment) = entry.attachment.as_ref() {
            let stored = self
                .payloads
                .put(&attachment.media_type, &attachment.bytes)?;
            let mut payload = entry.payload.clone();
            if let Some(object) = payload.as_object_mut() {
                object.insert(
                    "attachment".into(),
                    json!({
                        "digest": stored.digest,
                        "mediaType": stored.media_type,
                        "byteSize": stored.byte_size,
                    }),
                );
            }
            return Ok((payload, Some(stored.digest)));
        }

        let encoded = serde_json::to_vec(&entry.payload)?;
        if encoded.len() <= INLINE_PAYLOAD_LIMIT {
            return Ok((entry.payload.clone(), None));
        }
        let stored = self.payloads.put("application/json", &encoded)?;
        Ok((
            json!({
                "spilled": true,
                "digest": stored.digest,
                "byteSize": stored.byte_size,
            }),
            Some(stored.digest),
        ))
    }
}

type EventRow = (
    String,
    i64,
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    Option<i64>,
);

fn decode_row(row: EventRow) -> Result<ActivityEvent, ActivityError> {
    let (
        task_id,
        sequence,
        id,
        run_id,
        kind,
        actor,
        actor_label,
        summary,
        payload_json,
        payload_digest,
        occurred_at,
        duration_ms,
    ) = row;
    Ok(ActivityEvent {
        id,
        task_id,
        sequence,
        run_id,
        kind: ActivityKind::parse(&kind)?,
        actor: Actor::parse(&actor)?,
        actor_label,
        summary,
        payload: serde_json::from_str(&payload_json)?,
        payload_digest,
        occurred_at,
        duration_ms,
    })
}

/// Read one event by task and sequence, for callers that hold a reference.
pub fn event_at(
    storage: &Storage,
    task_id: &str,
    sequence: i64,
) -> Result<Option<ActivityEvent>, ActivityError> {
    let row = storage.with_connection(|connection| {
        connection
            .query_row(
                "SELECT task_id, sequence, id, run_id, kind, actor, actor_label, summary,
                        payload_json, payload_digest, occurred_at, duration_ms
                 FROM activity_events
                 WHERE task_id = ?1 AND sequence = ?2",
                params![task_id, sequence],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, Option<i64>>(11)?,
                    ))
                },
            )
            .optional()
    })?;
    row.map(decode_row).transpose()
}
