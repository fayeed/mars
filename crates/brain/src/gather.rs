//! Retrieval, graph expansion, and reranking.
//!
//! Three stages sit between a query plan and the context budget. Hybrid search
//! finds chunks that look like the question. Graph expansion pulls in the code
//! around the best of those, because the answer to "how does X happen" is
//! usually adjacent to the chunk that mentions X rather than inside it.
//! Reranking then scores every candidate against the *question*, not against
//! the query that happened to find it, and records the named signals it used
//! so a ranking can be explained rather than merely asserted.

use std::collections::HashMap;

use mars_indexer::IndexError;
use mars_indexer::api::{Direction, FreshnessState, IndexApi, NeighborOptions};
use mars_indexer::search::SearchHit;
use mars_storage::Storage;
use rusqlite::params;
use tokio_util::sync::CancellationToken;

use crate::answer::Signal;
use crate::rewrite::{QueryIntent, QueryPlan};

/// Reciprocal-rank fusion constant, matching the index's own fusion so the two
/// stages agree on how steeply rank should discount a hit.
const RRF_CONSTANT: f64 = 60.0;

/// How many fused hits seed graph expansion. Expansion is the expensive stage,
/// and past the first handful of hits it mostly rediscovers what search
/// already found.
const EXPANSION_SEEDS: usize = 8;

/// A graph neighbour starts at a fraction of the score of the hit that led to
/// it. Adjacency is evidence of relevance, but weaker evidence than matching
/// the query directly.
const GRAPH_DECAY: f64 = 0.55;

/// Per-query retrieval depth.
const PER_QUERY_LIMIT: usize = 24;

#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub chunk_id: String,
    pub path: String,
    pub symbol_path: String,
    pub header: String,
    pub body: String,
    pub language: String,
    pub line_start: usize,
    pub line_end: usize,
    pub blob_sha: String,
    pub symbol_id: Option<String>,
    /// Score after fusion; replaced by the reranked score in `rerank`.
    pub score: f64,
    pub signals: Vec<Signal>,
    /// Retrieval provenance, carried through to the citation's `why`.
    pub why: String,
    pub stale: bool,
}

impl Candidate {
    /// The text a synthesis provider sees, and the text redundancy is measured
    /// over. The header names the file and symbol path, which is what makes a
    /// packed chunk self-describing.
    pub(crate) fn content(&self) -> String {
        format!("{}\n{}", self.header, self.body)
    }
}

pub(crate) struct Gathered {
    pub candidates: Vec<Candidate>,
    /// Paths that search touched but whose index rows no longer match disk.
    pub stale_paths: Vec<String>,
    /// True when no query returned anything at all.
    pub empty: bool,
}

/// Runs every planned query, fuses the results, expands through the graph, and
/// reranks. Returns candidates in descending order of final score.
///
/// `seed_symbols` names symbols the caller already knows are the subject of the
/// question — `brain.explain(symbolId)` knows exactly which definition it was
/// asked about. Their chunks enter retrieval as if they were the strongest hit,
/// so the answer is anchored on the right definition even when its text is a
/// poor match for the words of the question.
pub(crate) fn gather(
    index: &IndexApi,
    storage: &Storage,
    repository_id: &str,
    plan: &QueryPlan,
    seed_symbols: &[String],
    cancellation: &CancellationToken,
) -> Result<Gathered, IndexError> {
    let mut fused: HashMap<String, Candidate> = HashMap::new();
    let mut stale_paths = Vec::new();
    let mut any_hits = false;

    for query in &plan.queries {
        if cancellation.is_cancelled() {
            break;
        }
        let results =
            index.search_hybrid(repository_id, &query.text, PER_QUERY_LIMIT, &plan.filters)?;
        if !results.hits.is_empty() {
            any_hits = true;
        }
        for freshness in &results.freshness {
            if freshness.state == FreshnessState::Stale
                && !stale_paths.contains(&freshness.relative_path)
            {
                stale_paths.push(freshness.relative_path.clone());
            }
        }
        for (rank, hit) in results.hits.iter().enumerate() {
            let contribution = query.weight / (RRF_CONSTANT + (rank + 1) as f64);
            match fused.get_mut(&hit.chunk_id) {
                Some(existing) => {
                    existing.score += contribution;
                }
                None => {
                    let mut candidate = candidate_from_hit(hit);
                    candidate.score = contribution;
                    candidate.why = format!(
                        "hybrid search for the {} query matched {}",
                        purpose_label(query.purpose),
                        hit.symbol_path
                    );
                    fused.insert(hit.chunk_id.clone(), candidate);
                }
            }
        }
    }

    let mut candidates: Vec<Candidate> = fused.into_values().collect();

    // A seeded symbol enters above the best search hit, because the caller
    // named it outright and no ranking signal outranks being asked directly.
    let strongest = candidates
        .iter()
        .map(|candidate| candidate.score)
        .fold(0.0_f64, f64::max);
    let mut seeded = Vec::new();
    for symbol_id in seed_symbols {
        for mut candidate in chunks_for_symbol(storage, repository_id, symbol_id)? {
            candidate.score = (strongest * 1.5).max(1.0);
            candidate.why = format!("the definition asked about, at {}", candidate.symbol_path);
            seeded.push(candidate);
        }
        if !seeded.is_empty() {
            any_hits = true;
        }
    }
    merge(&mut candidates, seeded);

    mark_stale(&mut candidates, &stale_paths);
    sort_by_score(&mut candidates);

    if !cancellation.is_cancelled() {
        let expanded =
            expand_through_graph(index, storage, repository_id, &candidates, cancellation)?;
        merge(&mut candidates, expanded);
        mark_stale(&mut candidates, &stale_paths);
    }

    rerank(&mut candidates, plan);
    Ok(Gathered {
        candidates,
        stale_paths,
        empty: !any_hits,
    })
}

