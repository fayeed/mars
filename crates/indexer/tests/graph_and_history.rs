mod support;

use std::time::Instant;

use mars_indexer::api::{Direction, IndexApi, NeighborOptions};
use tokio_util::sync::CancellationToken;

use support::{index_fixture, polyglot_fixture};

/// Answers "who calls X" against the TypeScript fixture, where `add` is called
/// from both the calculator and the test.
#[test]
fn graph_neighbors_answers_who_calls_a_symbol() {
    let fixture = index_fixture("graph-callers");
    let api = IndexApi::new(fixture.storage.clone());
    api.reindex(&fixture.repository_id, None, &CancellationToken::new())
        .expect("fixture should index");

    let symbol_id: String = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT s.id FROM symbols s JOIN files f ON f.id = s.file_id
                 WHERE s.name = 'add' AND s.kind = 'function' AND f.relative_path = 'src/math.ts'",
                [],
                |row| row.get(0),
            )
        })
        .expect("add should be indexed");

    let callers = api
        .graph_neighbors(
            &fixture.repository_id,
            &symbol_id,
            &NeighborOptions {
                kinds: vec!["CALLS".into()],
                depth: 1,
                direction: Direction::Incoming,
                limit: 50,
            },
        )
        .expect("neighbors should resolve");

    assert!(
        callers
            .edges
            .iter()
            .all(|edge| edge.edge_kind == "CALLS" && edge.direction == Direction::Incoming),
        "{:?}",
        callers.edges
    );
    assert!(
        callers
            .nodes
            .iter()
            .any(|node| node.name == "calculate" || node.name == "adds values"),
        "a caller of add should appear: {:?}",
        callers.nodes
    );
    assert!(callers.freshness.is_fresh());
    // Every edge keeps the evidence that produced it.
    for edge in &callers.edges {
        assert!(edge.evidence_json.contains("startLine"), "{edge:?}");
        assert!(edge.confidence > 0.0);
    }
}

#[test]
fn graph_neighbors_answers_what_breaks_if_a_symbol_changes() {
    let fixture = index_fixture("graph-impact");
    let api = IndexApi::new(fixture.storage.clone());
    api.reindex(&fixture.repository_id, None, &CancellationToken::new())
        .expect("fixture should index");

    let symbol_id: String = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT s.id FROM symbols s JOIN files f ON f.id = s.file_id
                 WHERE s.name = 'BaseCalculator'",
                [],
                |row| row.get(0),
            )
        })
        .expect("BaseCalculator should be indexed");

    // Two hops out: what extends it, and what depends on those.
    let impact = api
        .graph_neighbors(
            &fixture.repository_id,
            &symbol_id,
            &NeighborOptions {
                kinds: Vec::new(),
                depth: 2,
                direction: Direction::Incoming,
                limit: 100,
            },
        )
        .expect("impact should resolve");

    assert!(
        impact.nodes.iter().any(|node| node.name == "Calculator"),
        "the subclass is affected: {:?}",
        impact.nodes
    );
    assert!(
        impact.nodes.iter().all(|node| node.distance <= 2),
        "traversal stays inside the requested depth: {:?}",
        impact.nodes
    );
}

#[test]
fn file_dependencies_answer_what_a_file_depends_on() {
    let fixture = index_fixture("graph-dependencies");
    let api = IndexApi::new(fixture.storage.clone());
    api.reindex(&fixture.repository_id, None, &CancellationToken::new())
        .expect("fixture should index");

    let dependencies = api
        .file_dependencies(&fixture.repository_id, "src/main.ts")
        .expect("dependencies should resolve");
    assert!(
        dependencies.imports.iter().any(|import| import == "./math"),
        "{dependencies:?}"
    );
    assert!(
        dependencies
            .imports
            .iter()
            .any(|import| import == "./constants"),
        "{dependencies:?}"
    );

    let dependents = api
        .file_dependencies(&fixture.repository_id, "src/math.ts")
        .expect("dependents should resolve");
    assert!(
        dependents
            .imported_by
            .iter()
            .any(|path| path == "src/main.ts"),
        "{dependents:?}"
    );
}

/// The stated budget is under 100 ms on a 20k-file repository. The fixture is
/// small, so this asserts the query path itself is bounded and indexed rather
/// than claiming the headline number.
#[test]
fn graph_queries_answer_well_inside_the_latency_budget() {
    let fixture = index_fixture("graph-latency");
    let api = IndexApi::new(fixture.storage.clone());
    api.reindex(&fixture.repository_id, None, &CancellationToken::new())
        .expect("fixture should index");

    let symbol_id: String = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT id FROM symbols WHERE name = 'add' LIMIT 1",
                [],
                |row| row.get(0),
            )
        })
        .expect("add should be indexed");

    let started = Instant::now();
    for _ in 0..20 {
        api.graph_neighbors(
            &fixture.repository_id,
            &symbol_id,
            &NeighborOptions {
                depth: 2,
                ..NeighborOptions::default()
            },
        )
        .expect("neighbors should resolve");
    }
    let average = started.elapsed() / 20;
    assert!(
        average.as_millis() < 100,
        "graph traversal averaged {average:?}"
    );
}

