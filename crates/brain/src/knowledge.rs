use std::cmp::Reverse;
use std::collections::HashSet;
use std::sync::Arc;

use mars_storage::{Storage, StorageError};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceKind {
    Human,
    ExtractedCode,
    ExtractedGit,
    AgentObserved,
    AgentInferred,
    Imported,
    Ambiguous,
}

impl ProvenanceKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::ExtractedCode => "extracted_code",
            Self::ExtractedGit => "extracted_git",
            Self::AgentObserved => "agent_observed",
            Self::AgentInferred => "agent_inferred",
            Self::Imported => "imported",
            Self::Ambiguous => "ambiguous",
        }
    }

    fn parse(value: &str) -> Result<Self, KnowledgeError> {
        match value {
            "human" => Ok(Self::Human),
            "extracted_code" => Ok(Self::ExtractedCode),
            "extracted_git" => Ok(Self::ExtractedGit),
            "agent_observed" => Ok(Self::AgentObserved),
            "agent_inferred" => Ok(Self::AgentInferred),
            "imported" => Ok(Self::Imported),
            "ambiguous" => Ok(Self::Ambiguous),
            other => Err(KnowledgeError::InvalidStoredValue(other.into())),
        }
    }

    fn authority(self) -> u8 {
        match self {
            Self::Human => 100,
            Self::ExtractedCode | Self::ExtractedGit => 80,
            Self::Imported => 70,
            Self::AgentObserved => 60,
            Self::AgentInferred => 40,
            Self::Ambiguous => 10,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeKind {
    Summary,
    Decision,
    Constraint,
    Attempt,
    Risk,
    Question,
    Test,
    Commit,
}

impl KnowledgeKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Decision => "decision",
            Self::Constraint => "constraint",
            Self::Attempt => "attempt",
            Self::Risk => "risk",
            Self::Question => "question",
            Self::Test => "test",
            Self::Commit => "commit",
        }
    }

    fn parse(value: &str) -> Result<Self, KnowledgeError> {
        match value {
            "summary" => Ok(Self::Summary),
            "decision" => Ok(Self::Decision),
            "constraint" => Ok(Self::Constraint),
            "attempt" => Ok(Self::Attempt),
            "risk" => Ok(Self::Risk),
            "question" => Ok(Self::Question),
            "test" => Ok(Self::Test),
            "commit" => Ok(Self::Commit),
            other => Err(KnowledgeError::InvalidStoredValue(other.into())),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeStatus {
    Candidate,
    Approved,
    Rejected,
    Stale,
    Superseded,
}

impl KnowledgeStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Stale => "stale",
            Self::Superseded => "superseded",
        }
    }

    fn parse(value: &str) -> Result<Self, KnowledgeError> {
        match value {
            "candidate" => Ok(Self::Candidate),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            "stale" => Ok(Self::Stale),
            "superseded" => Ok(Self::Superseded),
            other => Err(KnowledgeError::InvalidStoredValue(other.into())),
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Approved => 5,
            Self::Candidate => 4,
            Self::Stale => 3,
            Self::Superseded => 2,
            Self::Rejected => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Project,
    Repository,
    Task,
    Context,
    Skill,
    AgentSession,
    File,
    Symbol,
    Knowledge,
    Branch,
    Worktree,
}

impl EntityKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Repository => "repository",
            Self::Task => "task",
            Self::Context => "context",
            Self::Skill => "skill",
            Self::AgentSession => "agent_session",
            Self::File => "file",
            Self::Symbol => "symbol",
            Self::Knowledge => "knowledge",
            Self::Branch => "branch",
            Self::Worktree => "worktree",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityRef {
    pub kind: EntityKind,
    pub id: String,
}

impl EntityRef {
    #[must_use]
    pub fn new(kind: EntityKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
        }
    }

    fn display(&self) -> String {
        format!("{}:{}", self.kind.as_str(), self.id)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewProvenance {
    pub project_id: String,
    pub kind: ProvenanceKind,
    pub source: EntityRef,
    pub evidence: Value,
    pub observed_at: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub id: String,
    pub project_id: String,
    pub repository_id: Option<String>,
    pub kind: ProvenanceKind,
    pub source: EntityRef,
    pub evidence: Value,
    pub observed_at: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewKnowledge {
    pub project_id: String,
    pub kind: KnowledgeKind,
    pub title: String,
    pub body: String,
    pub confidence: f64,
    pub provenance_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeRecord {
    pub id: String,
    pub project_id: String,
    pub repository_id: Option<String>,
    pub kind: KnowledgeKind,
    pub title: String,
    pub body: String,
    pub status: KnowledgeStatus,
    pub confidence: f64,
    pub provenance_id: String,
    pub provenance_kind: ProvenanceKind,
    pub resolution_provenance_id: Option<String>,
    pub supersedes_id: Option<String>,
    pub pinned: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityLink {
    pub id: String,
    pub project_id: String,
    pub source: EntityRef,
    pub relation: String,
    pub target: EntityRef,
    pub provenance_id: String,
    pub confidence: f64,
    pub created_at: String,
}

#[derive(Debug, Error)]
pub enum KnowledgeError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("confidence must be a finite value between zero and one: {0}")]
    InvalidConfidence(f64),
    #[error("{0} cannot be empty")]
    EmptyField(&'static str),
    #[error("only HUMAN provenance can approve, correct, reject, or mark knowledge stale")]
    HumanResolutionRequired,
    #[error("provenance `{0}` does not exist")]
    ProvenanceNotFound(String),
    #[error("knowledge record `{0}` does not exist")]
    RecordNotFound(String),
    #[error("knowledge record `{id}` cannot transition from {from} to {to}")]
    InvalidTransition {
        id: String,
        from: String,
        to: String,
    },
    #[error("entity `{0}` does not exist")]
    EntityNotFound(String),
    #[error("entity `{entity}` does not belong to project `{project_id}`")]
    ProjectMismatch { entity: String, project_id: String },
    #[error("unsupported knowledge relation `{0}`")]
    InvalidRelation(String),
    #[error("stored knowledge value is invalid: {0}")]
    InvalidStoredValue(String),
    #[error("failed to format knowledge timestamp: {0}")]
    Timestamp(#[from] time::error::Format),
}

/// How a record reached the knowledge store.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeVia {
    /// An agent recorded it while it worked, through the Mars MCP server.
    Agent,
    /// Extraction read it out of a finished session.
    Extraction,
    /// A person wrote it, or corrected what an agent wrote.
    Person,
}

/// Where a record came from — what a person reviewing it needs in order to
/// check it: the task and agent, what else was weighed, what it touches.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeOrigin {
    pub task_id: Option<String>,
    pub agent: Option<String>,
    pub run_id: Option<String>,
    pub via: KnowledgeVia,
    pub alternatives: Vec<String>,
    /// Repository-relative paths, linked through the index or named by the
    /// agent.
    pub files: Vec<String>,
}

fn evidence_strings(evidence: &Value, name: &str) -> Vec<String> {
    evidence
        .get(name)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn evidence_string(evidence: &Value, name: &str) -> Option<String> {
    evidence
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[derive(Clone)]
pub struct KnowledgeService {
    storage: Arc<Storage>,
}

impl KnowledgeService {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }

    pub fn create_provenance(&self, input: NewProvenance) -> Result<Provenance, KnowledgeError> {
        validate_nonempty("project_id", &input.project_id)?;
        validate_nonempty("observed_at", &input.observed_at)?;
        if !input.evidence.is_object() && !input.evidence.is_array() {
            return Err(KnowledgeError::EmptyField("evidence"));
        }
        let repository_id = self.validate_entity(&input.project_id, &input.source)?;
        let id = Uuid::now_v7().to_string();
        let created_at = now()?;
        let evidence = serde_json::to_string(&input.evidence)?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "INSERT INTO provenance
                 (id, project_id, repository_id, kind, source_type, source_id, evidence_json, observed_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![id, input.project_id, repository_id, input.kind.as_str(), input.source.kind.as_str(), input.source.id, evidence, input.observed_at, created_at],
            )?;
            Ok(())
        })?;
        Ok(Provenance {
            id,
            project_id: input.project_id,
            repository_id,
            kind: input.kind,
            source: input.source,
            evidence: input.evidence,
            observed_at: input.observed_at,
            created_at,
        })
    }

    pub fn create_candidate(&self, input: NewKnowledge) -> Result<KnowledgeRecord, KnowledgeError> {
        validate_confidence(input.confidence)?;
        validate_nonempty("title", &input.title)?;
        validate_nonempty("body", &input.body)?;
        let provenance = self.provenance(&input.provenance_id)?;
        if provenance.project_id != input.project_id {
            return Err(KnowledgeError::ProjectMismatch {
                entity: format!("provenance:{}", provenance.id),
                project_id: input.project_id,
            });
        }
        let id = Uuid::now_v7().to_string();
        let timestamp = now()?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "INSERT INTO knowledge_records
                 (id, project_id, repository_id, kind, title, body, status, confidence, provenance_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'candidate', ?7, ?8, ?9, ?9)",
                params![id, input.project_id, provenance.repository_id, input.kind.as_str(), input.title, input.body, input.confidence, input.provenance_id, timestamp],
            )?;
            Ok(())
        })?;
        self.get(&id)
    }

    pub fn approve(
        &self,
        record_id: &str,
        resolver_id: &str,
    ) -> Result<KnowledgeRecord, KnowledgeError> {
        let record = self.get(record_id)?;
        self.require_human_resolver(&record.project_id, resolver_id)?;
        self.transition(
            &record,
            KnowledgeStatus::Candidate,
            KnowledgeStatus::Approved,
            resolver_id,
        )?;
        self.get(record_id)
    }

    pub fn reject(
        &self,
        record_id: &str,
        resolver_id: &str,
    ) -> Result<KnowledgeRecord, KnowledgeError> {
        let record = self.get(record_id)?;
        self.require_human_resolver(&record.project_id, resolver_id)?;
        self.transition(
            &record,
            KnowledgeStatus::Candidate,
            KnowledgeStatus::Rejected,
            resolver_id,
        )?;
        self.get(record_id)
    }

    pub fn mark_stale(
        &self,
        record_id: &str,
        resolver_id: &str,
    ) -> Result<KnowledgeRecord, KnowledgeError> {
        let record = self.get(record_id)?;
        self.require_human_resolver(&record.project_id, resolver_id)?;
        self.transition(
            &record,
            KnowledgeStatus::Approved,
            KnowledgeStatus::Stale,
            resolver_id,
        )?;
        self.get(record_id)
    }

    pub fn correct(
        &self,
        record_id: &str,
        title: &str,
        body: &str,
        resolver_id: &str,
    ) -> Result<KnowledgeRecord, KnowledgeError> {
        validate_nonempty("title", title)?;
        validate_nonempty("body", body)?;
        let original = self.get(record_id)?;
        let resolver = self.require_human_resolver(&original.project_id, resolver_id)?;
        if original.status != KnowledgeStatus::Approved {
            return Err(invalid_transition(&original, KnowledgeStatus::Superseded));
        }
        let id = Uuid::now_v7().to_string();
        let link_id = Uuid::now_v7().to_string();
        let timestamp = now()?;
        self.storage.with_transaction(|transaction| {
            transaction.execute(
                "UPDATE knowledge_records SET status = 'superseded', resolution_provenance_id = ?2, updated_at = ?3 WHERE id = ?1 AND status = 'approved'",
                params![record_id, resolver_id, timestamp],
            )?;
            transaction.execute("DELETE FROM knowledge_fts WHERE entity_kind = 'knowledge' AND entity_id = ?1", [record_id])?;
            transaction.execute(
                "INSERT INTO knowledge_records
                 (id, project_id, repository_id, kind, title, body, status, confidence, provenance_id,
                  resolution_provenance_id, supersedes_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'approved', 1.0, ?7, ?7, ?8, ?9, ?9)",
                params![id, original.project_id, original.repository_id, original.kind.as_str(), title, body, resolver.id, record_id, timestamp],
            )?;
            if let Some(repository_id) = &original.repository_id {
                transaction.execute(
                    "INSERT INTO knowledge_fts (repository_id, entity_kind, entity_id, title, body) VALUES (?1, 'knowledge', ?2, ?3, ?4)",
                    params![repository_id, id, title, body],
                )?;
            }
            let inherited_links: Vec<(String, String, String, String, String, f64)> = {
                let mut statement = transaction.prepare(
                    "SELECT source_type, source_id, relation, target_type, target_id, confidence
                     FROM entity_links
                     WHERE project_id = ?1
                       AND relation != 'SUPERSEDES'
                       AND ((source_type = 'knowledge' AND source_id = ?2)
                         OR (target_type = 'knowledge' AND target_id = ?2))",
                )?;
                statement
                    .query_map(params![original.project_id, record_id], |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                        ))
                    })?
                    .collect::<Result<_, _>>()?
            };
            for (source_type, source_id, relation, target_type, target_id, confidence) in
                inherited_links
            {
                let source_id = if source_type == "knowledge" && source_id == record_id {
                    id.clone()
                } else {
                    source_id
                };
                let target_id = if target_type == "knowledge" && target_id == record_id {
                    id.clone()
                } else {
                    target_id
                };
                transaction.execute(
                    "INSERT INTO entity_links
                     (id, project_id, source_type, source_id, relation, target_type, target_id, provenance_id, confidence, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![Uuid::now_v7().to_string(), original.project_id, source_type, source_id, relation, target_type, target_id, resolver.id, confidence, timestamp],
                )?;
            }
            transaction.execute(
                "INSERT INTO entity_links
                 (id, project_id, source_type, source_id, relation, target_type, target_id, provenance_id, confidence, created_at)
                 VALUES (?1, ?2, 'knowledge', ?3, 'SUPERSEDES', 'knowledge', ?4, ?5, 1.0, ?6)",
                params![link_id, original.project_id, id, record_id, resolver.id, timestamp],
            )?;
            Ok(())
        })?;
        self.get(&id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn link(
        &self,
        project_id: &str,
        source: EntityRef,
        relation: &str,
        target: EntityRef,
        provenance_id: &str,
        confidence: f64,
    ) -> Result<EntityLink, KnowledgeError> {
        validate_confidence(confidence)?;
        let relation = relation.trim().to_ascii_uppercase();
        if !matches!(
            relation.as_str(),
            "ABOUT"
                | "AFFECTS"
                | "CONSTRAINS"
                | "DECIDED_IN"
                | "ATTACHED_TO"
                | "PRODUCED_BY"
                | "REVIEWS"
                | "IMPLEMENTS_TASK"
                | "CHANGED_BY"
                | "COMMITTED_AS"
                | "DISCUSSED_IN"
                | "SUPERSEDES"
                | "DEPENDS_ON"
        ) {
            return Err(KnowledgeError::InvalidRelation(relation));
        }
        self.validate_entity(project_id, &source)?;
        self.validate_entity(project_id, &target)?;
        let provenance = self.provenance(provenance_id)?;
        if provenance.project_id != project_id {
            return Err(KnowledgeError::ProjectMismatch {
                entity: format!("provenance:{provenance_id}"),
                project_id: project_id.into(),
            });
        }
        let id = Uuid::now_v7().to_string();
        let created_at = now()?;
        let id = self.storage.with_connection(|connection| {
            connection.execute(
                "INSERT INTO entity_links
                 (id, project_id, source_type, source_id, relation, target_type, target_id, provenance_id, confidence, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(project_id, source_type, source_id, relation, target_type, target_id)
                 DO UPDATE SET provenance_id = excluded.provenance_id, confidence = excluded.confidence",
                params![id, project_id, source.kind.as_str(), source.id, relation, target.kind.as_str(), target.id, provenance_id, confidence, created_at],
            )?;
            connection.query_row(
                "SELECT id FROM entity_links WHERE project_id = ?1 AND source_type = ?2
                 AND source_id = ?3 AND relation = ?4 AND target_type = ?5 AND target_id = ?6",
                params![project_id, source.kind.as_str(), source.id, relation, target.kind.as_str(), target.id],
                |row| row.get(0),
            )
        })?;
        Ok(EntityLink {
            id,
            project_id: project_id.into(),
            source,
            relation,
            target,
            provenance_id: provenance_id.into(),
            confidence,
            created_at,
        })
    }

    pub fn get(&self, record_id: &str) -> Result<KnowledgeRecord, KnowledgeError> {
        let row = self.storage.with_connection(|connection| {
            connection.query_row(
                "SELECT k.id, k.project_id, k.repository_id, k.kind, k.title, k.body, k.status,
                        k.confidence, k.provenance_id, p.kind, k.resolution_provenance_id,
                        k.supersedes_id, k.pinned, k.created_at, k.updated_at
                 FROM knowledge_records k JOIN provenance p ON p.id = k.provenance_id WHERE k.id = ?1",
                [record_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?, row.get(12)?, row.get(13)?, row.get(14)?)),
            ).optional()
        })?;
        row.map(record_from_row)
            .transpose()?
            .ok_or_else(|| KnowledgeError::RecordNotFound(record_id.into()))
    }

    pub fn list(
        &self,
        project_id: &str,
        include_inactive: bool,
    ) -> Result<Vec<KnowledgeRecord>, KnowledgeError> {
        let rows: Vec<RawRecord> = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT k.id, k.project_id, k.repository_id, k.kind, k.title, k.body, k.status,
                        k.confidence, k.provenance_id, p.kind, k.resolution_provenance_id,
                        k.supersedes_id, k.pinned, k.created_at, k.updated_at
                 FROM knowledge_records k JOIN provenance p ON p.id = k.provenance_id
                 WHERE k.project_id = ?1 AND (?2 OR k.status IN ('candidate', 'approved')) ORDER BY k.id",
            )?;
            statement.query_map(params![project_id, include_inactive], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?, row.get(12)?, row.get(13)?, row.get(14)?)))?.collect()
        })?;
        let mut records = rows
            .into_iter()
            .map(record_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        records.sort_by_key(|record| {
            (
                Reverse(record.status.rank()),
                Reverse(record.provenance_kind.authority()),
                Reverse((record.confidence * 1_000_000.0) as i64),
                record.created_at.clone(),
                record.id.clone(),
            )
        });
        Ok(records)
    }

    pub fn count(&self, project_id: &str) -> Result<usize, KnowledgeError> {
        Ok(self.storage.with_connection(|connection| {
            connection.query_row(
                "SELECT COUNT(*) FROM knowledge_records WHERE project_id = ?1",
                [project_id],
                |row| row.get::<_, i64>(0),
            )
        })? as usize)
    }

    pub fn list_for_task(&self, task_id: &str) -> Result<Vec<KnowledgeRecord>, KnowledgeError> {
        let project_id: String = self.storage.with_connection(|connection| {
            connection.query_row(
                "SELECT project_id FROM tasks WHERE id = ?1",
                [task_id],
                |row| row.get(0),
            )
        })?;
        let ids: HashSet<String> = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT k.id FROM knowledge_records k JOIN provenance p ON p.id = k.provenance_id
                 WHERE (p.source_type = 'task' AND p.source_id = ?1)
                    OR (p.source_type = 'agent_session' AND EXISTS (
                        SELECT 1 FROM agent_sessions a WHERE a.id = p.source_id AND a.task_id = ?1
                    ))
                    OR EXISTS (
                        SELECT 1 FROM entity_links l WHERE l.source_type = 'knowledge' AND l.source_id = k.id
                          AND l.target_type = 'task' AND l.target_id = ?1
                    )",
            )?;
            statement.query_map([task_id], |row| row.get(0))?.collect()
        })?;
        Ok(self
            .list(&project_id, true)?
            .into_iter()
            .filter(|record| ids.contains(&record.id))
            .collect())
    }

    pub fn set_pinned(
        &self,
        record_id: &str,
        pinned: bool,
        resolver_id: &str,
    ) -> Result<KnowledgeRecord, KnowledgeError> {
        let record = self.get(record_id)?;
        self.require_human_resolver(&record.project_id, resolver_id)?;
        if !matches!(
            record.status,
            KnowledgeStatus::Approved | KnowledgeStatus::Stale
        ) {
            return Err(invalid_transition(&record, record.status));
        }
        let timestamp = now()?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE knowledge_records SET pinned = ?2, resolution_provenance_id = ?3, updated_at = ?4 WHERE id = ?1",
                params![record_id, pinned, resolver_id, timestamp],
            )?;
            Ok(())
        })?;
        self.get(record_id)
    }

    /// Where a record came from. A correction is the person's, but it keeps
    /// the task, agent and alternatives of the record it superseded.
    pub fn origin(&self, record: &KnowledgeRecord) -> Result<KnowledgeOrigin, KnowledgeError> {
        let provenance = self.provenance(&record.provenance_id)?;
        if provenance.kind == ProvenanceKind::Human
            && let Some(superseded) = &record.supersedes_id
        {
            let original = self.get(superseded)?;
            let mut origin = self.origin(&original)?;
            origin.via = KnowledgeVia::Person;
            for path in self.linked_files(&record.id)? {
                if !origin.files.contains(&path) {
                    origin.files.push(path);
                }
            }
            return Ok(origin);
        }
        let evidence = &provenance.evidence;
        let via = match provenance.kind {
            ProvenanceKind::Human => KnowledgeVia::Person,
            _ if evidence_string(evidence, "via").as_deref() == Some("mcp") => KnowledgeVia::Agent,
            _ => KnowledgeVia::Extraction,
        };
        let (session_task, session_agent): (Option<String>, Option<String>) =
            if provenance.source.kind == EntityKind::AgentSession {
                self.storage
                    .with_connection(|connection| {
                        connection
                            .query_row(
                                "SELECT task_id, adapter_kind FROM agent_sessions WHERE id = ?1",
                                [&provenance.source.id],
                                |row| Ok((row.get(0)?, row.get(1)?)),
                            )
                            .optional()
                    })?
                    .unwrap_or((None, None))
            } else {
                (None, None)
            };
        let task_id = match provenance.source.kind {
            EntityKind::Task => Some(provenance.source.id.clone()),
            EntityKind::AgentSession => session_task,
            _ => None,
        };
        let task_id = match task_id {
            Some(task_id) => Some(task_id),
            None => self.storage.with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT target_id FROM entity_links
                         WHERE source_type = 'knowledge' AND source_id = ?1 AND target_type = 'task'
                         ORDER BY created_at LIMIT 1",
                        [&record.id],
                        |row| row.get(0),
                    )
                    .optional()
            })?,
        };
        let mut files = self.linked_files(&record.id)?;
        // A path the agent named that the index does not know still matters
        // to whoever reads the decision.
        for path in evidence_strings(evidence, "files") {
            if !files.contains(&path) {
                files.push(path);
            }
        }
        Ok(KnowledgeOrigin {
            task_id,
            agent: evidence_string(evidence, "agent").or(session_agent),
            run_id: evidence_string(evidence, "runId"),
            via,
            alternatives: evidence_strings(evidence, "alternatives"),
            files,
        })
    }

    fn linked_files(&self, record_id: &str) -> Result<Vec<String>, KnowledgeError> {
        Ok(self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT DISTINCT f.relative_path FROM entity_links l JOIN files f ON f.id = l.target_id
                 WHERE l.source_type = 'knowledge' AND l.source_id = ?1 AND l.target_type = 'file'
                   AND l.relation = 'AFFECTS'
                 ORDER BY f.relative_path",
            )?;
            statement.query_map([record_id], |row| row.get(0))?.collect()
        })?)
    }

    pub fn get_provenance(&self, id: &str) -> Result<Provenance, KnowledgeError> {
        self.provenance(id)
    }

    fn provenance(&self, id: &str) -> Result<Provenance, KnowledgeError> {
        let row: Option<RawProvenance> = self.storage.with_connection(|connection| {
            connection.query_row(
                "SELECT id, project_id, repository_id, kind, source_type, source_id, evidence_json, observed_at, created_at FROM provenance WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?)),
            ).optional()
        })?;
        let Some((
            id,
            project_id,
            repository_id,
            kind,
            source_type,
            source_id,
            evidence,
            observed_at,
            created_at,
        )) = row
        else {
            return Err(KnowledgeError::ProvenanceNotFound(id.into()));
        };
        Ok(Provenance {
            id,
            project_id,
            repository_id,
            kind: ProvenanceKind::parse(&kind)?,
            source: EntityRef::new(parse_entity_kind(&source_type)?, source_id),
            evidence: serde_json::from_str(&evidence)?,
            observed_at,
            created_at,
        })
    }

    fn require_human_resolver(
        &self,
        project_id: &str,
        id: &str,
    ) -> Result<Provenance, KnowledgeError> {
        let provenance = self.provenance(id)?;
        if provenance.project_id != project_id {
            return Err(KnowledgeError::ProjectMismatch {
                entity: format!("provenance:{id}"),
                project_id: project_id.into(),
            });
        }
        if provenance.kind != ProvenanceKind::Human {
            return Err(KnowledgeError::HumanResolutionRequired);
        }
        Ok(provenance)
    }

    fn transition(
        &self,
        record: &KnowledgeRecord,
        expected: KnowledgeStatus,
        target: KnowledgeStatus,
        resolver_id: &str,
    ) -> Result<(), KnowledgeError> {
        if record.status != expected {
            return Err(invalid_transition(record, target));
        }
        let timestamp = now()?;
        self.storage.with_transaction(|transaction| {
            transaction.execute(
                "UPDATE knowledge_records SET status = ?2, resolution_provenance_id = ?3, updated_at = ?4 WHERE id = ?1 AND status = ?5",
                params![record.id, target.as_str(), resolver_id, timestamp, expected.as_str()],
            )?;
            transaction.execute("DELETE FROM knowledge_fts WHERE entity_kind = 'knowledge' AND entity_id = ?1", [&record.id])?;
            if target == KnowledgeStatus::Approved && let Some(repository_id) = &record.repository_id {
                transaction.execute(
                    "INSERT INTO knowledge_fts (repository_id, entity_kind, entity_id, title, body) VALUES (?1, 'knowledge', ?2, ?3, ?4)",
                    params![repository_id, record.id, record.title, record.body],
                )?;
            }
            Ok(())
        })?;
        Ok(())
    }

    fn validate_entity(
        &self,
        project_id: &str,
        entity: &EntityRef,
    ) -> Result<Option<String>, KnowledgeError> {
        if entity.kind == EntityKind::Skill {
            let (project_exists, skill_exists): (bool, bool) =
                self.storage.with_connection(|connection| {
                    Ok((
                        connection.query_row(
                            "SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1)",
                            [project_id],
                            |row| row.get(0),
                        )?,
                        connection.query_row(
                            "SELECT EXISTS(SELECT 1 FROM skills WHERE id = ?1)",
                            [&entity.id],
                            |row| row.get(0),
                        )?,
                    ))
                })?;
            if !project_exists || !skill_exists {
                return Err(KnowledgeError::EntityNotFound(entity.display()));
            }
            return Ok(None);
        }
        let found: Option<(String, Option<String>)> =
            self.storage.with_connection(|connection| {
                entity_project(connection, entity)
                    .map(Option::from)
                    .or_else(|error| {
                        if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
                            Ok(None)
                        } else {
                            Err(error)
                        }
                    })
            })?;
        let Some((actual_project, repository_id)) = found else {
            return Err(KnowledgeError::EntityNotFound(entity.display()));
        };
        if actual_project != project_id {
            return Err(KnowledgeError::ProjectMismatch {
                entity: entity.display(),
                project_id: project_id.into(),
            });
        }
        Ok(repository_id)
    }
}

