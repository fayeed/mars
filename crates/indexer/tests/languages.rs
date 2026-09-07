mod support;

use mars_indexer::Indexer;
use tokio_util::sync::CancellationToken;

use support::polyglot_fixture;

/// One row per definition the extractors must find, so a grammar regression in
/// any language shows up as a named failure rather than a count mismatch.
#[test]
fn every_supported_language_yields_definitions_with_enclosure_and_documentation() {
    let fixture = polyglot_fixture("languages");
    Indexer::new(fixture.storage.clone())
        .index_repository(&fixture.repository_id, &CancellationToken::new())
        .expect("polyglot fixture should index");

    /// path, name, kind, enclosing symbol, documentation prefix
    type Expectation = (
        &'static str,
        &'static str,
        &'static str,
        Option<&'static str>,
        Option<&'static str>,
    );

    let expectations: &[Expectation] = &[
        (
            "src/service.py",
            "apply_tax",
            "function",
            None,
            Some("Applies the standard tax rate"),
        ),
        (
            "src/service.py",
            "InvoiceBuilder",
            "class",
            None,
            Some("Builds invoices from line items."),
        ),
        (
            "src/service.py",
            "render",
            "method",
            Some("InvoiceBuilder"),
            Some("Serialises the invoice as JSON."),
        ),
        (
            "src/server.go",
            "NewRouter",
            "function",
            None,
            Some("NewRouter builds an empty router."),
        ),
        (
            "src/server.go",
            "Router",
            "struct",
            None,
            Some("Router dispatches requests to handlers."),
        ),
        ("src/server.go", "Serve", "method", Some("Handler"), None),
        (
            "src/engine.rs",
            "MAX_ATTEMPTS",
            "constant",
            None,
            Some("Maximum attempts before the engine gives up."),
        ),
        (
            "src/engine.rs",
            "Backoff",
            "interface",
            None,
            Some("Describes how a retry backs off."),
        ),
        (
            "src/engine.rs",
            "Outcome",
            "enum",
            None,
            Some("Retry outcomes reported to callers."),
        ),
        (
            "src/engine.rs",
            "run_with_retries",
            "function",
            None,
            Some("Runs an operation until it succeeds"),
        ),
        (
            "package.json",
            "left-pad",
            "property",
            Some("dependencies"),
            None,
        ),
        (
            "config/settings.yaml",
            "backoff",
            "property",
            Some("retries"),
            None,
        ),
        ("README.md", "Retries", "heading", Some("Services"), None),
    ];

    for (path, name, kind, parent, documentation) in expectations {
        let row: (String, Option<String>, Option<String>) = fixture
            .storage
            .with_connection(|connection| {
                connection.query_row(
                    "SELECT s.kind, p.name, s.documentation
                     FROM symbols s JOIN files f ON f.id = s.file_id
                     LEFT JOIN symbols p ON p.id = s.parent_symbol_id
                     WHERE f.relative_path = ?1 AND s.name = ?2",
                    rusqlite::params![path, name],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
            })
            .unwrap_or_else(|error| panic!("{path}:{name} should be indexed: {error}"));

        assert_eq!(&row.0.as_str(), kind, "{path}:{name} kind");
        assert_eq!(row.1.as_deref(), *parent, "{path}:{name} enclosing symbol");
        if let Some(expected) = documentation {
            let actual = row
                .2
                .unwrap_or_else(|| panic!("{path}:{name} should carry documentation"));
            assert!(
                actual.starts_with(expected),
                "{path}:{name} documentation was `{actual}`"
            );
        }
    }
}

#[test]
fn export_visibility_follows_each_language_convention() {
    let fixture = polyglot_fixture("visibility");
    Indexer::new(fixture.storage.clone())
        .index_repository(&fixture.repository_id, &CancellationToken::new())
        .expect("polyglot fixture should index");

    // Go exports by capitalisation, Rust by `pub`, Python by leading underscore.
    let cases = [
        ("src/server.go", "NewRouter", true),
        ("src/engine.rs", "run_with_retries", true),
        ("src/engine.rs", "unexported_helper", false),
        ("src/service.py", "apply_tax", true),
        ("src/service.py", "__init__", false),
    ];

    for (path, name, exported) in cases {
        let actual: i64 = fixture
            .storage
            .with_connection(|connection| {
                connection.query_row(
                    "SELECT s.exported FROM symbols s JOIN files f ON f.id = s.file_id
                     WHERE f.relative_path = ?1 AND s.name = ?2",
                    rusqlite::params![path, name],
                    |row| row.get(0),
                )
            })
            .unwrap_or_else(|error| panic!("{path}:{name} should be indexed: {error}"));
        assert_eq!(actual == 1, exported, "{path}:{name} export visibility");
    }
}

#[test]
fn imports_and_manifest_dependencies_become_edges() {
    let fixture = polyglot_fixture("edges");
    Indexer::new(fixture.storage.clone())
        .index_repository(&fixture.repository_id, &CancellationToken::new())
        .expect("polyglot fixture should index");

    let edges: Vec<(String, String, String)> = fixture
        .storage
        .with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT f.relative_path, e.edge_kind, e.target_key
                 FROM code_edges e JOIN files f ON f.id = e.source_id
                 WHERE e.source_kind = 'file'
                 ORDER BY f.relative_path, e.target_key",
            )?;
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect()
        })
        .expect("file edges should query");

    let has = |path: &str, kind: &str, target: &str| {
        edges
            .iter()
            .any(|edge| edge.0 == path && edge.1 == kind && edge.2 == target)
    };

    assert!(has("src/service.py", "IMPORTS", "json"), "{edges:?}");
    assert!(has("src/service.py", "IMPORTS", "decimal"), "{edges:?}");
    assert!(has("src/server.go", "IMPORTS", "errors"), "{edges:?}");
    assert!(
        has("src/engine.rs", "IMPORTS", "std::time::Duration"),
        "{edges:?}"
    );
    // A manifest names real external packages, not modules resolved in-tree.
    assert!(has("package.json", "DEPENDS_ON", "left-pad"), "{edges:?}");
    assert!(has("package.json", "DEPENDS_ON", "vitest"), "{edges:?}");
}

#[test]
fn binary_and_oversized_files_join_the_file_layer_without_symbols() {
    let fixture = polyglot_fixture("binary");
    Indexer::new(fixture.storage.clone())
        .index_repository(&fixture.repository_id, &CancellationToken::new())
        .expect("polyglot fixture should index");

    let (language, is_binary, blob_sha, symbols): (String, i64, Option<String>, i64) = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT f.language, f.is_binary, f.blob_sha,
                        (SELECT COUNT(*) FROM symbols s WHERE s.file_id = f.id)
                 FROM files f WHERE f.relative_path = 'assets/logo.bin'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
        })
        .expect("binary file should be indexed");

    assert_eq!(language, "binary");
    assert_eq!(is_binary, 1);
    assert_eq!(symbols, 0);
    // Even an unparsed file carries the Git identity it was derived from.
    assert_eq!(blob_sha.map(|sha| sha.len()), Some(40));
}
