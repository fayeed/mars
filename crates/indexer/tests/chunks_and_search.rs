mod support;

use std::fs;

use mars_indexer::api::IndexApi;
use mars_indexer::search::{Retrieval, SearchFilters};
use mars_indexer::{Indexer, embedding};
use tokio_util::sync::CancellationToken;

use support::polyglot_fixture;

#[test]
fn chunks_follow_symbol_boundaries_and_carry_their_location_header() {
    let fixture = polyglot_fixture("chunk-shape");
    Indexer::new(fixture.storage.clone())
        .index_repository(&fixture.repository_id, &CancellationToken::new())
        .expect("fixture should index");

    let chunks: Vec<(String, String, String, i64, i64)> = fixture
        .storage
        .with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT f.relative_path, c.symbol_path, c.header, c.start_line, c.end_line
                 FROM chunks c JOIN files f ON f.id = c.file_id
                 WHERE f.relative_path = 'src/service.py'
                 ORDER BY c.ordinal",
            )?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                })?
                .collect()
        })
        .expect("chunks should query");

    assert!(!chunks.is_empty(), "the Python file should produce chunks");
    for (path, symbol_path, header, start, end) in &chunks {
        assert!(
            header.starts_with(path),
            "header `{header}` must name its file"
        );
        if !symbol_path.is_empty() {
            assert!(
                header.contains(symbol_path.as_str()),
                "header `{header}` must name its symbol path"
            );
        }
        assert!(end >= start, "chunk lines must be ordered");
    }

    // A function is one chunk, never split across two.
    let function_chunks = chunks
        .iter()
        .filter(|(_, symbol_path, _, _, _)| symbol_path == "apply_tax")
        .count();
    assert_eq!(function_chunks, 1, "{chunks:?}");

    let bodies: Vec<String> = fixture
        .storage
        .with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT c.body FROM chunks c JOIN files f ON f.id = c.file_id
                 WHERE f.relative_path = 'src/service.py' AND c.symbol_path = 'apply_tax'",
            )?;
            statement.query_map([], |row| row.get(0))?.collect()
        })
        .expect("chunk body should query");
    assert!(
        bodies[0].contains("def apply_tax") && bodies[0].contains("return amount"),
        "a function chunk holds its whole body: {}",
        bodies[0]
    );
}

#[test]
fn chunk_ids_survive_reindexing_when_content_is_unchanged() {
    let fixture = polyglot_fixture("chunk-stability");
    let indexer = Indexer::new(fixture.storage.clone());
    indexer
        .index_repository(&fixture.repository_id, &CancellationToken::new())
        .expect("first index should finish");

    let ids = |fixture: &support::IndexFixture, path: &str| -> Vec<String> {
        fixture
            .storage
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT c.id FROM chunks c JOIN files f ON f.id = c.file_id
                     WHERE f.relative_path = ?1 ORDER BY c.symbol_path, c.start_byte",
                )?;
                statement.query_map([path], |row| row.get(0))?.collect()
            })
            .expect("chunk ids should query")
    };

    let before_go = ids(&fixture, "src/server.go");
    let before_python = ids(&fixture, "src/service.py");
    assert!(!before_go.is_empty());

    // Touch one file; every other file's chunk ids must be untouched, and the
    // edited file keeps the ids of the chunks that did not change.
    let python = fixture.root.join("src/service.py");
    let original = fs::read_to_string(&python).expect("python source should read");
    fs::write(
        &python,
        original.replace(
            "TAX_RATE = Decimal(\"0.2\")",
            "TAX_RATE = Decimal(\"0.25\")",
        ),
    )
    .expect("python source should write");

    indexer
        .index_repository(&fixture.repository_id, &CancellationToken::new())
        .expect("second index should finish");

    assert_eq!(
        before_go,
        ids(&fixture, "src/server.go"),
        "an untouched file keeps every chunk id"
    );
    let after_python = ids(&fixture, "src/service.py");
    let unchanged: Vec<&String> = before_python
        .iter()
        .filter(|id| after_python.contains(id))
        .collect();
    assert!(
        !unchanged.is_empty(),
        "unchanged chunks in an edited file keep their ids"
    );
    assert_ne!(
        before_python, after_python,
        "the edited chunk gets a new id"
    );
}

#[test]
fn every_chunk_carries_a_normalized_local_embedding() {
    let fixture = polyglot_fixture("embeddings");
    Indexer::new(fixture.storage.clone())
        .index_repository(&fixture.repository_id, &CancellationToken::new())
        .expect("fixture should index");

    let (chunks, embeddings): (i64, i64) = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT (SELECT COUNT(*) FROM chunks), (SELECT COUNT(*) FROM chunk_embeddings)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .expect("counts should query");
    assert_eq!(chunks, embeddings, "every chunk is embedded");

    let (model, dimensions, vector): (String, i64, Vec<u8>) = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT model, dimensions, vector FROM chunk_embeddings LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
        })
        .expect("embedding should query");

    assert_eq!(model, embedding::MODEL);
    assert_eq!(dimensions as usize, embedding::DIMENSIONS);
    let decoded = embedding::decode(&vector).expect("vector should decode");
    assert_eq!(decoded.len(), embedding::DIMENSIONS);
    let magnitude = decoded
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    assert!(
        (magnitude - 1.0).abs() < 1e-3,
        "vector magnitude was {magnitude}"
    );
}

