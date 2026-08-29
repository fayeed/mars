CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    repository_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    intent TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('planned', 'provisioning', 'ready', 'active', 'review', 'completed', 'failed', 'cancelled')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE branches (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    task_id TEXT NOT NULL UNIQUE REFERENCES tasks(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    base_ref TEXT NOT NULL,
    head_oid TEXT,
    state TEXT NOT NULL CHECK (state IN ('planned', 'created', 'removed', 'failed')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(repository_id, name)
) STRICT;

CREATE TABLE worktrees (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    task_id TEXT NOT NULL UNIQUE REFERENCES tasks(id) ON DELETE CASCADE,
    branch_id TEXT NOT NULL UNIQUE REFERENCES branches(id) ON DELETE CASCADE,
    path TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL CHECK (state IN ('planned', 'provisioning', 'ready', 'removing', 'removed', 'orphaned', 'failed')),
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE INDEX worktrees_repository_state ON worktrees(repository_id, state);

CREATE TABLE git_mutations (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    operation TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('started', 'completed', 'rolled_back', 'failed')),
    details_json TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT
) STRICT;

CREATE INDEX git_mutations_task_order ON git_mutations(task_id, started_at, id);