#[test]
fn history_records_commits_churn_and_last_touched_by() {
    let fixture = polyglot_fixture("history");
    let api = IndexApi::new(fixture.storage.clone());
    api.reindex(&fixture.repository_id, None, &CancellationToken::new())
        .expect("fixture should index");

    let commits: i64 = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row("SELECT COUNT(*) FROM commits", [], |row| row.get(0))
        })
        .expect("commits should query");
    assert!(commits >= 1, "the fixture commit should be ingested");

    let blame = api
        .history_blame(&fixture.repository_id, "src/engine.rs", 1, 10)
        .expect("blame should resolve");

    assert!(!blame.ranges.is_empty(), "blame should cover the range");
    for range in &blame.ranges {
        assert_eq!(range.author_name, "Mars Fixture", "{range:?}");
        assert!(range.start_line >= 1 && range.end_line <= 10, "{range:?}");
        assert_eq!(range.commit_oid.len(), 40, "{range:?}");
    }

    assert_eq!(blame.churn.relative_path, "src/engine.rs");
    assert!(blame.churn.commits >= 1, "{:?}", blame.churn);
    assert!(blame.churn.insertions > 0, "{:?}", blame.churn);
    assert_eq!(blame.churn.last_touched_by.as_deref(), Some("Mars Fixture"));
    assert!(blame.churn.last_commit_oid.is_some());
}

#[test]
fn history_ingest_is_incremental_across_runs() {
    let fixture = polyglot_fixture("history-incremental");
    let api = IndexApi::new(fixture.storage.clone());
    api.reindex(&fixture.repository_id, None, &CancellationToken::new())
        .expect("first index should finish");

    let first = api
        .status(&fixture.repository_id)
        .expect("status should resolve");
    assert_eq!(first.counts.commits, 1);

    // A second run over unchanged history must not re-read what it already has.
    let summary = api
        .reindex(&fixture.repository_id, None, &CancellationToken::new())
        .expect("second index should finish");
    assert_eq!(summary.commits_ingested, 0);

    std::fs::write(
        fixture.root.join("src/engine.rs"),
        "//! Replaced.\npub fn replaced() -> usize { 1 }\n",
    )
    .expect("source should write");
    support::git(&fixture.root, &["add", "-A"]);
    support::git(&fixture.root, &["commit", "-m", "replace the engine"]);

    let third = api
        .reindex(&fixture.repository_id, None, &CancellationToken::new())
        .expect("third index should finish");
    assert_eq!(third.commits_ingested, 1, "only the new commit is read");

    let status = api
        .status(&fixture.repository_id)
        .expect("status should resolve");
    assert_eq!(status.counts.commits, 2);
    assert!(status.head_matches_index, "{status:?}");
}

#[test]
fn symbols_at_reports_the_innermost_definition_first() {
    let fixture = polyglot_fixture("symbols-at");
    let api = IndexApi::new(fixture.storage.clone());
    api.reindex(&fixture.repository_id, None, &CancellationToken::new())
        .expect("fixture should index");

    // Line 27 sits inside InvoiceBuilder.render, which sits inside the class.
    let symbols = api
        .symbols_at(&fixture.repository_id, "src/service.py", 27)
        .expect("symbols should resolve");

    assert!(symbols.len() >= 2, "{symbols:?}");
    assert_eq!(symbols[0].name, "render", "{symbols:?}");
    assert_eq!(symbols[0].kind, "method");
    assert!(
        symbols.iter().any(|symbol| symbol.name == "InvoiceBuilder"),
        "the enclosing class is reported too: {symbols:?}"
    );
    assert!(symbols[0].freshness.is_fresh());
    assert_eq!(
        symbols[0].documentation.as_deref(),
        Some("Serialises the invoice as JSON.")
    );

    let outside = api
        .symbols_at(&fixture.repository_id, "src/service.py", 1)
        .expect("symbols should resolve");
    assert!(
        outside.iter().all(|symbol| symbol.kind != "method"),
        "the module docstring line is not inside a method: {outside:?}"
    );
}

#[test]
fn status_reports_counts_and_the_commit_the_index_was_built_from() {
    let fixture = polyglot_fixture("status");
    let api = IndexApi::new(fixture.storage.clone());
    let summary = api
        .reindex(&fixture.repository_id, None, &CancellationToken::new())
        .expect("fixture should index");

    let status = api
        .status(&fixture.repository_id)
        .expect("status should resolve");

    assert_eq!(status.repository_id, fixture.repository_id);
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(status.head_oid, summary.head_oid);
    assert_eq!(status.indexed_head_oid, summary.head_oid);
    assert!(status.head_matches_index);
    assert_eq!(status.running_runs, 0);

    assert!(status.counts.files >= 8, "{status:?}");
    assert!(status.counts.symbols > 0, "{status:?}");
    assert!(status.counts.chunks > 0, "{status:?}");
    assert_eq!(status.counts.ignored_paths, 3, "{status:?}");

    let last_run = status.last_run.expect("a completed run should be recorded");
    assert_eq!(last_run.run_id, summary.run_id);
    assert_eq!(last_run.status, "completed");
    assert_eq!(last_run.mode, "full");
    assert_eq!(last_run.indexed_files, summary.indexed_files);
    assert!(last_run.finished_at.is_some());
    assert!(last_run.error.is_none());
}
