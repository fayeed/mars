use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use mars_storage::Storage;
use rusqlite::params;

use crate::context_pack::{ContextPackError, PackSection};
use crate::memory;

type ContextRow = (
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
);
type SymbolRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    i64,
    i64,
    String,
);
type SkillRow = (String, String, String, String, bool, bool, String, String);

#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub section: PackSection,
    pub entity_ref: String,
    pub title: String,
    pub content: String,
    pub provenance: String,
    pub freshness: String,
    pub source_location: String,
    pub reason: String,
    pub score: f64,
    pub required: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Excluded {
    pub entity_ref: String,
    pub reason: String,
}

pub(crate) fn retrieve(
    storage: &Arc<Storage>,
    task_id: &str,
    adapter: &str,
) -> Result<(Vec<Candidate>, Vec<Excluded>), ContextPackError> {
    let (project_id, repository_id, intent, root_path): (String, String, String, String) =
        storage.with_connection(|connection| {
            connection.query_row(
                "SELECT t.project_id, t.repository_id, t.intent, r.root_path FROM tasks t
             JOIN repositories r ON r.id = t.repository_id WHERE t.id = ?1",
                [task_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
        })?;
    let terms = terms(&intent);
    let mut candidates = Vec::new();
    let mut exclusions = Vec::new();

    // Approved memories are instructions: a convention or a caution is
    // something the agent must follow, not background reading. Only active
    // memories are eligible.
    //
    // Building a pack never counts as using them. A pack is built to be
    // previewed, and rebuilt again to be revalidated, before anything is
    // handed to an agent; counting those would make the usage trail claim
    // influence that was never had. The injection is recorded at launch, from
    // the pack that was actually sent, by `memory::record_uses`.
    let memories = memory::MemoryService::new(storage.clone())
        .for_retrieval(&memory::RetrievalContext {
            project_id,
            repository_id: Some(repository_id.clone()),
            task_id: Some(task_id.to_owned()),
            session_id: None,
            query: intent.clone(),
            surface: "context_pack".into(),
            limit: None,
            record_uses: false,
        })
        .map_err(|error| ContextPackError::Memory(error.to_string()))?;
    for retrieved in memories {
        let memory = retrieved.memory;
        candidates.push(Candidate {
            section: PackSection::Instructions,
            entity_ref: memory::pack_entity_ref(&memory.id),
            title: format!("{} ({})", memory.kind.as_str(), memory.scope.as_str()),
            content: memory.text,
            provenance: format!("HUMAN approved by {}", memory.approved_by),
            freshness: format!("approved {}", memory.approved_at),
            source_location: format!(
                "conversation:{}:{}",
                memory.origin.conversation_id, memory.origin.message_id
            ),
            reason: retrieved.reason,
            score: retrieved.relevance,
            // A caution is the one kind whose absence can cause harm, so it is
            // carried even when the section budget is tight.
            required: memory.kind == memory::MemoryKind::Caution,
        });
    }

    let context_rows: Vec<ContextRow> = storage.with_connection(|connection| {
        let mut statement = connection.prepare(
            "SELECT c.id, c.title, c.body_text, c.mime_type, c.classification, c.quarantine_reason
                 FROM context_items c JOIN context_links l ON l.context_item_id = c.id
                 WHERE l.entity_kind = 'task' AND l.entity_id = ?1
                 ORDER BY c.captured_at, c.id",
        )?;
        statement
            .query_map([task_id], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })?
            .collect()
    })?;
    let mut attached_ids = HashSet::new();
    for (id, title, body, mime, classification, quarantine) in context_rows {
        let entity_ref = format!("context:{id}");
        if let Some(reason) = quarantine {
            exclusions.push(Excluded {
                entity_ref,
                reason: format!("secret quarantine: {reason}"),
            });
            continue;
        }
        attached_ids.insert(id.clone());
        let constraint = classification.as_deref() == Some("constraint");
        candidates.push(Candidate {
            section: if constraint {
                PackSection::Instructions
            } else {
                PackSection::Context
            },
            entity_ref,
            title,
            content: body.unwrap_or_else(|| format!("Attached artifact ({mime})")),
            provenance: "HUMAN".into(),
            freshness: "current task attachment".into(),
            source_location: format!("context:{id}"),
            reason: if constraint {
                "active task constraint"
            } else {
                "explicit task attachment"
            }
            .into(),
            score: if constraint { 1.0 } else { 0.98 },
            required: true,
        });
    }

    if !terms.is_empty() {
        let pattern = terms
            .iter()
            .map(|term| format!("\"{}\"*", term.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" OR ");
        let lexical: Vec<(String, String, Option<String>, String)> =
            storage.with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT c.id, c.title, c.body_text, c.mime_type
                     FROM context_fts f JOIN context_items c ON c.id = f.context_item_id
                     WHERE context_fts MATCH ?1 AND c.repository_id = ?2
                       AND c.quarantine_reason IS NULL
                     ORDER BY bm25(context_fts), c.id LIMIT 20",
                )?;
                statement
                    .query_map(params![pattern, repository_id], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                    })?
                    .collect()
            })?;
        for (id, title, body, mime) in lexical {
            if attached_ids.contains(&id) {
                continue;
            }
            candidates.push(Candidate {
                section: PackSection::Context,
                entity_ref: format!("context:{id}"),
                title,
                content: body.unwrap_or_else(|| format!("Project artifact ({mime})")),
                provenance: "HUMAN".into(),
                freshness: "current project context".into(),
                source_location: format!("context:{id}"),
                reason: "project context FTS match".into(),
                score: 0.62,
                required: false,
            });
        }
    }

    if !terms.is_empty() {
        let pattern = fts_pattern(&terms);
        let knowledge: Vec<(String, String, String, String)> =
            storage.with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT entity_kind, entity_id, title, body FROM knowledge_fts
                 WHERE knowledge_fts MATCH ?1 AND repository_id = ?2
                 ORDER BY bm25(knowledge_fts), entity_kind, entity_id LIMIT 20",
                )?;
                statement
                    .query_map(params![pattern, repository_id], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                    })?
                    .collect()
            })?;
        for (kind, id, title, body) in knowledge {
            candidates.push(Candidate {
                section: if kind == "constraint" {
                    PackSection::Instructions
                } else {
                    PackSection::Context
                },
                entity_ref: format!("{kind}:{id}"),
                title,
                content: body,
                provenance: "HUMAN".into(),
                freshness: "current indexed knowledge".into(),
                source_location: format!("knowledge:{kind}:{id}"),
                reason: "project knowledge FTS match".into(),
                score: 0.67,
                required: kind == "constraint",
            });
        }
    }

    let symbols: Vec<SymbolRow> = storage.with_connection(|connection| {
        let mut statement = connection.prepare(
            "SELECT s.id, s.name, s.kind, COALESCE(s.qualified_name, s.name), s.signature,
                        s.start_line, s.end_line, f.relative_path
                 FROM symbols s JOIN files f ON f.id = s.file_id
                 WHERE f.repository_id = ?1 AND f.deleted_at IS NULL
                 ORDER BY s.name, f.relative_path, s.start_line LIMIT 500",
        )?;
        statement
            .query_map([&repository_id], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            })?
            .collect()
    })?;
    let mut selected_symbols = BTreeSet::new();
    for (id, name, kind, qualified, signature, start, end, path) in &symbols {
        let lower = name.to_lowercase();
        let exact = terms.contains(&lower);
        let partial = !exact
            && terms
                .iter()
                .any(|term| lower.contains(term) || term.contains(&lower));
        if !exact && !partial {
            continue;
        }
        selected_symbols.insert(id.clone());
        let recency = git_recency(Path::new(&root_path), path);
        candidates.push(Candidate {
            section: PackSection::Code,
            entity_ref: format!("symbol:{id}"),
            title: format!("{kind} {qualified}"),
            content: format!(
                "{}:{}-{}\n{}",
                path,
                start + 1,
                end + 1,
                signature.as_deref().unwrap_or(name)
            ),
            provenance: "EXTRACTED_CODE".into(),
            freshness: recency
                .clone()
                .unwrap_or_else(|| "indexed project snapshot".into()),
            source_location: format!("{path}:{}", start + 1),
            reason: format!(
                "{}{}",
                if exact {
                    "exact task symbol match"
                } else {
                    "task term matched symbol"
                },
                if recency.is_some() {
                    " + recent Git history"
                } else {
                    ""
                }
            ),
            score: (if exact { 0.86 } else { 0.7 }) + if recency.is_some() { 0.08 } else { 0.0 },
            required: false,
        });
    }

    if !selected_symbols.is_empty() {
        let edges: Vec<(String, Option<String>, String, f64)> =
            storage.with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT source_id, target_id, edge_kind, confidence FROM code_edges
                 WHERE repository_id = ?1 ORDER BY confidence DESC, id LIMIT 500",
                )?;
                statement
                    .query_map([&repository_id], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                    })?
                    .collect()
            })?;
        let neighbor_ids = edges
            .iter()
            .filter_map(|(source, target, _, _)| {
                if selected_symbols.contains(source) {
                    target.clone()
                } else if target
                    .as_ref()
                    .is_some_and(|id| selected_symbols.contains(id))
                {
                    Some(source.clone())
                } else {
                    None
                }
            })
            .collect::<BTreeSet<_>>();
        for (id, name, kind, qualified, signature, start, end, path) in &symbols {
            if !neighbor_ids.contains(id) || selected_symbols.contains(id) {
                continue;
            }
            candidates.push(Candidate {
                section: PackSection::Code,
                entity_ref: format!("symbol:{id}"),
                title: format!("{kind} {qualified}"),
                content: format!(
                    "{}:{}-{}\n{}",
                    path,
                    start + 1,
                    end + 1,
                    signature.as_deref().unwrap_or(name)
                ),
                provenance: "EXTRACTED_CODE".into(),
                freshness: git_recency(Path::new(&root_path), path)
                    .unwrap_or_else(|| "indexed project snapshot".into()),
                source_location: format!("{path}:{}", start + 1),
                reason: "one-hop structural graph neighbor".into(),
                score: 0.56,
                required: false,
            });
        }
    }

    let skill_rows: Vec<SkillRow> = storage.with_connection(|connection| {
        let mut statement = connection.prepare(
            "SELECT s.id, s.canonical_name, s.skill_markdown, s.trust_state, p.allowed,
                    i.enabled, i.pinned_hash, i.scope
             FROM skill_installations i
             JOIN skills s ON s.id = i.skill_id
             JOIN skill_agent_policies p ON p.installation_id = i.id AND p.adapter_kind = ?2
             WHERE i.task_id = ?1 OR (i.repository_id = ?3 AND i.scope = 'project') OR i.scope = 'global'
             ORDER BY s.canonical_name, s.id",
        )?;
        statement
            .query_map(params![task_id, adapter, repository_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?)))?
            .collect()
    })?;
    for (id, name, markdown, trust, allowed, enabled, hash, scope) in skill_rows {
        let entity_ref = format!("skill:{id}");
        if trust != "trusted" || !allowed || !enabled {
            exclusions.push(Excluded {
                entity_ref,
                reason: if trust == "blocked" || !enabled {
                    "skill is blocked"
                } else {
                    "skill is not allowed for selected adapter"
                }
                .into(),
            });
            continue;
        }
        let relevance = lexical_overlap(&terms, &format!("{name} {markdown}"));
        let explicit = matches!(scope.as_str(), "task" | "workflow");
        if !explicit && relevance == 0 {
            exclusions.push(Excluded {
                entity_ref,
                reason: "skill is trusted but not relevant to this task".into(),
            });
            continue;
        }
        candidates.push(Candidate {
            section: PackSection::Skills,
            entity_ref,
            title: name,
            content: markdown,
            provenance: "IMPORTED".into(),
            freshness: format!("pinned snapshot {}", &hash[..12.min(hash.len())]),
            source_location: format!("skill:{id}/SKILL.md"),
            reason: format!(
                "{} skill pinned at {}",
                if explicit {
                    "explicit task"
                } else {
                    "relevant trusted"
                },
                &hash[..12.min(hash.len())]
            ),
            score: if explicit {
                0.9
            } else {
                0.68 + (relevance.min(3) as f64 * 0.03)
            },
            required: explicit,
        });
    }

    let active_tasks: Vec<(String, String, String)> = storage.with_connection(|connection| {
        let mut statement = connection.prepare(
            "SELECT id, title, intent FROM tasks WHERE repository_id = ?1 AND id <> ?2
             AND status IN ('ready', 'active', 'review') ORDER BY updated_at DESC, id LIMIT 10",
        )?;
        statement
            .query_map(params![repository_id, task_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect()
    })?;
    for (id, title, content) in active_tasks {
        candidates.push(Candidate {
            section: PackSection::History,
            entity_ref: format!("task:{id}"),
            title,
            content,
            provenance: "HUMAN".into(),
            freshness: "active task".into(),
            source_location: format!("task:{id}"),
            reason: "other active work in this repository".into(),
            score: 0.35,
            required: false,
        });
    }
    let quarantined: Vec<(String, String, Option<String>)> =
        storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, title, body_text FROM context_items
             WHERE repository_id = ?1 AND quarantine_reason IS NOT NULL ORDER BY id",
            )?;
            statement
                .query_map([&repository_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?
                .collect()
        })?;
    for (id, title, body) in quarantined {
        if lexical_overlap(&terms, &format!("{title} {}", body.unwrap_or_default())) > 0 {
            exclusions.push(Excluded {
                entity_ref: format!("context:{id}"),
                reason: "secret quarantine".into(),
            });
        }
    }
    Ok((candidates, exclusions))
}

fn fts_pattern(terms: &BTreeSet<String>) -> String {
    terms
        .iter()
        .map(|term| format!("\"{}\"*", term.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn lexical_overlap(terms: &BTreeSet<String>, text: &str) -> usize {
    let lower = text.to_lowercase();
    terms
        .iter()
        .filter(|term| lower.contains(term.as_str()))
        .count()
}

fn git_recency(root: &Path, relative_path: &str) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["log", "-1", "--format=%cI", "--", relative_path])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let timestamp = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!timestamp.is_empty()).then(|| format!("Git commit {timestamp}"))
}

fn terms(intent: &str) -> BTreeSet<String> {
    const STOP: &[&str] = &[
        "the", "and", "for", "with", "this", "that", "from", "into", "must", "should", "build",
        "create", "make",
    ];
    intent
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .map(str::to_lowercase)
        .filter(|term| term.len() >= 3 && !STOP.contains(&term.as_str()))
        .collect()
}
