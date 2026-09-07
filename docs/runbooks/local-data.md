# Local data and backup

Status: Milestone A repository foundation

## Database

Mars uses one bundled SQLite database per local application profile. At startup, the desktop resolves the profile directory through Tauri's application-data API, creates it when needed, and opens `mars.sqlite3` beneath the `dev.mars.desktop` application directory. Startup applies pending migrations before commands become available and fails rather than running against an unavailable or invalid database.

Every connection enables foreign keys, a five-second busy timeout, WAL journaling, and `synchronous=NORMAL`. Mars bundles SQLite through `rusqlite`; the test suite creates an FTS5 virtual table to prove that the shipped build includes FTS5 instead of relying on the host SQLite binary.

The database also contains the rebuildable repository index: files, symbols, graph edges, symbol-aware chunks with their local embeddings, recorded ignore decisions, and ingested Git commit and churn history. Index runs publish changed files transactionally; interrupted runs retain diagnostic state without exposing partial replacements. Source files and Git remain authoritative and can be reindexed at any time.

Every indexed row records the Git blob SHA it was derived from. A consumer can ask whether a path is still current, and answers that rest on changed files are labelled stale rather than served as though they were fresh. Nothing about the index is written inside the user's repository.

Related files may sit beside the database:

- `mars.sqlite3-wal` — committed pages not yet checkpointed into the main file.
- `mars.sqlite3-shm` — WAL shared-memory coordination.
- `artifacts/` — future content-addressed Context Library originals; not implemented in Milestone 0.

## Migrations

Migrations are ordered, append-only SQL files under `crates/storage/migrations/`. Each migration runs in a transaction and is recorded in `_mars_migrations` with its version, name, SHA-256 checksum, and RFC 3339 application timestamp. Startup refuses to accept a changed checksum for an applied migration.

Do not edit a migration after it has shipped. Add the next numbered migration.

## Safe backup

Milestone 0 does not yet expose a backup command. Until it does:

1. Exit Mars and confirm no background Mars process remains.
2. Copy `mars.sqlite3` and the future `artifacts/` directory together.
3. If the process cannot be stopped, use SQLite's online backup API rather than copying only the main database while WAL mode is active. Online backup is planned before user data enters the database.

Source repositories and Git worktrees are not stored in this database and require their normal backup strategy. Project records contain canonical local paths and resolved Git common-directory paths; restoring a database on another machine does not copy or relocate those repositories.

## Recovery behavior

On startup, any job left in `running` or `cancel_requested` state is marked `interrupted`; its ordered events remain available for diagnosis. A forge mutation left in `started` is likewise marked `interrupted` without claiming the remote operation succeeded. Queued and terminal jobs are unchanged. Recovery never deletes a worktree, source file, branch, or hosted object.

Current limitations:

- There is no online backup, restore, compaction, or artifact garbage collection UI.
- Jobs persist status/events but do not yet expose checkpoint update APIs; those arrive with the first resumable indexer.
- Embedding vectors are keyed by the model that produced them (`mars-hashing-v1`). Replacing the embedder requires re-embedding the chunk layer, which a full reindex does.
