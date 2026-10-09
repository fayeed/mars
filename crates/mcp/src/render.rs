//! Tool results as text written to be cheap to read.
//!
//! An agent pays for every token it reads, and the whole point of these tools
//! is to cost less than grepping. So results are line-oriented, locations are
//! `path:start-end`, ids appear once where a follow-up call needs them, and a
//! snippet is the first few lines of a match rather than the match. Anything
//! that rests on a file changed since indexing says so, because a stale answer
//! that looks current is worse than none.

use std::fmt::Write;

use mars_brain::{
    BrainAnswer, BrainEdge, BrainNeighborhood, BrainSymbol, ClaimKind, KnowledgeRecord,
    KnowledgeStatus, RetrievedMemory,
};
use mars_indexer::api::{
    FileDependencies, FreshnessState, IndexStatus, Neighborhood, SearchResults, SymbolLocation,
};

const SNIPPET_LINES: usize = 4;
const SNIPPET_HITS: usize = 6;
const LINE_WIDTH: usize = 160;

pub fn search(results: &SearchResults, snippets: bool) -> String {
    if results.hits.is_empty() {
        return format!(
            "Nothing indexed matches \"{}\". Try other words, or `find_symbol` for a name.",
            results.query
        );
    }
    let mut out = format!(
        "{} match(es) for \"{}\"{}\n",
        results.hits.len(),
        results.query,
        if results.contains_stale {
            " — some files changed since indexing; reread before relying on them"
        } else {
            ""
        }
    );
    let stale: Vec<&str> = results
        .freshness
        .iter()
        .filter(|freshness| freshness.state != FreshnessState::Fresh)
        .map(|freshness| freshness.relative_path.as_str())
        .collect();
    for (index, hit) in results.hits.iter().enumerate() {
        let marker = if stale.contains(&hit.relative_path.as_str()) {
            " (changed)"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "{}. {}:{}-{}  {}{}",
            index + 1,
            hit.relative_path,
            hit.start_line,
            hit.end_line,
            if hit.symbol_path.is_empty() {
                hit.header.as_str()
            } else {
                hit.symbol_path.as_str()
            },
            marker
        );
        if snippets && index < SNIPPET_HITS {
            for line in hit
                .body
                .lines()
                .filter(|line| !line.trim().is_empty())
                .take(SNIPPET_LINES)
            {
                let _ = writeln!(out, "   {}", clip(line.trim_end()));
            }
        }
    }
    out
}

pub fn symbols(query: &str, symbols: &[BrainSymbol]) -> String {
    if symbols.is_empty() {
        return format!("No indexed symbol matches `{query}`.");
    }
    let mut out = String::new();
    for symbol in symbols {
        let _ = writeln!(
            out,
            "{} {}  {}:{}-{}{}  id={}",
            symbol.kind,
            if symbol.qualified_name.is_empty() {
                &symbol.name
            } else {
                &symbol.qualified_name
            },
            symbol.relative_path,
            symbol.start_line,
            symbol.end_line,
            if symbol.exported { "  exported" } else { "" },
            symbol.id
        );
    }
    out
}

pub fn enclosing(path: &str, line: usize, symbols: &[SymbolLocation]) -> String {
    if symbols.is_empty() {
        return format!("No indexed definition encloses {path}:{line}.");
    }
    let mut out = format!("{path}:{line} is inside, innermost first:\n");
    for symbol in symbols {
        let _ = writeln!(
            out,
            "- {} {}  {}-{}{}  id={}",
            symbol.kind,
            symbol.qualified_name.as_deref().unwrap_or(&symbol.name),
            symbol.start_line,
            symbol.end_line,
            symbol
                .signature
                .as_deref()
                .map(|signature| format!("  `{}`", clip(signature)))
                .unwrap_or_default(),
            symbol.symbol_id
        );
    }
    if !symbols[0].freshness.is_fresh() {
        out.push_str("The file changed since it was indexed; line numbers may have moved.\n");
    }
    out
}

