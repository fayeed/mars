//! Project Brain: the question-answering pipeline.
//!
//! Seven stages, in order: rewrite the query, retrieve from the hybrid index,
//! expand through the code graph around the best hits, rerank against the
//! question, pack a token budget, synthesise claims, and validate every
//! citation against the working tree.
//!
//! Validation runs last, as it must — evidence can only be checked once it has
//! been cited — but claims are streamed *during* it rather than before it. A
//! surface that renders progressively must never show a claim it will shortly
//! retract, so each claim is emitted at the moment its evidence is confirmed,
//! and a claim whose evidence has vanished is never emitted at all.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use mars_indexer::IndexError;
use mars_indexer::api::{Direction, IndexApi, NeighborOptions};
use mars_storage::{Storage, StorageError};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::answer::{
    AnswerInvalid, AnswerSource, BrainAnswer, Citation, Claim, ClaimKind, Gap, GapKind, IdAllocator,
};
use crate::budget::{self, DEFAULT_BUDGET_TOKENS, MAX_BUDGET_TOKENS, MIN_BUDGET_TOKENS};
use crate::gather;
use crate::rewrite::{self, QueryIntent, QueryPlan};
use crate::synthesis::{
    DeterministicSynthesizer, SYNTHESIS_INSTRUCTIONS, SynthesisOutput, SynthesisProvider,
    SynthesisRequest, SynthesisSource,
};
use crate::validate;

/// Default ceiling on packed sources. Beyond roughly this many, an answer
/// stops being an answer and becomes a search results page.
pub const DEFAULT_MAX_SOURCES: usize = 24;

/// Lines of a chunk carried as a citation snippet. Enough to recognise the
/// code and to re-find it after an edit, short enough that an answer document
/// stays readable.
const SNIPPET_LINES: usize = 12;

#[derive(Clone, Debug)]
pub struct AskRequest {
    pub repository_id: String,
    pub question: String,
    /// Token ceiling for the synthesis context.
    pub budget_tokens: usize,
    pub max_sources: usize,
}

impl AskRequest {
    #[must_use]
    pub fn new(repository_id: impl Into<String>, question: impl Into<String>) -> Self {
        Self {
            repository_id: repository_id.into(),
            question: question.into(),
            budget_tokens: DEFAULT_BUDGET_TOKENS,
            max_sources: DEFAULT_MAX_SOURCES,
        }
    }

    #[must_use]
    pub fn with_budget(mut self, budget_tokens: usize) -> Self {
        self.budget_tokens = budget_tokens;
        self
    }

    #[must_use]
    pub fn with_max_sources(mut self, max_sources: usize) -> Self {
        self.max_sources = max_sources;
        self
    }
}

/// Progress from a running answer. A consumer can render claims as they are
/// confirmed instead of waiting for the whole pipeline.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum BrainEvent {
    Planned {
        queries: Vec<String>,
        intent: QueryIntent,
    },
    Retrieved {
        candidates: usize,
        stale_paths: usize,
    },
    Packed {
        sources: usize,
        tokens: usize,
        dropped: usize,
    },
    Synthesized {
        provider: String,
        claims: usize,
    },
    /// Emitted before any claim that cites it.
    Citation(Citation),
    /// Emitted only once its evidence has been confirmed against the tree.
    Claim(Claim),
    Gap(Gap),
    Completed {
        sources: usize,
        claims: usize,
        latency_ms: u64,
        confidence: f64,
    },
    Cancelled {
        stage: String,
    },
}

