//! The tools, their schemas, and what each one reads or writes.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use mars_activity::ActivityRecord;
use mars_brain::{
    AskRequest, BrainService, EntityKind, EntityRef, KnowledgeKind, KnowledgeService, MemoryKind,
    MemoryScope, MemoryService, NewKnowledge, NewProposal, NewProvenance, ProjectBrain,
    ProvenanceKind, RetrievalContext, TranscriptSource,
};
use mars_indexer::api::{IndexApi, NeighborOptions};
use mars_indexer::search::SearchFilters;
use mars_security::{CapabilityCeiling, PermissionBroker};
use mars_storage::Storage;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::render;
use crate::{ServerConfig, ToolOutcome};

/// What the server tells the agent about itself on connection.
pub const INSTRUCTIONS: &str = "Mars has already indexed this repository: symbols, references, \
call and import graphs, and code chunks with embeddings. Ask it before listing directories or \
grepping — `search` finds code by meaning and by name at once, `find_symbol` and \
`references`/`callers`/`callees` walk the graph, `impact` says what a change would reach, and \
`ask` answers questions with citations. Results are compact on purpose; read the cited lines \
with your own file tool when you need more. `memory` returns the conventions people approved \
for this project — follow them. When you make a choice another engineer would want explained, \
call `record_decision`; when the person tells you something worth remembering across tasks, \
call `remember`. Both wait for a person to approve them.";

/// Everything a tool call can reach, opened once per server process.
pub struct ToolContext {
    pub(crate) config: ServerConfig,
    pub(crate) storage: Arc<Storage>,
    index: Arc<IndexApi>,
    brain: ProjectBrain,
    graph: BrainService,
    knowledge: KnowledgeService,
    pub(crate) broker: PermissionBroker,
    pub(crate) project_id: String,
}

