//! The Phase 0 acceptance tests: rename detection, cold-index timing at scale,
//! and delta correctness after a branch switch.

mod support;

use std::fs;
use std::time::Instant;

use mars_indexer::api::IndexApi;
use mars_indexer::watcher::{RepositoryWatcher, WatchEvent, WatchOptions};
use mars_indexer::{IndexEvent, Indexer};
use tokio_util::sync::CancellationToken;

use support::{IndexFixture, git, polyglot_fixture, scale_fixture};

#[test]
fn a_renamed_file_keeps_its_identity_and_moves_its_symbols() {
    let fixture = polyglot_fixture("rename");
    let indexer = Indexer::new(fixture.storage.clone());
    indexer
        .index_repository(&fixture.repository_id, &CancellationToken::new())
        .expect("first index should finish");

    let before = file_identity(&fixture, "src/engine.rs");
    let before_symbols = symbol_identities(&fixture, "src/engine.rs");
    assert!(!before_symbols.is_empty());

    git(&fixture.root, &["mv", "src/engine.rs", "src/retry.rs"]);

    let summary = indexer
        .index_repository(&fixture.repository_id, &CancellationToken::new())
        .expect("second index should finish");

    assert_eq!(summary.renamed_files, 1, "the move is detected as a rename");
    assert_eq!(summary.deleted_files, 0, "nothing is retired by a rename");

    let after = file_identity(&fixture, "src/retry.rs");
    assert_eq!(before.0, after.0, "the file keeps its id across the move");
    assert_eq!(
        before.1, after.1,
        "content is unchanged, so the blob matches"
    );

    let renamed_from: Option<String> = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT renamed_from FROM files WHERE relative_path = 'src/retry.rs'",
                [],
                |row| row.get(0),
            )
        })
        .expect("rename provenance should query");
    assert_eq!(renamed_from.as_deref(), Some("src/engine.rs"));

    // Symbol ids survive the move, so anything referring to them stays valid.
    let after_symbols = symbol_identities(&fixture, "src/retry.rs");
    assert_eq!(before_symbols, after_symbols);

    // The old path is gone from the active index, and chunk headers follow the
    // file to its new location.
    let old_rows: i64 = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT COUNT(*) FROM files
                 WHERE relative_path = 'src/engine.rs' AND deleted_at IS NULL",
                [],
                |row| row.get(0),
            )
        })
        .expect("old path should query");
    assert_eq!(old_rows, 0);

    let stale_headers: i64 = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT COUNT(*) FROM chunks WHERE header LIKE 'src/engine.rs%'",
                [],
                |row| row.get(0),
            )
        })
        .expect("chunk headers should query");
    assert_eq!(stale_headers, 0, "chunk headers move with the file");
}

#[test]
fn a_branch_switch_leaves_the_index_matching_the_new_tree() {
    let fixture = polyglot_fixture("branch-switch");
    let api = IndexApi::new(fixture.storage.clone());
    api.reindex(&fixture.repository_id, None, &CancellationToken::new())
        .expect("first index should finish");

    // Build a branch that adds, edits, and deletes files.
    git(&fixture.root, &["checkout", "-q", "-b", "feature"]);
    fs::write(
        fixture.root.join("src/queue.go"),
        "package server\n\n// Queue buffers pending work.\ntype Queue struct{}\n",
    )
    .expect("new source should write");
    fs::write(
        fixture.root.join("src/service.py"),
        "\"\"\"Rewritten on the feature branch.\"\"\"\n\n\ndef only_on_feature():\n    return 1\n",
    )
    .expect("source should rewrite");
    fs::remove_file(fixture.root.join("src/server.go")).expect("source should delete");
    git(&fixture.root, &["add", "-A"]);
    git(&fixture.root, &["commit", "-m", "feature branch work"]);

    let feature = api
        .reindex(&fixture.repository_id, None, &CancellationToken::new())
        .expect("feature index should finish");
    assert_eq!(feature.deleted_files, 1, "{feature:?}");

    assert!(indexed_paths(&fixture).contains(&"src/queue.go".to_owned()));
    assert!(!indexed_paths(&fixture).contains(&"src/server.go".to_owned()));
    assert!(
        symbol_names(&fixture).contains(&"only_on_feature".to_owned()),
        "the rewritten file's symbols replace the old ones"
    );
    assert!(!symbol_names(&fixture).contains(&"NewRouter".to_owned()));

    // Switch back: the index must return to exactly the main-branch state.
    git(&fixture.root, &["checkout", "-q", "main"]);
    let back = api
        .reindex(&fixture.repository_id, None, &CancellationToken::new())
        .expect("main index should finish");

    assert_eq!(
        back.deleted_files, 1,
        "the branch-only file is retired: {back:?}"
    );
    let paths = indexed_paths(&fixture);
    assert!(!paths.contains(&"src/queue.go".to_owned()), "{paths:?}");
    assert!(paths.contains(&"src/server.go".to_owned()), "{paths:?}");

    let names = symbol_names(&fixture);
    assert!(names.contains(&"NewRouter".to_owned()), "{names:?}");
    assert!(!names.contains(&"only_on_feature".to_owned()), "{names:?}");

    // Nothing is left dangling: no symbol, chunk, or edge belongs to a retired
    // file, and every remaining row matches the working tree.
    let orphans: i64 = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT (SELECT COUNT(*) FROM symbols s JOIN files f ON f.id = s.file_id
                          WHERE f.deleted_at IS NOT NULL)
                      + (SELECT COUNT(*) FROM chunks c JOIN files f ON f.id = c.file_id
                          WHERE f.deleted_at IS NOT NULL)",
                [],
                |row| row.get(0),
            )
        })
        .expect("orphan check should query");
    assert_eq!(orphans, 0);

    let stale = api
        .stale_paths(&fixture.repository_id)
        .expect("stale paths should resolve");
    assert!(
        stale.is_empty(),
        "index matches the tree after the switch: {stale:?}"
    );

    let status = api
        .status(&fixture.repository_id)
        .expect("status should resolve");
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert!(status.head_matches_index);
}

