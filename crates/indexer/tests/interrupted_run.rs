mod support;

use mars_indexer::{IndexError, Indexer};
use tokio_util::sync::CancellationToken;

use support::index_fixture;

#[test]
fn interrupted_runs_publish_no_partial_file_replacements() {
    let fixture = index_fixture("interrupted");
    let indexer = Indexer::new(fixture.storage.clone());
    let cancellation = CancellationToken::new();

    let error = indexer
        .index_repository_observing(
            &fixture.repository_id,
            &cancellation,
            |files_seen, _path| {
                if files_seen == 1 {
                    cancellation.cancel();
                }
            },
        )
        .expect_err("cancelled index should stop");
    assert!(matches!(error, IndexError::Cancelled { .. }));

    fixture
        .storage
        .with_connection(|connection| {
            let files: i64 =
                connection.query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))?;
            let symbols: i64 =
                connection.query_row("SELECT COUNT(*) FROM symbols", [], |row| row.get(0))?;
            let interrupted: i64 = connection.query_row(
                "SELECT COUNT(*) FROM index_runs WHERE status = 'interrupted'",
                [],
                |row| row.get(0),
            )?;
            assert_eq!((files, symbols, interrupted), (0, 0, 1));
            Ok(())
        })
        .expect("interrupted state should query");
}