type RawRecord = (
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    f64,
    String,
    String,
    Option<String>,
    Option<String>,
    bool,
    String,
    String,
);
type RawProvenance = (
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    String,
    String,
);

fn record_from_row(row: RawRecord) -> Result<KnowledgeRecord, KnowledgeError> {
    Ok(KnowledgeRecord {
        id: row.0,
        project_id: row.1,
        repository_id: row.2,
        kind: KnowledgeKind::parse(&row.3)?,
        title: row.4,
        body: row.5,
        status: KnowledgeStatus::parse(&row.6)?,
        confidence: row.7,
        provenance_id: row.8,
        provenance_kind: ProvenanceKind::parse(&row.9)?,
        resolution_provenance_id: row.10,
        supersedes_id: row.11,
        pinned: row.12,
        created_at: row.13,
        updated_at: row.14,
    })
}

fn entity_project(
    connection: &rusqlite::Connection,
    entity: &EntityRef,
) -> rusqlite::Result<(String, Option<String>)> {
    let query = match entity.kind {
        EntityKind::Project => "SELECT id, NULL FROM projects WHERE id = ?1",
        EntityKind::Repository => "SELECT project_id, id FROM repositories WHERE id = ?1",
        EntityKind::Task => "SELECT project_id, repository_id FROM tasks WHERE id = ?1",
        EntityKind::Context => {
            "SELECT r.project_id, c.repository_id FROM context_items c JOIN repositories r ON r.id = c.repository_id WHERE c.id = ?1"
        }
        EntityKind::Skill => unreachable!("skills are project-neutral and validated separately"),
        EntityKind::AgentSession => {
            "SELECT t.project_id, t.repository_id FROM agent_sessions a JOIN tasks t ON t.id = a.task_id WHERE a.id = ?1"
        }
        EntityKind::File => {
            "SELECT r.project_id, f.repository_id FROM files f JOIN repositories r ON r.id = f.repository_id WHERE f.id = ?1"
        }
        EntityKind::Symbol => {
            "SELECT r.project_id, f.repository_id FROM symbols s JOIN files f ON f.id = s.file_id JOIN repositories r ON r.id = f.repository_id WHERE s.id = ?1"
        }
        EntityKind::Knowledge => {
            "SELECT project_id, repository_id FROM knowledge_records WHERE id = ?1"
        }
        EntityKind::Branch => {
            "SELECT r.project_id, b.repository_id FROM branches b JOIN repositories r ON r.id = b.repository_id WHERE b.id = ?1"
        }
        EntityKind::Worktree => {
            "SELECT r.project_id, w.repository_id FROM worktrees w JOIN repositories r ON r.id = w.repository_id WHERE w.id = ?1"
        }
    };
    connection.query_row(query, [&entity.id], |row| Ok((row.get(0)?, row.get(1)?)))
}

