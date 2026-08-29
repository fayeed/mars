use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use mars_storage::{Storage, StorageError};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::knowledge::{
    EntityKind, EntityRef, KnowledgeError, KnowledgeKind, KnowledgeService, KnowledgeStatus,
};

const MAX_DEPTH: usize = 5;
const MAX_PATHS: usize = 20;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WhyStatus {
    Answered,
    AnsweredWithConflicts,
    InsufficientEvidence,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhyRequest {
    pub project_id: String,
    pub subject: EntityRef,
    pub max_depth: usize,
    pub max_paths: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhyFact {
    pub knowledge_id: String,
    pub kind: KnowledgeKind,
    pub title: String,
    pub body: String,
    pub confidence: f64,
    pub provenance: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhySource {
    pub label: String,
    pub location: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhyPathStep {
    pub kind: String,
    pub label: String,
    pub location: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhyPath {
    pub relation: String,
    pub steps: Vec<WhyPathStep>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhyAnswer {
    pub status: WhyStatus,
    pub subject: EntityRef,
    pub facts: Vec<WhyFact>,
    pub inferences: Vec<String>,
    pub warnings: Vec<String>,
    pub sources: Vec<WhySource>,
    pub paths: Vec<WhyPath>,
}

#[derive(Debug, Error)]
pub enum WhyError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Knowledge(#[from] KnowledgeError),
    #[error("why-query bounds are invalid: depth {depth}, paths {paths}")]
    InvalidBounds { depth: usize, paths: usize },
}

#[derive(Clone)]
pub struct WhyService {
    storage: Arc<Storage>,
}

impl WhyService {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }

    pub fn ask(&self, request: WhyRequest) -> Result<WhyAnswer, WhyError> {
        if !(1..=MAX_DEPTH).contains(&request.max_depth)
            || !(1..=MAX_PATHS).contains(&request.max_paths)
        {
            return Err(WhyError::InvalidBounds {
                depth: request.max_depth,
                paths: request.max_paths,
            });
        }
        let links: Vec<(String, String)> = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT CASE WHEN source_type = 'knowledge' THEN source_id ELSE target_id END,
                        relation
                 FROM entity_links
                 WHERE project_id = ?1
                   AND ((source_type = 'knowledge' AND target_type = ?2 AND target_id = ?3)
                     OR (target_type = 'knowledge' AND source_type = ?2 AND source_id = ?3))
                 ORDER BY relation, id LIMIT ?4",
            )?;
            statement
                .query_map(
                    params![
                        request.project_id,
                        entity_kind_name(request.subject.kind),
                        request.subject.id,
                        request.max_paths as i64
                    ],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?
                .collect()
        })?;
        let relations = links.into_iter().collect::<HashMap<_, _>>();
        let linked_ids = relations.keys().cloned().collect::<HashSet<_>>();
        let knowledge = KnowledgeService::new(self.storage.clone());
        let records = knowledge
            .list(&request.project_id, true)?
            .into_iter()
            .filter(|record| linked_ids.contains(&record.id))
            .collect::<Vec<_>>();
        let mut facts = Vec::new();
        let mut warnings = Vec::new();
        let mut sources = Vec::new();
        let mut paths = Vec::new();
        let mut approved_by_kind = HashMap::<KnowledgeKind, Vec<String>>::new();

        for record in records {
            if record.status != KnowledgeStatus::Approved {
                warnings.push(format!(
                    "{} knowledge `{}` remains visible but is {}",
                    status_name(record.status),
                    record.title,
                    status_name(record.status)
                ));
                continue;
            }
            approved_by_kind
                .entry(record.kind)
                .or_default()
                .push(normalize(&record.body));
            facts.push(WhyFact {
                knowledge_id: record.id.clone(),
                kind: record.kind,
                title: record.title.clone(),
                body: record.body.clone(),
                confidence: record.confidence,
                provenance: provenance_name(record.provenance_kind).into(),
            });
            let provenance = knowledge.get_provenance(&record.provenance_id)?;
            let evidence = evidence_locations(&provenance.evidence);
            let source_location = format!(
                "{}:{}",
                entity_kind_name(provenance.source.kind),
                provenance.source.id
            );
            sources.push(WhySource {
                label: format!("{} source", provenance_name(provenance.kind)),
                location: source_location.clone(),
            });
            sources.extend(evidence.iter().map(|location| WhySource {
                label: "Evidence".into(),
                location: location.clone(),
            }));
            let task = self.task_for_source(&provenance.source)?;
            let mut base = vec![
                WhyPathStep {
                    kind: entity_kind_name(request.subject.kind).into(),
                    label: request.subject.id.clone(),
                    location: format!(
                        "{}:{}",
                        entity_kind_name(request.subject.kind),
                        request.subject.id
                    ),
                },
                WhyPathStep {
                    kind: "knowledge".into(),
                    label: record.title.clone(),
                    location: format!("knowledge:{}", record.id),
                },
                WhyPathStep {
                    kind: entity_kind_name(provenance.source.kind).into(),
                    label: provenance_name(provenance.kind).into(),
                    location: source_location,
                },
            ];
            if let Some((task_id, title)) = task {
                base.push(WhyPathStep {
                    kind: "task".into(),
                    label: title,
                    location: format!("task:{task_id}"),
                });
            }
            if evidence.is_empty() {
                base.truncate(request.max_depth);
                paths.push(WhyPath {
                    relation: relations[&record.id].clone(),
                    steps: base,
                });
            } else {
                for location in evidence {
                    let mut steps = base.clone();
                    steps.push(WhyPathStep {
                        kind: "evidence".into(),
                        label: location.clone(),
                        location,
                    });
                    steps.truncate(request.max_depth);
                    paths.push(WhyPath {
                        relation: relations[&record.id].clone(),
                        steps,
                    });
                    if paths.len() >= request.max_paths {
                        break;
                    }
                }
            }
            if paths.len() >= request.max_paths {
                break;
            }
        }
        for (kind, bodies) in approved_by_kind {
            if bodies.into_iter().collect::<HashSet<_>>().len() > 1 {
                warnings.push(format!(
                    "conflicting approved {} facts require human resolution",
                    knowledge_kind_name(kind)
                ));
            }
        }
        sources.sort_by(|left, right| left.location.cmp(&right.location));
        sources.dedup_by(|left, right| left.location == right.location);
        warnings.sort();
        let status = if facts.is_empty() {
            WhyStatus::InsufficientEvidence
        } else if warnings
            .iter()
            .any(|warning| warning.contains("conflicting approved"))
        {
            WhyStatus::AnsweredWithConflicts
        } else {
            WhyStatus::Answered
        };
        Ok(WhyAnswer {
            status,
            subject: request.subject,
            facts,
            inferences: Vec::new(),
            warnings,
            sources,
            paths,
        })
    }

    fn task_for_source(&self, source: &EntityRef) -> Result<Option<(String, String)>, WhyError> {
        let task = match source.kind {
            EntityKind::Task => self.storage.with_connection(|connection| {
                connection
                    .query_row("SELECT id, title FROM tasks WHERE id = ?1", [&source.id], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })
                    .optional()
            })?,
            EntityKind::AgentSession => self.storage.with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT t.id, t.title FROM agent_sessions a JOIN tasks t ON t.id = a.task_id WHERE a.id = ?1",
                        [&source.id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()
            })?,
            _ => None,
        };
        Ok(task)
    }
}

use rusqlite::OptionalExtension;

fn evidence_locations(value: &Value) -> Vec<String> {
    let mut locations = Vec::new();
    collect_strings(value, None, &mut locations);
    locations.retain(|value| {
        value.starts_with("diff:")
            || value.starts_with("event:")
            || value.starts_with("commit:")
            || value.starts_with("task:")
    });
    locations.sort();
    locations.dedup();
    locations
}

fn collect_strings(value: &Value, key: Option<&str>, output: &mut Vec<String>) {
    match value {
        Value::String(text)
            if key.is_some_and(|key| key.to_ascii_lowercase().contains("evidence")) =>
        {
            output.push(text.clone());
        }
        Value::Array(items) => {
            for item in items {
                collect_strings(item, key, output);
            }
        }
        Value::Object(fields) => {
            for (field, value) in fields {
                collect_strings(value, Some(field), output);
            }
        }
        _ => {}
    }
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn entity_kind_name(kind: EntityKind) -> &'static str {
    match kind {
        EntityKind::Project => "project",
        EntityKind::Repository => "repository",
        EntityKind::Task => "task",
        EntityKind::Context => "context",
        EntityKind::Skill => "skill",
        EntityKind::AgentSession => "agent_session",
        EntityKind::File => "file",
        EntityKind::Symbol => "symbol",
        EntityKind::Knowledge => "knowledge",
        EntityKind::Branch => "branch",
        EntityKind::Worktree => "worktree",
    }
}

fn knowledge_kind_name(kind: KnowledgeKind) -> &'static str {
    match kind {
        KnowledgeKind::Summary => "summary",
        KnowledgeKind::Decision => "decision",
        KnowledgeKind::Constraint => "constraint",
        KnowledgeKind::Attempt => "attempt",
        KnowledgeKind::Risk => "risk",
        KnowledgeKind::Question => "question",
        KnowledgeKind::Test => "test",
        KnowledgeKind::Commit => "commit",
    }
}

fn status_name(status: KnowledgeStatus) -> &'static str {
    match status {
        KnowledgeStatus::Candidate => "candidate",
        KnowledgeStatus::Approved => "approved",
        KnowledgeStatus::Rejected => "rejected",
        KnowledgeStatus::Stale => "stale",
        KnowledgeStatus::Superseded => "superseded",
    }
}

fn provenance_name(kind: crate::knowledge::ProvenanceKind) -> &'static str {
    match kind {
        crate::knowledge::ProvenanceKind::Human => "HUMAN",
        crate::knowledge::ProvenanceKind::ExtractedCode => "EXTRACTED_CODE",
        crate::knowledge::ProvenanceKind::ExtractedGit => "EXTRACTED_GIT",
        crate::knowledge::ProvenanceKind::AgentObserved => "AGENT_OBSERVED",
        crate::knowledge::ProvenanceKind::AgentInferred => "AGENT_INFERRED",
        crate::knowledge::ProvenanceKind::Imported => "IMPORTED",
        crate::knowledge::ProvenanceKind::Ambiguous => "AMBIGUOUS",
    }
}