#[test]
fn lexical_semantic_and_hybrid_search_each_answer_and_label_freshness() {
    let fixture = polyglot_fixture("search");
    let api = IndexApi::new(fixture.storage.clone());
    api.reindex(&fixture.repository_id, None, &CancellationToken::new())
        .expect("fixture should index");

    let filters = SearchFilters::default();

    let lexical = api
        .search_lexical(&fixture.repository_id, "InvoiceBuilder", &filters, 5)
        .expect("lexical search should run");
    assert!(
        lexical
            .hits
            .iter()
            .any(|hit| hit.relative_path == "src/service.py"),
        "{:?}",
        lexical.hits
    );
    assert!(
        lexical
            .hits
            .iter()
            .all(|hit| hit.retrieval == Retrieval::Lexical)
    );
    assert!(!lexical.contains_stale);

    // Semantic recall works on wording the file never uses verbatim.
    let semantic = api
        .search_semantic(
            &fixture.repository_id,
            "retry backoff attempts",
            5,
            &filters,
        )
        .expect("semantic search should run");
    assert!(
        semantic
            .hits
            .iter()
            .any(|hit| hit.relative_path == "src/engine.rs"),
        "{:?}",
        semantic.hits
    );

    let hybrid = api
        .search_hybrid(&fixture.repository_id, "invoice tax", 5, &filters)
        .expect("hybrid search should run");
    assert!(!hybrid.hits.is_empty());
    assert!(
        hybrid
            .hits
            .iter()
            .all(|hit| hit.retrieval == Retrieval::Hybrid)
    );
    // Fusion is ordered by the combined rank, best first.
    for pair in hybrid.hits.windows(2) {
        assert!(pair[0].score >= pair[1].score, "{:?}", hybrid.hits);
    }

    // A hit reached by both retrievers outranks one reached by only one.
    let fused = hybrid
        .hits
        .iter()
        .any(|hit| hit.lexical_rank.is_some() && hit.semantic_rank.is_some());
    assert!(fused, "hybrid should fuse both lists: {:?}", hybrid.hits);
}

#[test]
fn search_filters_restrict_results_by_path_and_language() {
    let fixture = polyglot_fixture("search-filters");
    let api = IndexApi::new(fixture.storage.clone());
    api.reindex(&fixture.repository_id, None, &CancellationToken::new())
        .expect("fixture should index");

    let filters = SearchFilters {
        path_prefixes: vec!["src/".into()],
        languages: vec!["rust".into()],
    };
    let results = api
        .search_hybrid(&fixture.repository_id, "backoff", 10, &filters)
        .expect("filtered search should run");

    assert!(!results.hits.is_empty());
    for hit in &results.hits {
        assert_eq!(hit.language, "rust", "{hit:?}");
        assert!(hit.relative_path.starts_with("src/"), "{hit:?}");
    }
}

#[test]
fn answers_built_on_changed_files_are_labelled_stale() {
    let fixture = polyglot_fixture("staleness");
    let api = IndexApi::new(fixture.storage.clone());
    api.reindex(&fixture.repository_id, None, &CancellationToken::new())
        .expect("fixture should index");

    let fresh = api
        .freshness(&fixture.repository_id, "src/engine.rs")
        .expect("freshness should resolve");
    assert!(fresh.is_fresh());
    assert_eq!(fresh.indexed_blob_sha, fresh.current_blob_sha);

    // Change the file without re-indexing: the index must admit it is behind.
    fs::write(
        fixture.root.join("src/engine.rs"),
        "pub fn replaced() -> usize { 1 }\n",
    )
    .expect("source should write");

    let stale = api
        .freshness(&fixture.repository_id, "src/engine.rs")
        .expect("freshness should resolve");
    assert!(!stale.is_fresh(), "{stale:?}");
    assert_ne!(stale.indexed_blob_sha, stale.current_blob_sha);

    let results = api
        .search_lexical(
            &fixture.repository_id,
            "backoff",
            &SearchFilters::default(),
            10,
        )
        .expect("search should run");
    assert!(
        results.contains_stale,
        "an answer resting on a changed file is labelled stale: {results:?}"
    );

    let stale_paths = api
        .stale_paths(&fixture.repository_id)
        .expect("stale paths should resolve");
    assert!(
        stale_paths
            .iter()
            .any(|entry| entry.relative_path == "src/engine.rs"),
        "{stale_paths:?}"
    );
}

#[test]
fn ignore_status_explains_which_rule_excluded_a_path() {
    let fixture = polyglot_fixture("ignore-status");
    let api = IndexApi::new(fixture.storage.clone());
    api.reindex(&fixture.repository_id, None, &CancellationToken::new())
        .expect("fixture should index");

    let gitignored = api
        .ignore_status(&fixture.repository_id, "generated/output.ts")
        .expect("ignore status should resolve");
    assert!(gitignored.ignored);
    assert_eq!(gitignored.reason.as_deref(), Some("gitignore"));

    let marsignored = api
        .ignore_status(&fixture.repository_id, "notes.private.md")
        .expect("ignore status should resolve");
    assert!(marsignored.ignored);
    assert_eq!(marsignored.reason.as_deref(), Some("marsignore"));

    let indexed = api
        .ignore_status(&fixture.repository_id, "src/engine.rs")
        .expect("ignore status should resolve");
    assert!(!indexed.ignored);
}
