mod support;

use std::fs;

use mars_indexer::Indexer;
use tokio_util::sync::CancellationToken;

use support::index_fixture;

#[test]
fn indexes_typescript_structure_with_spans_resolution_and_evidence() {
    let fixture = index_fixture("graph");
    let indexer = Indexer::new(fixture.storage.clone());

    let summary = indexer
        .index_repository(&fixture.repository_id, &CancellationToken::new())
        .expect("fixture should index");

    // The file layer covers every non-ignored file, including the ignore
    // configuration itself; only source files contribute symbols.
    assert_eq!(summary.indexed_files, 8);
    assert_eq!(summary.skipped_files, 0);
    assert_eq!(summary.deleted_files, 0);
    assert_eq!(summary.renamed_files, 0);

    fixture
        .storage
        .with_connection(|connection| {
            let ignored: i64 = connection.query_row(
                "SELECT COUNT(*) FROM files WHERE relative_path IN ('ignored.ts', 'private.ts', 'node_modules/dependency/index.js')",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(ignored, 0);

            let add: (String, String, i64, i64, i64) = connection.query_row(
                "SELECT kind, stable_key, start_line, end_line, exported FROM symbols WHERE name = 'add' AND kind = 'function'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )?;
            assert_eq!(add.0, "function");
            assert!(add.1.contains("add"));
            assert!(add.2 > 0 && add.3 >= add.2);
            assert_eq!(add.4, 1);

            let required_symbols: i64 = connection.query_row(
                "SELECT COUNT(*) FROM symbols WHERE (name, kind) IN (('Numeric', 'interface'), ('Calculator', 'class'), ('calculate', 'method'), ('stillVisible', 'function'), ('adds values', 'test'))",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(required_symbols, 5);

            let parse_errors: i64 = connection.query_row(
                "SELECT parse_errors FROM files WHERE relative_path = 'src/broken.ts'",
                [],
                |row| row.get(0),
            )?;
            assert!(parse_errors > 0);

            let resolved_call: (Option<String>, String, f64, String) = connection.query_row(
                "SELECT target_id, target_key, confidence, evidence_json FROM code_edges WHERE edge_kind = 'CALLS' AND target_key = 'add' LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
            assert!(resolved_call.0.is_some());
            assert!(resolved_call.2 >= 0.8);
            assert!(resolved_call.3.contains("startLine"));

            let unresolved: (Option<String>, f64) = connection.query_row(
                "SELECT target_id, confidence FROM code_edges WHERE edge_kind = 'CALLS' AND target_key = 'expect' LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            assert!(unresolved.0.is_none());
            assert!(unresolved.1 < 1.0);

            let inheritance: i64 = connection.query_row(
                "SELECT COUNT(*) FROM code_edges WHERE edge_kind = 'EXTENDS' AND target_key = 'BaseCalculator'",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(inheritance, 1);

            let reference: (Option<String>, f64) = connection.query_row(
                "SELECT target_id, confidence FROM code_edges WHERE edge_kind = 'REFERENCES' AND target_key = 'sharedValue' LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            assert!(reference.0.is_some());
            assert!(reference.1 >= 0.9);
            Ok(())
        })
        .expect("graph should query");
}

#[test]
fn incremental_runs_skip_unchanged_files_and_retire_deleted_paths() {
    let fixture = index_fixture("incremental");
    let indexer = Indexer::new(fixture.storage.clone());
    indexer
        .index_repository(&fixture.repository_id, &CancellationToken::new())
        .expect("initial index should finish");

    let unchanged = indexer
        .index_repository(&fixture.repository_id, &CancellationToken::new())
        .expect("unchanged index should finish");
    assert_eq!(unchanged.indexed_files, 0);
    assert_eq!(unchanged.skipped_files, 8);

    fs::write(
        fixture.root.join("src/math.ts"),
        "export function changed(): number { return 7; }\n",
    )
    .expect("changed source should write");
    fs::remove_file(fixture.root.join("src/constants.ts")).expect("source should delete");
    let changed = indexer
        .index_repository(&fixture.repository_id, &CancellationToken::new())
        .expect("changed index should finish");
    assert_eq!(changed.indexed_files, 1);
    assert_eq!(changed.skipped_files, 6);
    assert_eq!(changed.deleted_files, 1);

    let deleted_at: Option<String> = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT deleted_at FROM files WHERE relative_path = 'src/constants.ts'",
                [],
                |row| row.get(0),
            )
        })
        .expect("deleted file should query");
    assert!(deleted_at.is_some());

    fs::rename(
        fixture.root.join("src/base.ts"),
        fixture.root.join("src/foundation.ts"),
    )
    .expect("source should rename");
    let renamed = indexer
        .index_repository(&fixture.repository_id, &CancellationToken::new())
        .expect("renamed index should finish");
    // The move is recognised as a rename, not as a delete plus an add.
    assert_eq!(renamed.renamed_files, 1);
    assert_eq!(renamed.deleted_files, 0);
    assert_eq!(renamed.indexed_files, 1);
    assert_eq!(renamed.skipped_files, 6);
}
