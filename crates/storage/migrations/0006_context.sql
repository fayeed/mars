CREATE TABLE context_artifacts (
    digest TEXT PRIMARY KEY,
    storage_key TEXT NOT NULL UNIQUE,
    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
    mime_type TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT;

CREATE TABLE context_items (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('text', 'image', 'file')),
    title TEXT NOT NULL,
    body_text TEXT,
    original_uri TEXT,
    mime_type TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    classification TEXT,
    quarantine_reason TEXT,
    captured_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(repository_id, content_hash, kind)
) STRICT;

CREATE INDEX context_items_repository_time
    ON context_items(repository_id, captured_at DESC, id);

CREATE TABLE context_item_artifacts (
    context_item_id TEXT PRIMARY KEY REFERENCES context_items(id) ON DELETE CASCADE,
    artifact_digest TEXT NOT NULL REFERENCES context_artifacts(digest),
    original_name TEXT NOT NULL
) STRICT;

CREATE TABLE context_links (
    context_item_id TEXT NOT NULL REFERENCES context_items(id) ON DELETE CASCADE,
    entity_kind TEXT NOT NULL CHECK (entity_kind IN ('task', 'repository')),
    entity_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(context_item_id, entity_kind, entity_id)
) STRICT;

CREATE INDEX context_links_entity ON context_links(entity_kind, entity_id, context_item_id);

CREATE VIRTUAL TABLE context_fts USING fts5(
    context_item_id UNINDEXED,
    title,
    body_text,
    tokenize = 'unicode61'
);
