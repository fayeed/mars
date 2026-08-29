CREATE TABLE skills (
    id TEXT PRIMARY KEY,
    canonical_name TEXT NOT NULL,
    source TEXT NOT NULL,
    slug TEXT NOT NULL,
    installs INTEGER NOT NULL DEFAULT 0,
    content_hash TEXT NOT NULL,
    registry_hash TEXT NOT NULL,
    trust_state TEXT NOT NULL CHECK (trust_state IN ('inspected', 'trusted', 'blocked')),
    skill_markdown TEXT NOT NULL,
    snapshot_json TEXT NOT NULL CHECK (json_valid(snapshot_json)),
    inspected_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(source, canonical_name)
) STRICT;

CREATE TABLE skill_installations (
    id TEXT PRIMARY KEY,
    skill_id TEXT NOT NULL REFERENCES skills(id) ON DELETE CASCADE,
    repository_id TEXT REFERENCES repositories(id) ON DELETE CASCADE,
    task_id TEXT REFERENCES tasks(id) ON DELETE CASCADE,
    scope TEXT NOT NULL CHECK (scope IN ('global', 'project', 'task', 'workflow')),
    install_path TEXT NOT NULL,
    pinned_hash TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    installed_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(skill_id, repository_id, task_id, scope)
) STRICT;

CREATE INDEX skill_installations_scope
    ON skill_installations(scope, repository_id, task_id, enabled);

CREATE TABLE skill_agent_policies (
    installation_id TEXT NOT NULL REFERENCES skill_installations(id) ON DELETE CASCADE,
    adapter_kind TEXT NOT NULL CHECK (adapter_kind IN ('claude', 'codex')),
    allowed INTEGER NOT NULL CHECK (allowed IN (0, 1)),
    PRIMARY KEY(installation_id, adapter_kind)
) STRICT;
