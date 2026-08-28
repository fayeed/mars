# Local data and backup

Status: Milestone 0 storage foundation

## Database

Mars uses one bundled SQLite database per local application profile. The desktop will resolve the profile directory through Tauri's application-data API and store the database as `mars.sqlite3` beneath the `dev.mars.desktop` application directory. `mars-storage` currently accepts that path explicitly; wiring the desktop path resolver is deferred until the first repository-opening slice.

Every connection enables foreign keys, a five-second busy timeout, WAL journaling, and `synchronous=NORMAL`. Mars bundles SQLite through `rusqlite`; the test suite creates an FTS5 virtual table to prove that the shipped build includes FTS5 instead of relying on the host SQLite binary.

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

Source repositories and Git worktrees are not stored in this database and require their normal backup strategy.

## Recovery behavior

On startup, any job left in `running` or `cancel_requested` state is marked `interrupted`; its ordered events remain available for diagnosis. Queued and terminal jobs are unchanged. Recovery never deletes a worktree or source file.

Current limitations:

- The desktop does not yet open the profile database.
- There is no online backup, restore, compaction, or artifact garbage collection UI.
- Jobs persist status/events but do not yet expose checkpoint update APIs; those arrive with the first resumable indexer.