impl ToolContext {
    pub fn open(config: ServerConfig) -> Result<Self, String> {
        let storage = Arc::new(
            Storage::open(&config.database)
                .map_err(|error| format!("{}: {error}", config.database.display()))?,
        );
        let project_id: String = storage
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT project_id FROM repositories WHERE id = ?1",
                        [&config.repository_id],
                        |row| row.get(0),
                    )
                    .optional()
            })
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("repository `{}` is not known to Mars", config.repository_id))?;
        let index = Arc::new(IndexApi::new(storage.clone()));
        // The app keeps spilled activity payloads beside its database.
        let payload_root = config.database.parent().map_or_else(
            || std::path::PathBuf::from("activity-payloads"),
            |data| data.join("activity-payloads"),
        );
        Ok(Self {
            brain: ProjectBrain::new(storage.clone(), index.clone()),
            graph: BrainService::new(storage.clone()),
            knowledge: KnowledgeService::new(storage.clone()),
            broker: PermissionBroker::new(storage.clone())
                .recording_to(ActivityRecord::new(storage.clone(), &payload_root)),
            index,
            storage,
            project_id,
            config,
        })
    }

    pub(crate) fn ceiling(&self) -> CapabilityCeiling {
        if self.config.role == "reviewer" {
            CapabilityCeiling::reviewer()
        } else {
            CapabilityCeiling::implementer()
        }
    }

    fn read_only(&self) -> bool {
        self.config.role == "reviewer"
    }

    /// The tools this process offers, as `tools/list` returns them.
    #[must_use]
    pub fn definitions(&self) -> Vec<Value> {
        let read = json!({ "readOnlyHint": true, "openWorldHint": false });
        let symbol = json!({
            "type": "string",
            "description": "A symbol id from an earlier result, or a name such as `parseConfig` or `Retry.run`."
        });
        let mut tools = vec![
            json!({
                "name": "search",
                "description": "Find code in this repository by meaning and by name at once (lexical and semantic, fused). Use this before grep or listing directories. Returns locations with the first lines of each match.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "What you are looking for, in words or identifiers." },
                        "path": { "type": "string", "description": "Only search under this repository-relative path prefix." },
                        "language": { "type": "string", "description": "Only this language, e.g. typescript, rust, python." },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 40, "default": 10 },
                        "snippets": { "type": "boolean", "default": true, "description": "False returns locations only, which costs fewer tokens." }
                    },
                    "required": ["query"]
                },
                "annotations": read,
            }),
            json!({
                "name": "find_symbol",
                "description": "Find where a function, class, type, method or variable is defined, by name or part of one. Returns ids to pass to the graph tools.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 15 }
                    },
                    "required": ["name"]
                },
                "annotations": read,
            }),
            json!({
                "name": "symbols_at",
                "description": "The definitions that enclose a line of a file, innermost first: which function you are in, and which class it belongs to.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Repository-relative path." },
                        "line": { "type": "integer", "minimum": 1 }
                    },
                    "required": ["path", "line"]
                },
                "annotations": read,
            }),
            json!({
                "name": "references",
                "description": "Every indexed place that refers to a symbol, with the line it happens on.",
                "inputSchema": { "type": "object", "properties": { "symbol": symbol, "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 40 } }, "required": ["symbol"] },
                "annotations": read,
            }),
            json!({
                "name": "callers",
                "description": "The functions that call a symbol. Syntax-derived: a dynamic call may be missing.",
                "inputSchema": { "type": "object", "properties": { "symbol": symbol, "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 40 } }, "required": ["symbol"] },
                "annotations": read,
            }),
            json!({
                "name": "callees",
                "description": "The functions a symbol calls.",
                "inputSchema": { "type": "object", "properties": { "symbol": symbol, "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 40 } }, "required": ["symbol"] },
                "annotations": read,
            }),
            json!({
                "name": "impact",
                "description": "What a change to a symbol could reach: everything that depends on it, transitively, to the given depth. Use before changing a widely used function.",
                "inputSchema": { "type": "object", "properties": { "symbol": symbol, "depth": { "type": "integer", "minimum": 1, "maximum": 5, "default": 2 }, "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 40 } }, "required": ["symbol"] },
                "annotations": read,
            }),
            json!({
                "name": "related",
                "description": "The neighbourhood of a symbol in both directions — what it uses and what uses it — to the given depth.",
                "inputSchema": { "type": "object", "properties": { "symbol": symbol, "depth": { "type": "integer", "minimum": 1, "maximum": 5, "default": 1 }, "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 40 } }, "required": ["symbol"] },
                "annotations": read,
            }),
            json!({
                "name": "file_deps",
                "description": "What a file imports and which files import it.",
                "inputSchema": { "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] },
                "annotations": read,
            }),
            json!({
                "name": "ask",
                "description": "Ask a question about this repository. The answer separates facts (each with a file:line citation) from inferences, and names what it could not determine. Read the cited lines rather than trusting a summary.",
                "inputSchema": { "type": "object", "properties": { "question": { "type": "string" } }, "required": ["question"] },
                "annotations": read,
            }),
            json!({
                "name": "where_is",
                "description": "Where a concept lives in the code, e.g. `authentication`, `rate limiting`, `the config loader`.",
                "inputSchema": { "type": "object", "properties": { "concept": { "type": "string" } }, "required": ["concept"] },
                "annotations": read,
            }),
            json!({
                "name": "explain",
                "description": "Explain one symbol from its definition, its callers and its callees, with citations.",
                "inputSchema": { "type": "object", "properties": { "symbol": symbol }, "required": ["symbol"] },
                "annotations": read,
            }),
            json!({
                "name": "index_status",
                "description": "How current the index is: what it covers, which commit it was built from, and whether files have changed since.",
                "inputSchema": { "type": "object", "properties": {} },
                "annotations": read,
            }),
            json!({
                "name": "memory",
                "description": "The conventions, decisions, preferences and cautions people approved for this project, most relevant first. Follow them; they outrank your own defaults.",
                "inputSchema": { "type": "object", "properties": { "query": { "type": "string", "description": "What you are about to do. Omit for the most important memories." }, "limit": { "type": "integer", "minimum": 1, "maximum": 30, "default": 10 } } },
                "annotations": read,
            }),
            json!({
                "name": "decisions",
                "description": "Decisions already recorded for this project, approved first, so you do not reopen one that was settled.",
                "inputSchema": { "type": "object", "properties": { "query": { "type": "string" }, "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 15 } } },
                "annotations": read,
            }),
        ];
        if !self.read_only() {
            tools.push(json!({
                "name": "record_decision",
                "description": "Record a decision you made that another engineer would want explained: an approach chosen over alternatives, a trade-off, a constraint you worked around. It is shown to the person for approval and, once approved, explains this code to future agents.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "A short name, e.g. `Retry with jittered backoff`." },
                        "decision": { "type": "string", "description": "What you decided." },
                        "rationale": { "type": "string", "description": "Why — the evidence and the trade-off." },
                        "alternatives": { "type": "array", "items": { "type": "string" }, "description": "What you considered and did not do." },
                        "files": { "type": "array", "items": { "type": "string" }, "description": "Repository-relative paths the decision shapes." },
                        "confidence": { "type": "number", "minimum": 0, "maximum": 1, "default": 0.7 }
                    },
                    "required": ["title", "decision", "rationale"]
                },
                "annotations": { "readOnlyHint": false, "destructiveHint": false, "openWorldHint": false },
            }));
            tools.push(json!({
                "name": "remember",
                "description": "Propose something for Mars to remember across tasks: a convention, preference, fact or caution the person stated or confirmed. It is queued for the person to approve and is never used until they do. Quote their exact words.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "text": { "type": "string", "description": "The memory, as an instruction to a future agent." },
                        "kind": { "type": "string", "enum": ["convention", "decision", "preference", "fact", "caution"] },
                        "scope": { "type": "string", "enum": ["repo", "task", "global"], "default": "repo" },
                        "quote": { "type": "string", "description": "The exact words from this conversation that motivated it." },
                        "confidence": { "type": "number", "minimum": 0, "maximum": 1, "default": 0.6 }
                    },
                    "required": ["text", "kind", "quote"]
                },
                "annotations": { "readOnlyHint": false, "destructiveHint": false, "openWorldHint": false },
            }));
        }
        if self.config.permission_tool {
            tools.push(json!({
                "name": "permission_prompt",
                "description": "Asked by the agent CLI before an action its allow-list does not cover. Not for direct use.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "tool_name": { "type": "string" },
                        "input": { "type": "object" },
                        "tool_use_id": { "type": "string" }
                    },
                    "required": ["tool_name", "input"]
                },
            }));
        }
        tools
    }

    /// Run one tool. Errors are reported to the agent as tool errors, never
    /// as protocol failures, so it can read them and adjust.
    pub fn call(&self, name: &str, arguments: &Value, cancelled: &AtomicBool) -> ToolOutcome {
        let result = match name {
            "search" => self.search(arguments),
            "find_symbol" => self.find_symbol(arguments),
            "symbols_at" => self.symbols_at(arguments),
            "references" | "callers" | "callees" => self.edges(name, arguments),
            "impact" | "related" => self.neighbourhood(name, arguments),
            "file_deps" => self.file_deps(arguments),
            "ask" | "where_is" | "explain" => self.answer(name, arguments),
            "index_status" => self.index_status(),
            "memory" => self.memory(arguments),
            "decisions" => self.decisions(arguments),
            "record_decision" if !self.read_only() => self.record_decision(arguments),
            "remember" if !self.read_only() => self.remember(arguments),
            "permission_prompt" if self.config.permission_tool => {
                return crate::permission::decide(self, arguments, cancelled);
            }
            other => Err(format!("there is no tool named `{other}` here")),
        };
        match result {
            Ok(text) => ToolOutcome::ok(text),
            Err(message) => ToolOutcome::error(message),
        }
    }

    fn repository(&self) -> &str {
        &self.config.repository_id
    }

    /// Say so plainly when there is no index to ask, instead of returning an
    /// empty result the agent would read as "nothing matches".
    fn require_index(&self) -> Result<(), String> {
        let status = self
            .index
            .status(self.repository())
            .map_err(|error| error.to_string())?;
        if status.counts.files == 0 {
            return Err(
                "This repository has not been indexed yet, so Mars cannot answer from \
                the graph. Read files directly for now; the person can index it from Mars."
                    .into(),
            );
        }
        Ok(())
    }

    fn search(&self, arguments: &Value) -> Result<String, String> {
        self.require_index()?;
        let query = required(arguments, "query")?;
        let limit = number(arguments, "limit", 10, 1, 40);
        let filters = SearchFilters {
            path_prefixes: optional(arguments, "path")
                .map(|path| vec![path.trim_start_matches("./").replace('\\', "/")])
                .unwrap_or_default(),
            languages: optional(arguments, "language")
                .map(|language| vec![language.to_ascii_lowercase()])
                .unwrap_or_default(),
        };
        let results = self
            .index
            .search_hybrid(self.repository(), &query, limit, &filters)
            .map_err(|error| error.to_string())?;
        let snippets = arguments
            .get("snippets")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        Ok(render::search(&results, snippets))
    }

    fn find_symbol(&self, arguments: &Value) -> Result<String, String> {
        self.require_index()?;
        let name = required(arguments, "name")?;
        let limit = number(arguments, "limit", 15, 1, 50);
        let symbols = self
            .graph
            .find_symbol(self.repository(), &name, limit)
            .map_err(|error| error.to_string())?;
        Ok(render::symbols(&name, &symbols))
    }

    fn symbols_at(&self, arguments: &Value) -> Result<String, String> {
        self.require_index()?;
        let path = self.relative(&required(arguments, "path")?);
        let line = number(arguments, "line", 1, 1, usize::MAX);
        let symbols = self
            .index
            .symbols_at(self.repository(), &path, line)
            .map_err(|error| error.to_string())?;
        Ok(render::enclosing(&path, line, &symbols))
    }

    /// A symbol argument is an id or a name. A name is resolved to the one
    /// definition it names; several matches are listed so the agent can pick.
    fn resolve_symbol(&self, arguments: &Value) -> Result<(String, String), String> {
        let given = required(arguments, "symbol")?;
        let by_id: Option<String> = self
            .storage
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT s.name FROM symbols s JOIN files f ON f.id = s.file_id
                         WHERE s.id = ?1 AND f.repository_id = ?2",
                        params![given, self.repository()],
                        |row| row.get(0),
                    )
                    .optional()
            })
            .map_err(|error| error.to_string())?;
        if let Some(name) = by_id {
            return Ok((given, name));
        }
        let candidates = self
            .graph
            .find_symbol(self.repository(), &given, 20)
            .map_err(|error| error.to_string())?;
        let short = given.rsplit(['.', ':']).next().unwrap_or(&given);
        let exact: Vec<_> = candidates
            .iter()
            .filter(|symbol| {
                symbol.name == given || symbol.qualified_name == given || symbol.name == short
            })
            .collect();
        match exact.as_slice() {
            [one] => Ok((one.id.clone(), one.name.clone())),
            [] if candidates.is_empty() => Err(format!(
                "No symbol named `{given}` is indexed. Try `search` with a description instead."
            )),
            [] => Err(format!(
                "No symbol is named exactly `{given}`. Close matches:\n{}",
                render::symbols(&given, &candidates)
            )),
            several => Err(format!(
                "`{given}` names {} symbols. Pass one of these ids:\n{}",
                several.len(),
                render::symbols(
                    &given,
                    &several
                        .iter()
                        .map(|symbol| (*symbol).clone())
                        .collect::<Vec<_>>()
                )
            )),
        }
    }

    fn edges(&self, kind: &str, arguments: &Value) -> Result<String, String> {
        self.require_index()?;
        let (symbol_id, name) = self.resolve_symbol(arguments)?;
        let limit = number(arguments, "limit", 40, 1, 100);
        let edges = match kind {
            "references" => self.graph.references(self.repository(), &symbol_id, limit),
            "callers" => self.graph.callers(self.repository(), &symbol_id, limit),
            _ => self.graph.callees(self.repository(), &symbol_id, limit),
        }
        .map_err(|error| error.to_string())?;
        Ok(render::edges(kind, &name, &edges, limit))
    }

    fn neighbourhood(&self, kind: &str, arguments: &Value) -> Result<String, String> {
        self.require_index()?;
        let (symbol_id, _) = self.resolve_symbol(arguments)?;
        let depth = number(
            arguments,
            "depth",
            if kind == "impact" { 2 } else { 1 },
            1,
            5,
        );
        let limit = number(arguments, "limit", 40, 1, 100);
        if kind == "impact" {
            let neighbourhood = self
                .graph
                .blast_radius(self.repository(), &symbol_id, depth, limit)
                .map_err(|error| error.to_string())?;
            return Ok(render::neighbourhood("impact", &neighbourhood, depth));
        }
        let neighbourhood = self
            .index
            .graph_neighbors(
                self.repository(),
                &symbol_id,
                &NeighborOptions {
                    depth,
                    limit,
                    ..NeighborOptions::default()
                },
            )
            .map_err(|error| error.to_string())?;
        Ok(render::graph(&neighbourhood))
    }

    fn file_deps(&self, arguments: &Value) -> Result<String, String> {
        self.require_index()?;
        let path = self.relative(&required(arguments, "path")?);
        let dependencies = self
            .index
            .file_dependencies(self.repository(), &path)
            .map_err(|error| error.to_string())?;
        Ok(render::file_dependencies(&dependencies))
    }

    fn answer(&self, kind: &str, arguments: &Value) -> Result<String, String> {
        self.require_index()?;
        let cancellation = CancellationToken::new();
        let answer = match kind {
            "ask" => self.brain.ask(
                &AskRequest::new(self.repository(), required(arguments, "question")?),
                &cancellation,
            ),
            "where_is" => self.brain.where_is(
                self.repository(),
                &required(arguments, "concept")?,
                &cancellation,
            ),
            _ => {
                let (symbol_id, _) = self.resolve_symbol(arguments)?;
                self.brain
                    .explain(self.repository(), &symbol_id, &cancellation)
            }
        }
        .map_err(|error| error.to_string())?;
        Ok(render::answer(&answer))
    }

    fn index_status(&self) -> Result<String, String> {
        let status = self
            .index
            .status(self.repository())
            .map_err(|error| error.to_string())?;
        let stale = self
            .index
            .stale_paths(self.repository())
            .map(|paths| paths.len())
            .unwrap_or(0);
        Ok(render::index_status(&status, stale))
    }

    fn memory(&self, arguments: &Value) -> Result<String, String> {
        let query = optional(arguments, "query").unwrap_or_else(|| self.task_intent());
        let limit = number(arguments, "limit", 10, 1, 30);
        let memories = MemoryService::new(self.storage.clone())
            .for_retrieval(&RetrievalContext {
                project_id: self.project_id.clone(),
                repository_id: Some(self.config.repository_id.clone()),
                task_id: self.config.task_id.clone(),
                session_id: None,
                query,
                // Calling this is the injection (ADR-036), so it is recorded.
                surface: "mcp".into(),
                limit: Some(limit),
                record_uses: true,
            })
            .map_err(|error| error.to_string())?;
        Ok(render::memories(&memories))
    }

    fn decisions(&self, arguments: &Value) -> Result<String, String> {
        let query = optional(arguments, "query").map(|query| query.to_lowercase());
        let limit = number(arguments, "limit", 15, 1, 50);
        let mut records: Vec<_> = self
            .knowledge
            .list(&self.project_id, false)
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|record| record.kind == KnowledgeKind::Decision)
            .filter(|record| {
                query.as_deref().is_none_or(|query| {
                    query.split_whitespace().any(|word| {
                        record.title.to_lowercase().contains(word)
                            || record.body.to_lowercase().contains(word)
                    })
                })
            })
            .collect();
        // Approved first: a decision a person accepted outranks a candidate.
        records.sort_by_key(|record| {
            (
                record.status != mars_brain::KnowledgeStatus::Approved,
                std::cmp::Reverse(record.created_at.clone()),
            )
        });
        records.truncate(limit);
        Ok(render::decisions(&records))
    }

    fn record_decision(&self, arguments: &Value) -> Result<String, String> {
        let title = required(arguments, "title")?;
        let decision = required(arguments, "decision")?;
        let rationale = required(arguments, "rationale")?;
        let alternatives: Vec<String> = strings(arguments, "alternatives");
        let files: Vec<String> = strings(arguments, "files")
            .into_iter()
            .map(|path| self.relative(&path))
            .collect();
        let confidence = arguments
            .get("confidence")
            .and_then(Value::as_f64)
            .unwrap_or(0.7)
            .clamp(0.0, 1.0);
        let Some(task_id) = self.config.task_id.clone() else {
            return Err("Decisions are recorded against a task, and this session has none.".into());
        };

        let mut body = format!("{decision}\n\nWhy: {rationale}");
        if !alternatives.is_empty() {
            body.push_str("\n\nConsidered and not chosen:");
            for alternative in &alternatives {
                body.push_str(&format!("\n- {alternative}"));
            }
        }
        let provenance = self
            .knowledge
            .create_provenance(NewProvenance {
                project_id: self.project_id.clone(),
                kind: ProvenanceKind::AgentInferred,
                source: EntityRef::new(EntityKind::Task, &task_id),
                evidence: json!({
                    "via": "mcp",
                    "agent": self.config.agent,
                    "runId": self.config.run_id,
                    "rationale": rationale,
                    "alternatives": alternatives,
                    "files": files,
                }),
                observed_at: now(&self.storage)?,
            })
            .map_err(|error| error.to_string())?;
        let record = self
            .knowledge
            .create_candidate(NewKnowledge {
                project_id: self.project_id.clone(),
                kind: KnowledgeKind::Decision,
                title: title.clone(),
                body,
                confidence,
                provenance_id: provenance.id.clone(),
            })
            .map_err(|error| error.to_string())?;
        let _ = self.knowledge.link(
            &self.project_id,
            EntityRef::new(EntityKind::Knowledge, &record.id),
            "DECIDED_IN",
            EntityRef::new(EntityKind::Task, &task_id),
            &provenance.id,
            confidence,
        );
        let mut linked = 0;
        for path in &files {
            if let Some(file_id) = self.file_id(path)?
                && self
                    .knowledge
                    .link(
                        &self.project_id,
                        EntityRef::new(EntityKind::Knowledge, &record.id),
                        "AFFECTS",
                        EntityRef::new(EntityKind::File, file_id),
                        &provenance.id,
                        confidence,
                    )
                    .is_ok()
            {
                linked += 1;
            }
        }
        Ok(format!(
            "Recorded decision `{title}` for the person to review ({}). It is not authoritative until they approve it.{}",
            record.id,
            if files.is_empty() {
                String::new()
            } else {
                format!(" Linked to {linked} of {} file(s).", files.len())
            }
        ))
    }

    fn remember(&self, arguments: &Value) -> Result<String, String> {
        let text = required(arguments, "text")?;
        let quote = required(arguments, "quote")?;
        let kind = match required(arguments, "kind")?.as_str() {
            "convention" => MemoryKind::Convention,
            "decision" => MemoryKind::Decision,
            "preference" => MemoryKind::Preference,
            "fact" => MemoryKind::Fact,
            "caution" => MemoryKind::Caution,
            other => return Err(format!("`{other}` is not a kind of memory")),
        };
        let scope = match optional(arguments, "scope").as_deref().unwrap_or("repo") {
            "repo" => MemoryScope::Repo,
            "task" => MemoryScope::Task,
            "global" => MemoryScope::Global,
            other => return Err(format!("`{other}` is not a memory scope")),
        };
        let confidence = arguments
            .get("confidence")
            .and_then(Value::as_f64)
            .unwrap_or(0.6)
            .clamp(0.0, 1.0);
        let Some(task_id) = self.config.task_id.clone() else {
            return Err(
                "Memories are proposed from a task's conversation, and this session has none."
                    .into(),
            );
        };

        // The quote is checked against the task's conversation. Found, the
        // proposal points at the exact turn; not found, it is recorded as
        // unverified so the person sees that before approving.
        let transcript = ActivityTranscript {
            storage: self.storage.clone(),
        };
        let found = transcript.find(&task_id, &quote);
        let proposal = NewProposal {
            project_id: self.project_id.clone(),
            repository_id: Some(self.config.repository_id.clone()),
            task_id: Some(task_id.clone()),
            scope,
            kind,
            text: text.clone(),
            confidence,
            conversation_id: task_id.clone(),
            message_id: found
                .map(|sequence| sequence.to_string())
                .unwrap_or_else(|| {
                    format!("run:{}", self.config.run_id.as_deref().unwrap_or("unknown"))
                }),
            quoted_text: quote,
            proposed_by: self.config.agent.clone(),
        };
        let service = if found.is_some() {
            MemoryService::with_transcript(self.storage.clone(), Arc::new(transcript))
        } else {
            MemoryService::new(self.storage.clone())
        };
        let proposed = service
            .propose(proposal)
            .map_err(|error| error.to_string())?;
        Ok(format!(
            "Queued for the person to approve ({}). It will not be used until they do.{}",
            proposed.id,
            if found.is_some() {
                ""
            } else {
                " The quote was not found in this task's conversation, so it is marked unverified."
            }
        ))
    }

    fn task_intent(&self) -> String {
        let Some(task_id) = &self.config.task_id else {
            return String::new();
        };
        self.storage
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT title || ' ' || intent FROM tasks WHERE id = ?1",
                        [task_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
            })
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    fn file_id(&self, relative_path: &str) -> Result<Option<String>, String> {
        self.storage
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT id FROM files WHERE repository_id = ?1 AND relative_path = ?2
                         AND deleted_at IS NULL",
                        params![self.repository(), relative_path],
                        |row| row.get(0),
                    )
                    .optional()
            })
            .map_err(|error| error.to_string())
    }

    /// A path as the index knows it: relative to the repository, with forward
    /// slashes. Agents report absolute worktree paths as often as relative
    /// ones.
    pub(crate) fn relative(&self, path: &str) -> String {
        let normalized = path.replace('\\', "/");
        if let Some(worktree) = &self.config.worktree {
            let root = worktree.to_string_lossy().replace('\\', "/");
            let root = root.trim_end_matches('/');
            if let Some(rest) = strip_prefix_ignore_case(&normalized, root) {
                return rest.trim_start_matches('/').to_owned();
            }
        }
        normalized.trim_start_matches("./").to_owned()
    }
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    if cfg!(windows) {
        let head = text.get(..prefix.len())?;
        head.eq_ignore_ascii_case(prefix)
            .then(|| &text[prefix.len()..])
            .filter(|rest| rest.is_empty() || rest.starts_with('/'))
    } else {
        text.strip_prefix(prefix)
            .filter(|rest| rest.is_empty() || rest.starts_with('/'))
    }
}