pub fn edges(kind: &str, name: &str, edges: &[BrainEdge], limit: usize) -> String {
    if edges.is_empty() {
        return format!(
            "No {kind} of `{name}` are indexed. The graph is syntax-derived, so a dynamic {} would not appear.",
            if kind == "callees" { "call" } else { "use" }
        );
    }
    let mut out = format!("{} {kind} of `{name}`:\n", edges.len());
    for edge in edges {
        let other = if kind == "callees" {
            edge.target.as_ref()
        } else {
            edge.source.as_ref()
        };
        let label = other.map_or_else(
            || edge.target_key.clone(),
            |symbol| format!("{} {}", symbol.kind, symbol.name),
        );
        let _ = writeln!(
            out,
            "- {}:{}  {}{}",
            edge.evidence.relative_path,
            edge.evidence.start_line,
            label,
            if edge.confidence < 0.8 {
                format!("  (confidence {:.1})", edge.confidence)
            } else {
                String::new()
            }
        );
    }
    if edges.len() >= limit {
        let _ = writeln!(
            out,
            "(limit reached; ask again with a larger limit for more)"
        );
    }
    out
}

pub fn neighbourhood(kind: &str, neighbourhood: &BrainNeighborhood, depth: usize) -> String {
    let root = &neighbourhood.root;
    if neighbourhood.symbols.is_empty() {
        return format!(
            "Nothing indexed depends on `{}` within {depth} hop(s).",
            root.name
        );
    }
    let mut out = format!(
        "{} {}: {} symbol(s) within {depth} hop(s) of {} {} ({}:{})\n",
        kind,
        root.name,
        neighbourhood.symbols.len(),
        root.kind,
        root.name,
        root.relative_path,
        root.start_line
    );
    let mut files: Vec<&str> = neighbourhood
        .symbols
        .iter()
        .map(|symbol| symbol.relative_path.as_str())
        .collect();
    files.sort_unstable();
    files.dedup();
    let _ = writeln!(out, "Files: {}", files.join(", "));
    for symbol in &neighbourhood.symbols {
        let _ = writeln!(
            out,
            "- {} {}  {}:{}-{}",
            symbol.kind, symbol.name, symbol.relative_path, symbol.start_line, symbol.end_line
        );
    }
    out
}

pub fn graph(neighbourhood: &Neighborhood) -> String {
    if neighbourhood.nodes.is_empty() {
        return "No indexed neighbours.".into();
    }
    let mut out = format!(
        "{} neighbour(s) within {} hop(s){}:\n",
        neighbourhood.nodes.len(),
        neighbourhood.depth,
        if neighbourhood.truncated {
            ", truncated"
        } else {
            ""
        }
    );
    for node in &neighbourhood.nodes {
        let relation = neighbourhood
            .edges
            .iter()
            .find(|edge| {
                edge.target_id.as_deref() == Some(node.symbol_id.as_str())
                    || edge.source_id == node.symbol_id
            })
            .map(|edge| edge.edge_kind.to_lowercase())
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "- [{}] {} {}  {}:{}-{}{}",
            node.distance,
            node.kind,
            node.name,
            node.relative_path,
            node.start_line,
            node.end_line,
            if relation.is_empty() {
                String::new()
            } else {
                format!("  {relation}")
            }
        );
    }
    out
}

pub fn file_dependencies(dependencies: &FileDependencies) -> String {
    let mut out = format!("{}\n", dependencies.relative_path);
    let _ = writeln!(
        out,
        "imports ({}): {}",
        dependencies.imports.len(),
        if dependencies.imports.is_empty() {
            "none".into()
        } else {
            dependencies.imports.join(", ")
        }
    );
    let _ = writeln!(
        out,
        "imported by ({}): {}",
        dependencies.imported_by.len(),
        if dependencies.imported_by.is_empty() {
            "none".into()
        } else {
            dependencies.imported_by.join(", ")
        }
    );
    if !dependencies.freshness.is_fresh() {
        out.push_str("The file changed since it was indexed.\n");
    }
    out
}