fn parse_entity_kind(value: &str) -> Result<EntityKind, KnowledgeError> {
    match value {
        "project" => Ok(EntityKind::Project),
        "repository" => Ok(EntityKind::Repository),
        "task" => Ok(EntityKind::Task),
        "context" => Ok(EntityKind::Context),
        "skill" => Ok(EntityKind::Skill),
        "agent_session" => Ok(EntityKind::AgentSession),
        "file" => Ok(EntityKind::File),
        "symbol" => Ok(EntityKind::Symbol),
        "knowledge" => Ok(EntityKind::Knowledge),
        "branch" => Ok(EntityKind::Branch),
        "worktree" => Ok(EntityKind::Worktree),
        other => Err(KnowledgeError::InvalidStoredValue(other.into())),
    }
}

fn validate_confidence(value: f64) -> Result<(), KnowledgeError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(KnowledgeError::InvalidConfidence(value))
    }
}

fn validate_nonempty(field: &'static str, value: &str) -> Result<(), KnowledgeError> {
    if value.trim().is_empty() {
        Err(KnowledgeError::EmptyField(field))
    } else {
        Ok(())
    }
}

fn invalid_transition(record: &KnowledgeRecord, target: KnowledgeStatus) -> KnowledgeError {
    KnowledgeError::InvalidTransition {
        id: record.id.clone(),
        from: record.status.as_str().into(),
        to: target.as_str().into(),
    }
}

fn now() -> Result<String, KnowledgeError> {
    Ok(OffsetDateTime::now_utc().format(&Rfc3339)?)
}