#[derive(Debug, Error)]
pub enum AskError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Index(#[from] IndexError),
    #[error("repository `{0}` is not open")]
    RepositoryNotFound(String),
    #[error("a question needs at least one word")]
    EmptyQuestion,
    #[error("symbol `{0}` does not exist in this repository")]
    SymbolNotFound(String),
    #[error("`{0}` matches no indexed file or symbol")]
    TargetNotFound(String),
    #[error(
        "context budget must be between {MIN_BUDGET_TOKENS} and {MAX_BUDGET_TOKENS} tokens, not {0}"
    )]
    InvalidBudget(usize),
    #[error("synthesis provider failed: {0}")]
    Provider(String),
    #[error("synthesis output does not match the claim schema: {0}")]
    InvalidOutput(String),
    #[error("claim `{claim}` cites `{source_id}`, which was not among the supplied sources")]
    UnknownSource { claim: String, source_id: String },
    #[error("claim `{claim}` reasons over `{support}`, which is not a claim in this answer")]
    UnknownSupport { claim: String, support: String },
    #[error("duplicate claim id `{0}` in synthesis output")]
    DuplicateClaim(String),
    #[error("synthesis produced an uncited fact: {0}")]
    Contract(#[from] AnswerInvalid),
    #[error("the question was cancelled during {stage}")]
    Cancelled { stage: String },
}

/// Where a dependency query was pointed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DependencyTarget {
    File,
    Symbol,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub path: String,
    pub line_start: usize,
    pub line_end: usize,
    /// Hops from the queried target; 0 is the target itself.
    pub distance: usize,
    /// False for a node that names something outside the repository, such as
    /// an external package, which has no source to open.
    pub resolved: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyEdge {
    pub kind: String,
    pub source: String,
    pub target: String,
    pub confidence: f64,
    /// Direction relative to the queried target: what it depends on, or what
    /// depends on it.
    pub incoming: bool,
}

/// A renderable slice of the dependency graph. Deliberately not an answer:
/// this is structure to draw, and inventing prose about it would only add an
/// uncited layer over data the viewer can already see.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencySlice {
    pub target: String,
    pub target_kind: DependencyTarget,
    pub nodes: Vec<DependencyNode>,
    pub edges: Vec<DependencyEdge>,
    pub citations: Vec<Citation>,
    pub gaps: Vec<Gap>,
    pub truncated: bool,
    pub latency_ms: u64,
}

/// The retrieval-and-evidence layer over the Phase 0 index.
pub struct ProjectBrain {
    storage: Arc<Storage>,
    index: Arc<IndexApi>,
    provider: Arc<dyn SynthesisProvider>,
}

impl ProjectBrain {
    /// Builds a Brain with no synthesis model, which answers with citation-
    /// backed structural facts and declares the absence of reasoning as a gap.
    #[must_use]
    pub fn new(storage: Arc<Storage>, index: Arc<IndexApi>) -> Self {
        Self::with_provider(storage, index, Arc::new(DeterministicSynthesizer))
    }

    #[must_use]
    pub fn with_provider(
        storage: Arc<Storage>,
        index: Arc<IndexApi>,
        provider: Arc<dyn SynthesisProvider>,
    ) -> Self {
        Self {
            storage,
            index,
            provider,
        }
    }

    /// `brain.ask(question)`.
    pub fn ask(
        &self,
        request: &AskRequest,
        cancellation: &CancellationToken,
    ) -> Result<BrainAnswer, AskError> {
        self.ask_observing(request, cancellation, |_| {})
    }

    /// `brain.ask(question)`, streaming claims as their evidence is confirmed.
    pub fn ask_observing(
        &self,
        request: &AskRequest,
        cancellation: &CancellationToken,
        observer: impl FnMut(&BrainEvent),
    ) -> Result<BrainAnswer, AskError> {
        if request.question.trim().is_empty() {
            return Err(AskError::EmptyQuestion);
        }
        let plan = rewrite::plan(&request.question);
        self.run(request, plan, &[], cancellation, observer)
    }

    /// `brain.explain(symbolId)` — what one definition is and does.
    pub fn explain(
        &self,
        repository_id: &str,
        symbol_id: &str,
        cancellation: &CancellationToken,
    ) -> Result<BrainAnswer, AskError> {
        self.explain_observing(repository_id, symbol_id, cancellation, |_| {})
    }

