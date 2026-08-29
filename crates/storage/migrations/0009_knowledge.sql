CREATE TABLE provenance (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    repository_id TEXT REFERENCES repositories(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN (
        'human', 'extracted_code', 'extracted_git', 'agent_observed',
        'agent_inferred', 'imported', 'ambiguous'
    )),
    source_type TEXT NOT NULL,
    source_id TEXT NOT NULL,
    evidence_json TEXT NOT NULL CHECK (json_valid(evidence_json)),
    observed_at TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT;

CREATE INDEX provenance_project_kind ON provenance(project_id, kind, created_at, id);

CREATE TABLE knowledge_records (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    repository_id TEXT REFERENCES repositories(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN (
        'summary', 'decision', 'constraint', 'attempt', 'risk', 'question', 'test', 'commit'
    )),
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('candidate', 'approved', 'rejected', 'stale', 'superseded')),
    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
    provenance_id TEXT NOT NULL REFERENCES provenance(id),
    resolution_provenance_id TEXT REFERENCES provenance(id),
    supersedes_id TEXT REFERENCES knowledge_records(id),
    pinned INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE INDEX knowledge_project_status_authority
    ON knowledge_records(project_id, status, kind, confidence DESC, created_at, id);
CREATE INDEX knowledge_supersedes ON knowledge_records(supersedes_id);

CREATE TABLE entity_links (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    source_type TEXT NOT NULL,
    source_id TEXT NOT NULL,
    relation TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    provenance_id TEXT NOT NULL REFERENCES provenance(id),
    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
    created_at TEXT NOT NULL,
    UNIQUE(project_id, source_type, source_id, relation, target_type, target_id)
) STRICT;

CREATE INDEX entity_links_out ON entity_links(project_id, source_type, source_id, relation);
CREATE INDEX entity_links_in ON entity_links(project_id, target_type, target_id, relation);
