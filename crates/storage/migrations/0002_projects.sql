CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE repositories (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    root_path TEXT NOT NULL UNIQUE,
    git_common_dir TEXT NOT NULL,
    default_branch TEXT,
    head_oid TEXT,
    state TEXT NOT NULL CHECK (state IN ('active', 'missing', 'untrusted')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