    pub fn explain_observing(
        &self,
        repository_id: &str,
        symbol_id: &str,
        cancellation: &CancellationToken,
        observer: impl FnMut(&BrainEvent),
    ) -> Result<BrainAnswer, AskError> {
        let symbol = self
            .symbol(repository_id, symbol_id)?
            .ok_or_else(|| AskError::SymbolNotFound(symbol_id.to_owned()))?;
        let question = format!(
            "What is {} in {}, and what does it do?",
            symbol.qualified_name.as_deref().unwrap_or(&symbol.name),
            symbol.relative_path
        );
        let request = AskRequest::new(repository_id, question);
        // The plan is built from the symbol's own name rather than the
        // generated sentence, so retrieval looks for the definition rather than
        // for the words of the question template.
        let plan = rewrite::plan(&format!(
            "{} {} {}",
            symbol.name,
            symbol.qualified_name.as_deref().unwrap_or_default(),
            symbol.relative_path
        ));
        self.run(
            &request,
            plan,
            std::slice::from_ref(&symbol.symbol_id),
            cancellation,
            observer,
        )
    }

    /// `brain.whereIsX(concept)` — where a concept lives in the repository.
    pub fn where_is(
        &self,
        repository_id: &str,
        concept: &str,
        cancellation: &CancellationToken,
    ) -> Result<BrainAnswer, AskError> {
        self.where_is_observing(repository_id, concept, cancellation, |_| {})
    }

    pub fn where_is_observing(
        &self,
        repository_id: &str,
        concept: &str,
        cancellation: &CancellationToken,
        observer: impl FnMut(&BrainEvent),
    ) -> Result<BrainAnswer, AskError> {
        if concept.trim().is_empty() {
            return Err(AskError::EmptyQuestion);
        }
        let request = AskRequest::new(
            repository_id,
            format!("Where is {} in this repository?", concept.trim()),
        );
        // Locate intent is asserted rather than inferred: the caller already
        // said this is a "where" question by choosing this entry point.
        let mut plan = rewrite::plan(concept);
        plan.intent = QueryIntent::Locate;
        plan.original.clone_from(&request.question);
        self.run(&request, plan, &[], cancellation, observer)
    }

    /// `brain.dependencies(target)` — a graph slice to render.
    ///
    /// `target` is a repository-relative file path or a symbol id.
    pub fn dependencies(
        &self,
        repository_id: &str,
        target: &str,
        cancellation: &CancellationToken,
    ) -> Result<DependencySlice, AskError> {
        let started = Instant::now();
        let root = self.repository_root(repository_id)?;
        let target = target.replace('\\', "/");

        if let Some(symbol) = self.symbol(repository_id, &target)? {
            return self.symbol_dependencies(repository_id, &root, &symbol, started, cancellation);
        }
        if self.file_exists(repository_id, &target)? {
            return self.file_dependencies(repository_id, &target, started);
        }
        Err(AskError::TargetNotFound(target))
    }

