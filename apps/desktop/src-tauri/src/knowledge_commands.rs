//! What agents decided, across every task in the project, and the person's
//! answer to each.
//!
//! A decision reaches the knowledge store two ways: an agent records it while
//! it works (the `record_decision` MCP tool), or extraction reads it out of a
//! finished session. Either way it is a candidate — nothing an agent is later
//! given as fact — until a person approves it here.

use mars_brain::{
    EntityKind, EntityRef, KnowledgeOrigin, KnowledgeRecord, KnowledgeStatus, NewProvenance,
    ProvenanceKind,
};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{AppState, CommandFailure};

/// One record, with where it came from.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeItem {
    pub record: KnowledgeRecord,
    pub task_title: Option<String>,
    #[serde(flatten)]
    pub origin: KnowledgeOrigin,
}

fn describe(state: &AppState, record: KnowledgeRecord) -> Result<KnowledgeItem, CommandFailure> {
    let origin = state.knowledge.origin(&record)?;
    let task_title = origin
        .task_id
        .as_deref()
        .and_then(|task_id| state.tasks.get(task_id).ok())
        .map(|task| task.title);
    Ok(KnowledgeItem {
        record,
        task_title,
        origin,
    })
}

/// Everything recorded in the project — candidates first, so what is waiting
/// on a person is at the top — optionally with what was already resolved.
#[tauri::command]
pub async fn project_knowledge(
    project_id: String,
    include_resolved: bool,
    state: State<'_, AppState>,
) -> Result<Vec<KnowledgeItem>, CommandFailure> {
    let mut items = state
        .knowledge
        .list(&project_id, include_resolved)?
        .into_iter()
        .map(|record| describe(&state, record))
        .collect::<Result<Vec<_>, _>>()?;
    items.sort_by(|left, right| {
        let settled = |item: &KnowledgeItem| item.record.status != KnowledgeStatus::Candidate;
        settled(left)
            .cmp(&settled(right))
            .then_with(|| right.record.created_at.cmp(&left.record.created_at))
    });
    Ok(items)
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeAction {
    Approve,
    Reject,
    /// Approve a candidate in the person's wording, or supersede an approved
    /// record with it.
    Correct,
    MarkStale,
    Pin,
    Unpin,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeResolution {
    pub record_id: String,
    pub action: KnowledgeAction,
    pub title: Option<String>,
    pub body: Option<String>,
}

/// A person's answer to one record. The answer is itself recorded as human
/// provenance, which is the only kind the knowledge store accepts as a
/// resolution.
#[tauri::command]
pub async fn resolve_knowledge(
    request: KnowledgeResolution,
    state: State<'_, AppState>,
) -> Result<KnowledgeItem, CommandFailure> {
    let record = state.knowledge.get(&request.record_id)?;
    let observed_at: String = state
        .storage
        .with_connection(|connection| {
            connection.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
                row.get(0)
            })
        })
        .map_err(|error| CommandFailure::new("knowledge.clock", error.to_string()))?;
    let human = state.knowledge.create_provenance(NewProvenance {
        project_id: record.project_id.clone(),
        kind: ProvenanceKind::Human,
        source: EntityRef::new(EntityKind::Knowledge, &record.id),
        evidence: serde_json::json!({
            "action": format!("{:?}", request.action),
            "visibleReview": true,
            "by": crate::local_actor(),
        }),
        observed_at,
    })?;
    let resolved = match request.action {
        KnowledgeAction::Approve => state.knowledge.approve(&record.id, &human.id)?,
        KnowledgeAction::Reject => state.knowledge.reject(&record.id, &human.id)?,
        KnowledgeAction::Correct => {
            let title = request.title.as_deref().unwrap_or(&record.title);
            let body = request.body.as_deref().unwrap_or(&record.body);
            if record.status == KnowledgeStatus::Candidate {
                // Accepted as reworded: approved, then superseded by the
                // person's version, so the history shows both.
                state.knowledge.approve(&record.id, &human.id)?;
            }
            state
                .knowledge
                .correct(&record.id, title, body, &human.id)?
        }
        KnowledgeAction::MarkStale => state.knowledge.mark_stale(&record.id, &human.id)?,
        KnowledgeAction::Pin => state.knowledge.set_pinned(&record.id, true, &human.id)?,
        KnowledgeAction::Unpin => state.knowledge.set_pinned(&record.id, false, &human.id)?,
    };
    describe(&state, resolved)
}