fn purpose_label(purpose: crate::rewrite::RewritePurpose) -> &'static str {
    use crate::rewrite::RewritePurpose::{Concept, Identifiers, Keywords, Subwords, Verbatim};
    match purpose {
        Verbatim => "verbatim",
        Keywords => "keyword",
        Identifiers => "identifier",
        Subwords => "subword",
        Concept => "concept",
    }
}

fn candidate_from_hit(hit: &SearchHit) -> Candidate {
    Candidate {
        chunk_id: hit.chunk_id.clone(),
        path: hit.relative_path.clone(),
        symbol_path: hit.symbol_path.clone(),
        header: hit.header.clone(),
        body: hit.body.clone(),
        language: hit.language.clone(),
        line_start: hit.start_line,
        line_end: hit.end_line,
        blob_sha: hit.blob_sha.clone(),
        symbol_id: None,
        score: 0.0,
        signals: Vec::new(),
        why: String::new(),
        stale: false,
    }
}

/// Walks one hop out from the strongest hits and pulls in the chunks belonging
/// to the symbols found there.
fn expand_through_graph(
    index: &IndexApi,
    storage: &Storage,
    repository_id: &str,
    seeds: &[Candidate],
    cancellation: &CancellationToken,
) -> Result<Vec<Candidate>, IndexError> {
    let mut expanded = Vec::new();
    for seed in seeds.iter().take(EXPANSION_SEEDS) {
        if cancellation.is_cancelled() {
            break;
        }
        // `symbols_at` returns innermost first, which is the definition the
        // chunk is actually about.
        let symbols = index.symbols_at(repository_id, &seed.path, seed.line_start)?;
        let Some(origin) = symbols.first() else {
            continue;
        };
        let neighborhood = index.graph_neighbors(
            repository_id,
            &origin.symbol_id,
            &NeighborOptions {
                kinds: Vec::new(),
                depth: 1,
                direction: Direction::Both,
                limit: 32,
            },
        )?;
        for node in &neighborhood.nodes {
            if node.distance == 0 || cancellation.is_cancelled() {
                continue;
            }
            for mut candidate in chunks_for_symbol(storage, repository_id, &node.symbol_id)? {
                candidate.score = seed.score * GRAPH_DECAY;
                candidate.why = format!(
                    "one hop from {} in the code graph, reached via {}",
                    origin.name, seed.symbol_path
                );
                expanded.push(candidate);
            }
        }
    }
    Ok(expanded)
}