    /// The whole pipeline. Every stage checks cancellation before starting, so
    /// a cancelled question stops at a stage boundary rather than part-way
    /// through emitting a half-built answer.
    fn run(
        &self,
        request: &AskRequest,
        plan: QueryPlan,
        seed_symbols: &[String],
        cancellation: &CancellationToken,
        mut observer: impl FnMut(&BrainEvent),
    ) -> Result<BrainAnswer, AskError> {
        if !(MIN_BUDGET_TOKENS..=MAX_BUDGET_TOKENS).contains(&request.budget_tokens) {
            return Err(AskError::InvalidBudget(request.budget_tokens));
        }
        let started = Instant::now();
        let root = self.repository_root(&request.repository_id)?;
        cancelled(cancellation, "planning")?;

        observer(&BrainEvent::Planned {
            queries: plan.query_texts(),
            intent: plan.intent,
        });

        cancelled(cancellation, "retrieval")?;
        let gathered = gather::gather(
            &self.index,
            &self.storage,
            &request.repository_id,
            &plan,
            seed_symbols,
            cancellation,
        )?;
        cancelled(cancellation, "retrieval")?;
        observer(&BrainEvent::Retrieved {
            candidates: gathered.candidates.len(),
            stale_paths: gathered.stale_paths.len(),
        });

        let retrieved = gathered.candidates.len();
        let packed = budget::pack(
            gathered.candidates,
            request.budget_tokens,
            request.max_sources,
        );
        observer(&BrainEvent::Packed {
            sources: packed.sources.len(),
            tokens: packed.tokens,
            dropped: packed.dropped.len(),
        });
        cancelled(cancellation, "synthesis")?;

        // Source ids are positional and opaque to the provider; the mapping
        // back to real citations never leaves this function.
        let mut citation_ids = IdAllocator::new("cite");
        let mut sources = Vec::new();
        let mut citations = Vec::new();
        let mut source_detail = Vec::new();
        let mut source_to_citation: HashMap<String, String> = HashMap::new();

        for (ordinal, source) in packed.sources.iter().enumerate() {
            let candidate = &source.candidate;
            let source_id = format!("src:{ordinal}");
            let citation_id = citation_ids.id_for(&candidate.chunk_id);
            source_to_citation.insert(source_id.clone(), citation_id.clone());
            // A chunk's span is a byte range, so it can open on a blank line.
            // Citing that line would point a reader at nothing, and would make
            // the validation pass "relabel" a file nobody has touched, so the
            // citation starts where the content does.
            let (snippet, offset) = snippet(&candidate.body);
            let line_start = candidate.line_start + offset;
            let line_end = candidate.line_end.max(line_start);
            sources.push(SynthesisSource {
                id: source_id,
                path: candidate.path.clone(),
                symbol_path: candidate.symbol_path.clone(),
                language: candidate.language.clone(),
                line_start,
                line_end,
                content: candidate.body.clone(),
                stale: candidate.stale,
            });
            citations.push(Citation {
                id: citation_id,
                path: candidate.path.clone(),
                line_start,
                line_end,
                blob_sha: candidate.blob_sha.clone(),
                snippet,
                why: candidate.why.clone(),
            });
            source_detail.push(AnswerSource {
                chunk_id: candidate.chunk_id.clone(),
                path: candidate.path.clone(),
                symbol_path: candidate.symbol_path.clone(),
                line_start,
                line_end,
                blob_sha: candidate.blob_sha.clone(),
                score: (candidate.score * 1000.0).round() / 1000.0,
                tokens: source.tokens,
                signals: candidate.signals.clone(),
                stale: candidate.stale,
            });
        }

        let synthesis_request = SynthesisRequest {
            instructions: SYNTHESIS_INSTRUCTIONS.to_owned(),
            question: request.question.clone(),
            sources,
        };
        let raw = self
            .provider
            .synthesize(&synthesis_request)
            .map_err(AskError::Provider)?;
        let output = serde_json::from_str::<SynthesisOutput>(&raw)
            .map_err(|error| AskError::InvalidOutput(error.to_string()))?;
        observer(&BrainEvent::Synthesized {
            provider: self.provider.name().to_owned(),
            claims: output.claims.len(),
        });
        cancelled(cancellation, "validation")?;

        let (claims, gaps) = self.adopt(output, &source_to_citation)?;

        let mut answer = BrainAnswer {
            question: request.question.clone(),
            rewritten_queries: plan.query_texts(),
            claims,
            citations,
            sources: packed.sources.len(),
            retrieved,
            latency_ms: 0,
            confidence: 0.0,
            gaps,
            source_detail,
            truncated: packed.truncated(),
        };

        // Semantic search always returns its nearest neighbours, however far
        // away they are, so "retrieval returned rows" is not the same as
        // "retrieval found the subject". When nothing packed shares a term or
        // an identifier with the question, the sources are neighbours rather
        // than matches, and saying so is the difference between an answer and
        // a plausible-looking accident.
        let nothing_matched = answer.source_detail.iter().all(|source| {
            source
                .signals
                .iter()
                .filter(|signal| signal.name == "termOverlap" || signal.name == "identifierMatch")
                .all(|signal| signal.value <= 0.0)
        });
        if gathered.empty || nothing_matched {
            answer.gaps.push(Gap {
                kind: GapKind::NotIndexed,
                subject: request.question.trim().to_owned(),
                detail: if gathered.empty {
                    "No indexed chunk matched any rewriting of this question. The repository \
                     may not be indexed, or the subject may live in a file the ignore rules \
                     exclude."
                        .to_owned()
                } else {
                    "No indexed chunk shares a term with this question. The sources below are \
                     the nearest neighbours in the semantic index, not matches, and nothing \
                     here should be read as an answer to what was asked."
                        .to_owned()
                },
            });
        }
        if packed.truncated() {
            let example = packed
                .dropped
                .first()
                .map(|dropped| {
                    format!(
                        " For example, {} in {} was left out because {}.",
                        if dropped.symbol_path.is_empty() {
                            "a chunk"
                        } else {
                            dropped.symbol_path.as_str()
                        },
                        dropped.path,
                        dropped.reason
                    )
                })
                .unwrap_or_default();
            answer.gaps.push(Gap {
                kind: GapKind::BudgetExhausted,
                subject: format!("{} further retrieved source(s)", packed.dropped.len()),
                detail: format!(
                    "Retrieval found more material than the {}-token context budget could \
                     carry whole.{example}",
                    request.budget_tokens
                ),
            });
        }

        // Confirm every citation against the working tree, then stream what
        // survived. Nothing reaches the observer that validation later removes.
        let current_sha = self.current_shas(&request.repository_id, &answer)?;
        validate::resolve_citations(&mut answer, &root, &current_sha);
        answer.prune_orphan_citations();
        answer.recompute_confidence();
        answer.latency_ms = started.elapsed().as_millis() as u64;

        answer.validate()?;

        for citation in &answer.citations {
            observer(&BrainEvent::Citation(citation.clone()));
        }
        for claim in &answer.claims {
            observer(&BrainEvent::Claim(claim.clone()));
        }
        for gap in &answer.gaps {
            observer(&BrainEvent::Gap(gap.clone()));
        }
        observer(&BrainEvent::Completed {
            sources: answer.sources,
            claims: answer.claims.len(),
            latency_ms: answer.latency_ms,
            confidence: answer.confidence,
        });
        Ok(answer)
    }