/// Cold-index timing at the scale the phase calls out. The budget is deliberately
/// loose: it is a regression guard against an accidental quadratic, not a
/// benchmark of the host.
#[test]
fn a_five_thousand_file_repository_indexes_cold_within_budget() {
    let fixture = scale_fixture("cold-index", 5_000);
    let indexer = Indexer::new(fixture.storage.clone());

    let mut seen_files = 0_usize;
    let started = Instant::now();
    let summary = indexer
        .run(
            &fixture.repository_id,
            &mars_indexer::IndexOptions::default(),
            &CancellationToken::new(),
            |event| {
                if matches!(event, IndexEvent::FileIndexed { .. }) {
                    seen_files += 1;
                }
            },
        )
        .expect("cold index should finish");
    let cold = started.elapsed();

    assert_eq!(summary.indexed_files, 5_000);
    assert_eq!(seen_files, 5_000, "progress is reported for every file");
    assert!(summary.chunk_count >= 5_000);
    println!(
        "cold index: {} files in {:?} ({} chunks)",
        summary.indexed_files, cold, summary.chunk_count
    );
    assert!(cold.as_secs() < 120, "cold index of 5k files took {cold:?}");

    // A second pass over unchanged content skips everything and is far cheaper.
    let started = Instant::now();
    let warm_summary = indexer
        .index_repository(&fixture.repository_id, &CancellationToken::new())
        .expect("warm index should finish");
    let warm = started.elapsed();

    assert_eq!(warm_summary.indexed_files, 0);
    assert_eq!(warm_summary.skipped_files, 5_000);
    println!("warm index: {warm:?}");
    assert!(
        warm < cold,
        "an unchanged pass must be cheaper than the cold one: {warm:?} vs {cold:?}"
    );

    // A targeted re-index touches one file, not the tree.
    let started = Instant::now();
    fs::write(
        fixture.root.join("src/module_0042.ts"),
        "export function changed(): number {\n  return 7;\n}\n",
    )
    .expect("source should write");
    let targeted = indexer
        .reindex_paths(
            &fixture.repository_id,
            vec!["src/module_0042.ts".into()],
            &CancellationToken::new(),
        )
        .expect("targeted index should finish");
    println!("targeted index: {:?}", started.elapsed());

    assert_eq!(targeted.mode, "paths");
    assert_eq!(targeted.indexed_files, 1);
    assert_eq!(targeted.deleted_files, 0);
    assert!(
        symbol_names(&fixture).contains(&"changed".to_owned()),
        "the targeted run published the edit"
    );
}

#[test]
fn the_watcher_debounces_a_burst_into_one_targeted_reindex() {
    let fixture = polyglot_fixture("watcher");
    Indexer::new(fixture.storage.clone())
        .index_repository(&fixture.repository_id, &CancellationToken::new())
        .expect("first index should finish");

    let (sender, receiver) = std::sync::mpsc::channel();
    let watcher = RepositoryWatcher::start(
        fixture.storage.clone(),
        &fixture.repository_id,
        &fixture.root,
        WatchOptions::default(),
        move |event| {
            let _ = sender.send(event.clone());
        },
    )
    .expect("watch should start");

    // A burst of writes to one file, the way an editor saves.
    let target = fixture.root.join("src/service.py");
    for attempt in 0..4 {
        fs::write(
            &target,
            format!("\"\"\"Edited.\"\"\"\n\n\ndef watched_{attempt}():\n    return {attempt}\n"),
        )
        .expect("source should write");
        std::thread::sleep(std::time::Duration::from_millis(30));
    }

    let mut indexed = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        match receiver.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(WatchEvent::Indexed { summary }) => {
                indexed.push(summary);
                break;
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }
    watcher.stop();

    let summary = indexed
        .first()
        .expect("the watcher should re-index the changed file");
    assert_eq!(
        summary.mode, "paths",
        "a change re-indexes only what changed"
    );
    assert!(summary.indexed_files >= 1);
    assert!(
        symbol_names(&fixture)
            .iter()
            .any(|name| name.starts_with("watched_")),
        "the watched edit reached the index"
    );
}

fn file_identity(fixture: &IndexFixture, path: &str) -> (String, Option<String>) {
    fixture
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT id, blob_sha FROM files WHERE relative_path = ?1 AND deleted_at IS NULL",
                [path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .unwrap_or_else(|error| panic!("{path} should be indexed: {error}"))
}

fn symbol_identities(fixture: &IndexFixture, path: &str) -> Vec<(String, String)> {
    fixture
        .storage
        .with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT s.id, s.stable_key FROM symbols s JOIN files f ON f.id = s.file_id
                 WHERE f.relative_path = ?1 ORDER BY s.stable_key",
            )?;
            statement
                .query_map([path], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .expect("symbols should query")
}

fn indexed_paths(fixture: &IndexFixture) -> Vec<String> {
    fixture
        .storage
        .with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT relative_path FROM files WHERE deleted_at IS NULL ORDER BY relative_path",
            )?;
            statement.query_map([], |row| row.get(0))?.collect()
        })
        .expect("files should query")
}

fn symbol_names(fixture: &IndexFixture) -> Vec<String> {
    fixture
        .storage
        .with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT s.name FROM symbols s JOIN files f ON f.id = s.file_id
                 WHERE f.deleted_at IS NULL",
            )?;
            statement.query_map([], |row| row.get(0))?.collect()
        })
        .expect("symbols should query")
}
