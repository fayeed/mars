use std::sync::Arc;

use mars_storage::{Storage, StorageError};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{ContextPack, ContextPackError, ContextPackRequest, ContextPackService};

const MIN_HANDOFF_BYTES: usize = 4 * 1024;
const MAX_HANDOFF_BYTES: usize = 512 * 1024;
const MAX_HANDOFF_EVENTS: usize = 200;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewHandoffRequest {
    pub task_id: String,
    pub implementation_session_id: String,
    pub reviewer_adapter: String,
    pub diff: String,
    pub max_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffEvent {
    pub sequence: i64,
    pub event_type: String,
    pub summary: String,
    pub source_location: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewHandoff {
    pub task_id: String,
    pub implementation_session_id: String,
    pub implementation_adapter: String,
    pub reviewer_adapter: String,
    pub context_pack: ContextPack,
    pub implementation_events: Vec<HandoffEvent>,
    pub diff: String,
    pub diff_sha256: String,
    pub max_bytes: usize,
    pub total_bytes: usize,
}

#[derive(Debug, Error)]
pub enum ReviewError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    ContextPack(#[from] ContextPackError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("review handoff budget must be between 4096 and 524288 bytes")]
    InvalidBudget,
    #[error("implementation session `{0}` is not a completed session for this task")]
    SessionNotReady(String),
    #[error("review adapter must be `claude` or `codex`")]
    InvalidReviewer,
    #[error("cross-agent review requires a reviewer different from the implementation adapter")]
    SameAdapter,
    #[error("exact diff cannot fit inside the review handoff budget")]
    ExactDiffTooLarge,
}

#[derive(Clone)]
pub struct ReviewService {
    storage: Arc<Storage>,
}

impl ReviewService {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }

    pub fn build_handoff(
        &self,
        request: ReviewHandoffRequest,
    ) -> Result<ReviewHandoff, ReviewError> {
        if !(MIN_HANDOFF_BYTES..=MAX_HANDOFF_BYTES).contains(&request.max_bytes) {
            return Err(ReviewError::InvalidBudget);
        }
        if !matches!(request.reviewer_adapter.as_str(), "claude" | "codex") {
            return Err(ReviewError::InvalidReviewer);
        }
        let implementation_adapter: Option<String> =
            self.storage.with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT adapter_kind FROM agent_sessions
                     WHERE id = ?1 AND task_id = ?2 AND status = 'completed'",
                        params![request.implementation_session_id, request.task_id],
                        |row| row.get(0),
                    )
                    .optional()
            })?;
        let Some(implementation_adapter) = implementation_adapter else {
            return Err(ReviewError::SessionNotReady(
                request.implementation_session_id,
            ));
        };
        if implementation_adapter == request.reviewer_adapter {
            return Err(ReviewError::SameAdapter);
        }
        if request.diff.trim().is_empty() || request.diff.len() + 2_048 >= request.max_bytes {
            return Err(ReviewError::ExactDiffTooLarge);
        }
        let context_budget = (request.max_bytes - request.diff.len() - 2_048).clamp(512, 24 * 1024);
        let context_pack =
            ContextPackService::new(self.storage.clone()).build(ContextPackRequest {
                task_id: request.task_id.clone(),
                adapter: request.reviewer_adapter.clone(),
                max_bytes: context_budget,
                excluded_entity_refs: Vec::new(),
            })?;
        let raw_events: Vec<(i64, String, String)> =
            self.storage.with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT sequence, event_type, payload_json FROM normalized_agent_events
                 WHERE session_id = ?1
                   AND event_type IN ('text', 'command', 'file_change', 'test', 'end')
                 ORDER BY sequence DESC LIMIT ?2",
                )?;
                statement
                    .query_map(
                        params![request.implementation_session_id, MAX_HANDOFF_EVENTS as i64],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )?
                    .collect()
            })?;
        let mut events = raw_events
            .into_iter()
            .rev()
            .filter_map(|(sequence, event_type, payload)| {
                let value = serde_json::from_str::<Value>(&payload).ok()?;
                Some(HandoffEvent {
                    sequence,
                    summary: summarize_event(&event_type, &value),
                    source_location: format!(
                        "agent_session:{}:normalized:{sequence}",
                        request.implementation_session_id
                    ),
                    event_type,
                })
            })
            .collect::<Vec<_>>();
        let diff_sha256 = format!("{:x}", Sha256::digest(request.diff.as_bytes()));
        let mut handoff = ReviewHandoff {
            task_id: request.task_id,
            implementation_session_id: request.implementation_session_id,
            implementation_adapter,
            reviewer_adapter: request.reviewer_adapter,
            context_pack,
            implementation_events: events.clone(),
            diff: request.diff,
            diff_sha256,
            max_bytes: request.max_bytes,
            total_bytes: 0,
        };
        loop {
            refresh_total(&mut handoff)?;
            if handoff.total_bytes <= handoff.max_bytes {
                break;
            }
            if events.is_empty() {
                return Err(ReviewError::ExactDiffTooLarge);
            }
            events.remove(0);
            handoff.implementation_events.clone_from(&events);
        }
        Ok(handoff)
    }
}

fn serialized_size(handoff: &ReviewHandoff) -> Result<usize, serde_json::Error> {
    Ok(serde_json::to_vec(handoff)?.len())
}

fn refresh_total(handoff: &mut ReviewHandoff) -> Result<(), serde_json::Error> {
    for _ in 0..4 {
        let size = serialized_size(handoff)?;
        if size == handoff.total_bytes {
            return Ok(());
        }
        handoff.total_bytes = size;
    }
    Ok(())
}

fn summarize_event(event_type: &str, value: &Value) -> String {
    let summary = match event_type {
        "text" => value.get("text").and_then(Value::as_str).unwrap_or(""),
        "command" | "test" => value
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or(event_type),
        "file_change" => value
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("file changed"),
        "end" => value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("session ended"),
        _ => "event",
    };
    truncate_utf8(summary.trim(), 2 * 1024)
}

fn truncate_utf8(value: &str, maximum: usize) -> String {
    if value.len() <= maximum {
        return value.into();
    }
    let mut end = maximum;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}