    /// Converts provider output into answer claims, failing closed on anything
    /// that breaks the contract. An uncited fact is not repaired, downgraded,
    /// or dropped: it means the producer cannot be trusted for this answer.
    fn adopt(
        &self,
        output: SynthesisOutput,
        source_to_citation: &HashMap<String, String>,
    ) -> Result<(Vec<Claim>, Vec<Gap>), AskError> {
        let mut claim_ids = IdAllocator::new("claim");
        let mut seen = Vec::new();
        for claim in &output.claims {
            if seen.contains(&claim.id) {
                return Err(AskError::DuplicateClaim(claim.id.clone()));
            }
            seen.push(claim.id.clone());
        }

        let mut claims = Vec::new();
        for raw in &output.claims {
            let mut citations = Vec::new();
            for source in &raw.sources {
                let citation =
                    source_to_citation
                        .get(source)
                        .ok_or_else(|| AskError::UnknownSource {
                            claim: raw.id.clone(),
                            source_id: source.clone(),
                        })?;
                if !citations.contains(citation) {
                    citations.push(citation.clone());
                }
            }
            if raw.kind == ClaimKind::Fact && citations.is_empty() {
                return Err(AskError::Contract(AnswerInvalid::UncitedFact(
                    raw.id.clone(),
                )));
            }
            let mut supports = Vec::new();
            for support in &raw.supports {
                if !seen.contains(support) {
                    return Err(AskError::UnknownSupport {
                        claim: raw.id.clone(),
                        support: support.clone(),
                    });
                }
                supports.push(claim_ids.id_for(support));
            }
            if raw.kind == ClaimKind::Fact && !supports.is_empty() {
                return Err(AskError::Contract(AnswerInvalid::FactWithSupports(
                    raw.id.clone(),
                )));
            }
            claims.push(Claim {
                id: claim_ids.id_for(&raw.id),
                kind: raw.kind,
                text: raw.text.trim().to_owned(),
                citations,
                supports,
                confidence: raw.confidence,
            });
        }

        let gaps = output
            .gaps
            .into_iter()
            .map(|gap| Gap {
                kind: gap.kind,
                subject: gap.subject,
                detail: gap.detail,
            })
            .collect();
        Ok((claims, gaps))
    }

