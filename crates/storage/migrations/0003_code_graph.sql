CREATE TABLE index_runs (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'failed', 'interrupted')),
    cursor_json TEXT,
    files_seen INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT
) STRICT;

CREATE TABLE files (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    language TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    byte_size INTEGER NOT NULL,
    mtime_ns INTEGER,
    parse_errors INTEGER NOT NULL DEFAULT 0,
    indexed_at TEXT NOT NULL,
    deleted_at TEXT,
    UNIQUE(repository_id, relative_path)
) STRICT;

CREATE INDEX files_active_path ON files(repository_id, relative_path, deleted_at);

CREATE TABLE symbols (
    id TEXT PRIMARY KEY,
    file_id TEXT NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    stable_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    qualified_name TEXT,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    signature TEXT,
    exported INTEGER NOT NULL DEFAULT 0 CHECK (exported IN (0, 1)),
    content_hash TEXT NOT NULL,
    UNIQUE(file_id, stable_key)
) STRICT;

CREATE INDEX symbols_name ON symbols(name, kind);
CREATE INDEX symbols_file_span ON symbols(file_id, start_byte, end_byte);

CREATE TABLE code_edges (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    source_kind TEXT NOT NULL,
    source_id TEXT NOT NULL,
    edge_kind TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    target_id TEXT,
    target_key TEXT NOT NULL,
    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
    evidence_json TEXT NOT NULL,
    index_run_id TEXT NOT NULL REFERENCES index_runs(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX code_edges_source ON code_edges(source_kind, source_id, edge_kind);
CREATE INDEX code_edges_target ON code_edges(target_kind, target_id, target_key, edge_kind);
CREATE UNIQUE INDEX code_edges_identity ON code_edges(
    source_kind,
    source_id,
    edge_kind,
    target_kind,
    IFNULL(target_id, ''),
    target_key
);