pub fn answer(answer: &BrainAnswer) -> String {
    let mut out = String::new();
    let citation = |id: &str| {
        answer
            .citations
            .iter()
            .find(|citation| citation.id == id)
            .map(|citation| {
                format!(
                    "{}:{}-{}",
                    citation.path, citation.line_start, citation.line_end
                )
            })
    };
    let facts: Vec<_> = answer
        .claims
        .iter()
        .filter(|claim| claim.kind == ClaimKind::Fact)
        .collect();
    let inferences: Vec<_> = answer
        .claims
        .iter()
        .filter(|claim| claim.kind == ClaimKind::Inference)
        .collect();
    if facts.is_empty() && inferences.is_empty() {
        out.push_str("No claim could be supported by the indexed code.\n");
    }
    if !facts.is_empty() {
        out.push_str("Facts (each cited):\n");
        for fact in facts {
            let cited: Vec<String> = fact
                .citations
                .iter()
                .filter_map(|id| citation(id))
                .collect();
            let _ = writeln!(out, "- {} [{}]", fact.text.trim(), cited.join(", "));
        }
    }
    if !inferences.is_empty() {
        out.push_str("Inferences (reasoning, not evidence):\n");
        for inference in inferences {
            let _ = writeln!(out, "- {}", inference.text.trim());
        }
    }
    if !answer.gaps.is_empty() {
        out.push_str("Could not determine:\n");
        for gap in &answer.gaps {
            let _ = writeln!(out, "- {}: {}", gap.subject, gap.detail);
        }
    }
    let _ = writeln!(
        out,
        "({} source(s) of {} retrieved, {}ms{})",
        answer.sources,
        answer.retrieved,
        answer.latency_ms,
        if answer.truncated {
            ", budget dropped some"
        } else {
            ""
        }
    );
    out
}

pub fn index_status(status: &IndexStatus, stale_files: usize) -> String {
    let mut out = format!(
        "{} file(s), {} symbol(s), {} edge(s), {} chunk(s), {} commit(s) indexed.\n",
        status.counts.files,
        status.counts.symbols,
        status.counts.edges,
        status.counts.chunks,
        status.counts.commits
    );
    let _ = writeln!(
        out,
        "Branch {}; index built from {}{}.",
        status.branch.as_deref().unwrap_or("(detached)"),
        status
            .indexed_head_oid
            .as_deref()
            .map_or("no commit", |oid| &oid[..oid.len().min(12)]),
        if status.head_matches_index {
            ", which is the current commit"
        } else {
            ", not the current commit"
        }
    );
    if stale_files > 0 {
        let _ = writeln!(
            out,
            "{stale_files} file(s) changed since indexing; results touching them are marked."
        );
    }
    if status.running_runs > 0 {
        out.push_str("An index run is in progress.\n");
    }
    out
}

pub fn memories(memories: &[RetrievedMemory]) -> String {
    if memories.is_empty() {
        return "No approved memory applies here.".into();
    }
    let mut out = format!(
        "{} approved memor(ies), most relevant first:\n",
        memories.len()
    );
    for retrieved in memories {
        let memory = &retrieved.memory;
        let _ = writeln!(
            out,
            "- [{} · {}] {}  (approved by {})",
            memory.kind.as_str(),
            memory.scope.as_str(),
            memory.text.trim(),
            memory.approved_by
        );
    }
    out
}

pub fn decisions(records: &[KnowledgeRecord]) -> String {
    if records.is_empty() {
        return "No decisions are recorded for this project yet.".into();
    }
    let mut out = String::new();
    for record in records {
        let status = match record.status {
            KnowledgeStatus::Approved => "approved",
            KnowledgeStatus::Candidate => "awaiting approval",
            KnowledgeStatus::Rejected => "rejected",
            KnowledgeStatus::Stale => "stale",
            KnowledgeStatus::Superseded => "superseded",
        };
        let _ = writeln!(out, "## {} ({status})", record.title.trim());
        for line in record.body.lines().take(8) {
            let _ = writeln!(out, "{}", clip(line));
        }
    }
    out
}

fn clip(line: &str) -> String {
    if line.chars().count() <= LINE_WIDTH {
        return line.to_owned();
    }
    let mut clipped: String = line.chars().take(LINE_WIDTH - 1).collect();
    clipped.push('…');
    clipped
}