    fn symbol_dependencies(
        &self,
        repository_id: &str,
        root: &std::path::Path,
        symbol: &SymbolRow,
        started: Instant,
        cancellation: &CancellationToken,
    ) -> Result<DependencySlice, AskError> {
        cancelled(cancellation, "graph traversal")?;
        let neighborhood = self.index.graph_neighbors(
            repository_id,
            &symbol.symbol_id,
            &NeighborOptions {
                kinds: Vec::new(),
                depth: 2,
                direction: Direction::Both,
                limit: 120,
            },
        )?;

        let mut nodes: Vec<DependencyNode> = neighborhood
            .nodes
            .iter()
            .map(|node| DependencyNode {
                id: node.symbol_id.clone(),
                label: node.name.clone(),
                kind: node.kind.clone(),
                path: node.relative_path.clone(),
                line_start: node.start_line,
                line_end: node.end_line,
                distance: node.distance,
                resolved: true,
            })
            .collect();

        let mut edges = Vec::new();
        let mut gaps = Vec::new();
        let mut unresolved = 0;
        for edge in &neighborhood.edges {
            match &edge.target_id {
                Some(target) => edges.push(DependencyEdge {
                    kind: edge.edge_kind.clone(),
                    source: edge.source_id.clone(),
                    target: target.clone(),
                    confidence: edge.confidence,
                    incoming: edge.direction == Direction::Incoming,
                }),
                None => {
                    // An unresolved target is a real relationship whose other
                    // end the syntax-only resolver could not name. Showing it
                    // as an unresolved node is more honest than hiding it.
                    unresolved += 1;
                    let id = format!("unresolved:{}", edge.target_key);
                    if !nodes.iter().any(|node| node.id == id) {
                        nodes.push(DependencyNode {
                            id: id.clone(),
                            label: edge.target_key.clone(),
                            kind: "unresolved".into(),
                            path: String::new(),
                            line_start: 0,
                            line_end: 0,
                            distance: 1,
                            resolved: false,
                        });
                    }
                    edges.push(DependencyEdge {
                        kind: edge.edge_kind.clone(),
                        source: edge.source_id.clone(),
                        target: id,
                        confidence: edge.confidence,
                        incoming: false,
                    });
                }
            }
        }
        if unresolved > 0 {
            gaps.push(Gap {
                kind: GapKind::Ambiguous,
                subject: format!("{unresolved} edge(s) with an unresolved target"),
                detail: "Symbol resolution is syntax-only: a name shared by more than one \
                         definition stays unresolved rather than being guessed."
                    .to_owned(),
            });
        }
        if neighborhood.truncated {
            gaps.push(Gap {
                kind: GapKind::BudgetExhausted,
                subject: "the traversal limit".into(),
                detail: "The graph slice hit its result ceiling; narrow the target to see the \
                         rest."
                    .to_owned(),
            });
        }

        let citations = self.citations_for_nodes(root, &nodes);
        Ok(DependencySlice {
            target: symbol.symbol_id.clone(),
            target_kind: DependencyTarget::Symbol,
            nodes,
            edges,
            citations,
            gaps,
            truncated: neighborhood.truncated,
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }

    fn file_dependencies(
        &self,
        repository_id: &str,
        path: &str,
        started: Instant,
    ) -> Result<DependencySlice, AskError> {
        let dependencies = self.index.file_dependencies(repository_id, path)?;
        let mut nodes = vec![DependencyNode {
            id: path.to_owned(),
            label: path.to_owned(),
            kind: "file".into(),
            path: path.to_owned(),
            line_start: 0,
            line_end: 0,
            distance: 0,
            resolved: true,
        }];
        let mut edges = Vec::new();
        let mut external = 0;

        for specifier in &dependencies.imports {
            // The index stores the specifier as written. A graph to render
            // needs the file it names, so a relative specifier is resolved
            // against the importing file's directory; anything else is a
            // package outside the repository and stays unresolved.
            let resolved = self.resolve_specifier(repository_id, path, specifier)?;
            if resolved.is_none() {
                external += 1;
            }
            let id = resolved.clone().unwrap_or_else(|| specifier.clone());
            if !nodes.iter().any(|node| node.id == id) {
                nodes.push(DependencyNode {
                    id: id.clone(),
                    label: resolved.clone().unwrap_or_else(|| specifier.clone()),
                    kind: if resolved.is_some() {
                        "file"
                    } else {
                        "package"
                    }
                    .into(),
                    path: resolved.clone().unwrap_or_default(),
                    line_start: 0,
                    line_end: 0,
                    distance: 1,
                    resolved: resolved.is_some(),
                });
            }
            edges.push(DependencyEdge {
                kind: "IMPORTS".into(),
                source: path.to_owned(),
                target: id,
                confidence: 1.0,
                incoming: false,
            });
        }
        for importer in &dependencies.imported_by {
            nodes.push(DependencyNode {
                id: importer.clone(),
                label: importer.clone(),
                kind: "file".into(),
                path: importer.clone(),
                line_start: 0,
                line_end: 0,
                distance: 1,
                resolved: true,
            });
            edges.push(DependencyEdge {
                kind: "IMPORTS".into(),
                source: importer.clone(),
                target: path.to_owned(),
                confidence: 1.0,
                incoming: true,
            });
        }
        nodes.dedup_by(|left, right| left.id == right.id);

        let mut gaps = Vec::new();
        if external > 0 {
            gaps.push(Gap {
                kind: GapKind::NotIndexed,
                subject: format!("{external} external package(s)"),
                detail: "These imports resolve outside the repository, so Mars has no source \
                         for them."
                    .to_owned(),
            });
        }
        if !dependencies.freshness.is_fresh() {
            gaps.push(Gap {
                kind: GapKind::Stale,
                subject: path.to_owned(),
                detail: "This file has changed since it was indexed, so its edges may not \
                         match the current source."
                    .to_owned(),
            });
        }

        Ok(DependencySlice {
            target: path.to_owned(),
            target_kind: DependencyTarget::File,
            nodes,
            edges,
            citations: Vec::new(),
            gaps,
            truncated: false,
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }

    /// Citations for the definitions a graph slice shows, so the rendering can
    /// open real source rather than only draw boxes.
    fn citations_for_nodes(
        &self,
        root: &std::path::Path,
        nodes: &[DependencyNode],
    ) -> Vec<Citation> {
        let mut citations = Vec::new();
        let mut ids = IdAllocator::new("cite");
        for node in nodes.iter().filter(|node| node.resolved) {
            let Some(snippet) = read_span(root, &node.path, node.line_start, node.line_end) else {
                continue;
            };
            citations.push(Citation {
                id: ids.id_for(&node.id),
                path: node.path.clone(),
                line_start: node.line_start,
                line_end: node.line_end,
                // The span was read from the working tree just now, so the tree
                // is what it is anchored to.
                blob_sha: "working-tree".into(),
                snippet,
                why: format!("{} hop(s) from the queried target", node.distance),
            });
        }
        citations
    }

    fn current_shas(
        &self,
        repository_id: &str,
        answer: &BrainAnswer,
    ) -> Result<HashMap<String, String>, AskError> {
        let mut shas = HashMap::new();
        for citation in &answer.citations {
            if shas.contains_key(&citation.path) {
                continue;
            }
            let freshness = self.index.freshness(repository_id, &citation.path)?;
            if let Some(current) = freshness.current_blob_sha {
                shas.insert(citation.path.clone(), current);
            }
        }
        Ok(shas)
    }

    fn repository_root(&self, repository_id: &str) -> Result<PathBuf, AskError> {
        let root: Option<String> = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT root_path FROM repositories WHERE id = ?1",
                    [repository_id],
                    |row| row.get(0),
                )
                .optional()
        })?;
        root.map(PathBuf::from)
            .ok_or_else(|| AskError::RepositoryNotFound(repository_id.to_owned()))
    }

    /// Resolves an import specifier to an indexed file, or `None` when it names
    /// something outside the repository.
    ///
    /// This is deliberately not a module resolver: it follows a relative
    /// specifier from the importing file's directory and tries the extensions
    /// and `index` conventions the indexed languages share. Bare specifiers,
    /// path aliases, and package `exports` maps are left unresolved rather than
    /// guessed, which is the same limit ADR-013 places on symbol resolution.
    fn resolve_specifier(
        &self,
        repository_id: &str,
        importer: &str,
        specifier: &str,
    ) -> Result<Option<String>, AskError> {
        if !specifier.starts_with("./") && !specifier.starts_with("../") {
            return Ok(None);
        }
        let mut segments: Vec<&str> = importer.split('/').collect();
        segments.pop();
        for segment in specifier.split('/') {
            match segment {
                "." | "" => {}
                ".." => {
                    segments.pop();
                }
                segment => segments.push(segment),
            }
        }
        let base = segments.join("/");
        if base.is_empty() {
            return Ok(None);
        }
        const EXTENSIONS: [&str; 8] = ["ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go"];
        let mut candidates = vec![base.clone()];
        candidates.extend(
            EXTENSIONS
                .iter()
                .map(|extension| format!("{base}.{extension}")),
        );
        candidates.extend(
            EXTENSIONS
                .iter()
                .map(|extension| format!("{base}/index.{extension}")),
        );
        for candidate in candidates {
            if self.file_exists(repository_id, &candidate)? {
                return Ok(Some(candidate));
            }
        }
        Ok(None)
    }

    fn file_exists(&self, repository_id: &str, relative_path: &str) -> Result<bool, AskError> {
        Ok(self
            .storage
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT 1 FROM files
                     WHERE repository_id = ?1 AND relative_path = ?2 AND deleted_at IS NULL",
                        params![repository_id, relative_path],
                        |_| Ok(true),
                    )
                    .optional()
            })?
            .unwrap_or(false))
    }

    fn symbol(&self, repository_id: &str, symbol_id: &str) -> Result<Option<SymbolRow>, AskError> {
        Ok(self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT s.id, s.name, s.qualified_name, f.relative_path
                     FROM symbols s JOIN files f ON f.id = s.file_id
                     WHERE s.id = ?1 AND f.repository_id = ?2 AND f.deleted_at IS NULL",
                    params![symbol_id, repository_id],
                    |row| {
                        Ok(SymbolRow {
                            symbol_id: row.get(0)?,
                            name: row.get(1)?,
                            qualified_name: row.get(2)?,
                            relative_path: row.get(3)?,
                        })
                    },
                )
                .optional()
        })?)
    }
}