fn chunks_for_symbol(
    storage: &Storage,
    repository_id: &str,
    symbol_id: &str,
) -> Result<Vec<Candidate>, IndexError> {
    let rows = storage.with_connection(|connection| {
        let mut statement = connection.prepare(
            "SELECT c.id, f.relative_path, c.symbol_path, c.header, c.body, f.language,
                    c.start_line, c.end_line, c.blob_sha, c.symbol_id
             FROM chunks c JOIN files f ON f.id = c.file_id
             WHERE c.repository_id = ?1 AND c.symbol_id = ?2 AND f.deleted_at IS NULL
             ORDER BY c.ordinal",
        )?;
        statement
            .query_map(params![repository_id, symbol_id], |row| {
                Ok(Candidate {
                    chunk_id: row.get(0)?,
                    path: row.get(1)?,
                    symbol_path: row.get(2)?,
                    header: row.get(3)?,
                    body: row.get(4)?,
                    language: row.get(5)?,
                    line_start: row.get::<_, i64>(6)? as usize,
                    line_end: row.get::<_, i64>(7)? as usize,
                    blob_sha: row.get(8)?,
                    symbol_id: row.get(9)?,
                    score: 0.0,
                    signals: Vec::new(),
                    why: String::new(),
                    stale: false,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
    })?;
    Ok(rows)
}

/// Adds expanded candidates that search did not already find. A chunk search
/// found on its own keeps its stronger provenance and score.
fn merge(candidates: &mut Vec<Candidate>, expanded: Vec<Candidate>) {
    let mut positions: HashMap<String, usize> = candidates
        .iter()
        .enumerate()
        .map(|(position, candidate)| (candidate.chunk_id.clone(), position))
        .collect();
    for candidate in expanded {
        match positions.get(&candidate.chunk_id) {
            Some(&position) => {
                // Being both a search hit and a graph neighbour is corroborating
                // evidence, so the two contributions add.
                candidates[position].score += candidate.score;
            }
            None => {
                positions.insert(candidate.chunk_id.clone(), candidates.len());
                candidates.push(candidate);
            }
        }
    }
}

fn mark_stale(candidates: &mut [Candidate], stale_paths: &[String]) {
    for candidate in candidates.iter_mut() {
        if stale_paths.contains(&candidate.path) {
            candidate.stale = true;
        }
    }
}

/// Scores each candidate against the question and records why. Weights are
/// deliberately flat and few: a scoring function nobody can explain is worse
/// than one that is merely approximate.
fn rerank(candidates: &mut [Candidate], plan: &QueryPlan) {
    let strongest = candidates
        .iter()
        .map(|candidate| candidate.score)
        .fold(0.0_f64, f64::max)
        .max(f64::MIN_POSITIVE);

    for candidate in candidates.iter_mut() {
        let content = candidate.content().to_lowercase();
        let path = candidate.path.to_lowercase();
        let symbol_path = candidate.symbol_path.to_lowercase();

        // Normalized so that the strongest retrieval result contributes 1.0 and
        // the weights below are comparable across queries of different sizes.
        let retrieval = candidate.score / strongest;

        let matched_terms = plan
            .terms
            .iter()
            .filter(|term| content.contains(term.as_str()))
            .count();
        let term_overlap = if plan.terms.is_empty() {
            0.0
        } else {
            matched_terms as f64 / plan.terms.len() as f64
        };

        let identifier_match = if plan.identifiers.is_empty() {
            0.0
        } else {
            let matched = plan
                .identifiers
                .iter()
                .filter(|identifier| {
                    let identifier = identifier.to_lowercase();
                    symbol_path.contains(&identifier) || content.contains(&identifier)
                })
                .count();
            matched as f64 / plan.identifiers.len() as f64
        };

        let path_match = if plan.terms.is_empty() {
            0.0
        } else {
            let matched = plan
                .terms
                .iter()
                .filter(|term| path.contains(term.as_str()))
                .count();
            matched as f64 / plan.terms.len() as f64
        };

        // A "where is it" question wants the definition; a "what calls it"
        // question wants the neighbourhood. Naming a symbol is what makes a
        // chunk answer the first kind of question.
        let intent_fit = match plan.intent {
            QueryIntent::Locate => {
                if candidate.symbol_path.is_empty() {
                    0.0
                } else {
                    identifier_match
                }
            }
            QueryIntent::Relate => {
                if candidate.symbol_id.is_some() {
                    1.0
                } else {
                    0.0
                }
            }
            QueryIntent::Explain | QueryIntent::Behaviour | QueryIntent::General => term_overlap,
        };

        // Stale evidence is not discarded here — the answer says so instead —
        // but it should not outrank an equally good fresh source.
        let freshness = if candidate.stale { -0.25 } else { 0.0 };

        let score = 1.00 * retrieval
            + 0.60 * term_overlap
            + 0.50 * identifier_match
            + 0.25 * path_match
            + 0.20 * intent_fit
            + freshness;

        candidate.signals = vec![
            Signal {
                name: "retrieval".into(),
                value: round(retrieval),
            },
            Signal {
                name: "termOverlap".into(),
                value: round(term_overlap),
            },
            Signal {
                name: "identifierMatch".into(),
                value: round(identifier_match),
            },
            Signal {
                name: "pathMatch".into(),
                value: round(path_match),
            },
            Signal {
                name: "intentFit".into(),
                value: round(intent_fit),
            },
            Signal {
                name: "freshness".into(),
                value: round(freshness),
            },
        ];
        candidate.signals.retain(|signal| signal.value != 0.0);
        candidate
            .signals
            .sort_by(|left, right| right.value.total_cmp(&left.value));
        candidate.score = score.max(0.0);
    }
    sort_by_score(candidates);
}

/// Sorts by score, breaking ties on path and line so that two runs over the
/// same index produce the same order.
fn sort_by_score(candidates: &mut [Candidate]) {
    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.line_start.cmp(&right.line_start))
            .then_with(|| left.chunk_id.cmp(&right.chunk_id))
    });
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}