/// A task's conversation, as the memory service verifies quotes against: the
/// conversation is the task and a message is one event's sequence number.
struct ActivityTranscript {
    storage: Arc<Storage>,
}

impl ActivityTranscript {
    /// The most recent message in the task that contains `quote`.
    fn find(&self, task_id: &str, quote: &str) -> Option<i64> {
        let needle = quote.trim();
        if needle.is_empty() {
            return None;
        }
        let rows: Vec<(i64, String)> = self
            .storage
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT sequence, payload_json FROM activity_events
                     WHERE task_id = ?1 AND kind = 'message'
                     ORDER BY sequence DESC LIMIT 2000",
                )?;
                statement
                    .query_map([task_id], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect()
            })
            .ok()?;
        rows.into_iter()
            .find(|(_, payload)| message_text(payload).contains(needle))
            .map(|(sequence, _)| sequence)
    }
}

impl TranscriptSource for ActivityTranscript {
    fn message_text(&self, conversation_id: &str, message_id: &str) -> Option<String> {
        let sequence: i64 = message_id.parse().ok()?;
        self.storage
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT payload_json FROM activity_events
                         WHERE task_id = ?1 AND sequence = ?2",
                        params![conversation_id, sequence],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
            })
            .ok()
            .flatten()
            .map(|payload| message_text(&payload))
    }
}

fn message_text(payload: &str) -> String {
    serde_json::from_str::<Value>(payload)
        .ok()
        .and_then(|value| value.get("text").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default()
}

pub(crate) fn now(storage: &Storage) -> Result<String, String> {
    storage
        .with_connection(|connection| {
            connection.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
                row.get::<_, String>(0)
            })
        })
        .map_err(|error| error.to_string())
}

fn required(arguments: &Value, name: &str) -> Result<String, String> {
    optional(arguments, name).ok_or_else(|| format!("`{name}` is required"))
}

fn optional(arguments: &Value, name: &str) -> Option<String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn strings(arguments: &Value, name: &str) -> Vec<String> {
    arguments
        .get(name)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn number(arguments: &Value, name: &str, default: usize, minimum: usize, maximum: usize) -> usize {
    arguments
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(default)
        .clamp(minimum, maximum)
}