struct SymbolRow {
    symbol_id: String,
    name: String,
    qualified_name: Option<String>,
    relative_path: String,
}

fn cancelled(cancellation: &CancellationToken, stage: &str) -> Result<(), AskError> {
    if cancellation.is_cancelled() {
        return Err(AskError::Cancelled {
            stage: stage.to_owned(),
        });
    }
    Ok(())
}

/// The leading lines of a chunk body, used to re-find the span after an edit,
/// together with how many blank lines were skipped to reach real content.
fn snippet(body: &str) -> (String, usize) {
    let offset = body
        .lines()
        .take_while(|line| line.trim().is_empty())
        .count();
    let snippet = body
        .lines()
        .skip(offset)
        .take(SNIPPET_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    (snippet, offset)
}

fn read_span(
    root: &std::path::Path,
    relative_path: &str,
    line_start: usize,
    line_end: usize,
) -> Option<String> {
    if relative_path.is_empty() || line_start == 0 || line_end < line_start {
        return None;
    }
    if relative_path
        .split('/')
        .any(|segment| segment == ".." || segment.is_empty())
    {
        return None;
    }
    let mut absolute = root.to_path_buf();
    for segment in relative_path.split('/') {
        absolute.push(segment);
    }
    let content = std::fs::read_to_string(absolute).ok()?;
    let lines: Vec<&str> = content
        .lines()
        .skip(line_start - 1)
        .take((line_end - line_start + 1).min(SNIPPET_LINES))
        .collect();
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}
