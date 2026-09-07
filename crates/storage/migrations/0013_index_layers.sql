-- Files layer: git identity, binary marking, and rename provenance.
ALTER TABLE files ADD COLUMN blob_sha TEXT;
ALTER TABLE files ADD COLUMN is_binary INTEGER NOT NULL DEFAULT 0 CHECK (is_binary IN (0, 1));
ALTER TABLE files ADD COLUMN renamed_from TEXT;

CREATE INDEX files_blob_sha ON files(repository_id, blob_sha);

-- Symbols layer: enclosing symbol, leading documentation, and derived-from blob.
ALTER TABLE symbols ADD COLUMN parent_stable_key TEXT;
ALTER TABLE symbols ADD COLUMN parent_symbol_id TEXT;
ALTER TABLE symbols ADD COLUMN documentation TEXT;
ALTER TABLE symbols ADD COLUMN blob_sha TEXT;

CREATE INDEX symbols_parent ON symbols(parent_symbol_id);

-- Graph layer: every edge records the blob it was derived from.
ALTER TABLE code_edges ADD COLUMN blob_sha TEXT;

-- Run accounting for index.status().
ALTER TABLE index_runs ADD COLUMN mode TEXT NOT NULL DEFAULT 'full';
ALTER TABLE index_runs ADD COLUMN base_head_oid TEXT;
ALTER TABLE index_runs ADD COLUMN indexed_files INTEGER NOT NULL DEFAULT 0;
ALTER TABLE index_runs ADD COLUMN skipped_files INTEGER NOT NULL DEFAULT 0;
ALTER TABLE index_runs ADD COLUMN deleted_files INTEGER NOT NULL DEFAULT 0;
ALTER TABLE index_runs ADD COLUMN renamed_files INTEGER NOT NULL DEFAULT 0;
ALTER TABLE index_runs ADD COLUMN chunk_count INTEGER NOT NULL DEFAULT 0;

-- Ignore status is recorded per pruned entry so consumers can explain exclusions.
CREATE TABLE ignored_paths (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    entry_kind TEXT NOT NULL CHECK (entry_kind IN ('file', 'directory')),
    reason TEXT NOT NULL CHECK (reason IN (
        'gitignore',
        'marsignore',
        'mars-default-directory',
        'sensitive-file',
        'too-large'
    )),
    detected_at TEXT NOT NULL,
    UNIQUE(repository_id, relative_path)
) STRICT;

CREATE INDEX ignored_paths_repository ON ignored_paths(repository_id, relative_path);

-- Chunk layer: symbol-aware spans with stable identity.
CREATE TABLE chunks (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    file_id TEXT NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    symbol_id TEXT REFERENCES symbols(id) ON DELETE SET NULL,
    ordinal INTEGER NOT NULL,
    symbol_path TEXT NOT NULL,
    header TEXT NOT NULL,
    body TEXT NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    blob_sha TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    indexed_at TEXT NOT NULL,
    UNIQUE(file_id, ordinal)
) STRICT;

CREATE INDEX chunks_repository ON chunks(repository_id);
CREATE INDEX chunks_symbol ON chunks(symbol_id);

CREATE TABLE chunk_embeddings (
    chunk_id TEXT PRIMARY KEY REFERENCES chunks(id) ON DELETE CASCADE,
    repository_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    model TEXT NOT NULL,
    dimensions INTEGER NOT NULL,
    vector BLOB NOT NULL,
    blob_sha TEXT NOT NULL
) STRICT;

CREATE INDEX chunk_embeddings_repository ON chunk_embeddings(repository_id, model);

CREATE VIRTUAL TABLE chunk_search USING fts5(
    chunk_id UNINDEXED,
    repository_id UNINDEXED,
    relative_path,
    header,
    body,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Git history layer.
CREATE TABLE commits (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    oid TEXT NOT NULL,
    parent_oids TEXT NOT NULL,
    author_name TEXT NOT NULL,
    author_email TEXT NOT NULL,
    authored_at TEXT NOT NULL,
    committed_at TEXT NOT NULL,
    subject TEXT NOT NULL,
    UNIQUE(repository_id, oid)
) STRICT;

CREATE INDEX commits_recency ON commits(repository_id, committed_at DESC);

CREATE TABLE commit_files (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    commit_id TEXT NOT NULL REFERENCES commits(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    previous_path TEXT,
    change_kind TEXT NOT NULL CHECK (change_kind IN (
        'added',
        'modified',
        'deleted',
        'renamed',
        'copied',
        'type-changed'
    )),
    insertions INTEGER NOT NULL,
    deletions INTEGER NOT NULL,
    UNIQUE(commit_id, relative_path)
) STRICT;

CREATE INDEX commit_files_path ON commit_files(repository_id, relative_path);

-- Per-repository index state backing index.status().
CREATE TABLE index_state (
    repository_id TEXT PRIMARY KEY REFERENCES repositories(id) ON DELETE CASCADE,
    head_oid TEXT,
    branch TEXT,
    history_head_oid TEXT,
    last_run_id TEXT,
    last_completed_at TEXT,
    updated_at TEXT NOT NULL
) STRICT;
