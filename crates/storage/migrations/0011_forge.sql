CREATE TABLE remotes (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    fetch_url TEXT NOT NULL,
    provider TEXT NOT NULL CHECK (provider IN ('github', 'gitlab', 'bitbucket', 'gitea', 'forgejo', 'unknown')),
    host TEXT NOT NULL,
    owner TEXT NOT NULL,
    repository_name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(repository_id, name)
) STRICT;

CREATE TABLE forge_connections (
    id TEXT PRIMARY KEY,
    remote_id TEXT NOT NULL UNIQUE REFERENCES remotes(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    api_base TEXT NOT NULL,
    account_login TEXT,
    credential_ref TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE issues (
    id TEXT PRIMARY KEY,
    remote_id TEXT NOT NULL REFERENCES remotes(id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL,
    number INTEGER NOT NULL CHECK (number > 0),
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    state TEXT NOT NULL,
    url TEXT NOT NULL,
    author TEXT NOT NULL,
    labels_json TEXT NOT NULL CHECK (json_valid(labels_json)),
    provider_updated_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(remote_id, provider_id),
    UNIQUE(remote_id, number)
) STRICT;

CREATE TABLE issue_task_links (
    issue_id TEXT NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    task_id TEXT NOT NULL UNIQUE REFERENCES tasks(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY(issue_id, task_id)
) STRICT;

CREATE TABLE pull_requests (
    id TEXT PRIMARY KEY,
    remote_id TEXT NOT NULL REFERENCES remotes(id) ON DELETE CASCADE,
    task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    issue_id TEXT REFERENCES issues(id) ON DELETE SET NULL,
    provider_id TEXT NOT NULL,
    number INTEGER NOT NULL CHECK (number > 0),
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    state TEXT NOT NULL,
    url TEXT NOT NULL,
    head TEXT NOT NULL,
    base TEXT NOT NULL,
    draft INTEGER NOT NULL CHECK (draft IN (0, 1)),
    mergeable INTEGER CHECK (mergeable IS NULL OR mergeable IN (0, 1)),
    head_sha TEXT NOT NULL,
    provider_updated_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(remote_id, provider_id),
    UNIQUE(remote_id, number)
) STRICT;

CREATE INDEX pull_requests_task ON pull_requests(task_id, updated_at);

CREATE TABLE review_comments (
    id TEXT PRIMARY KEY,
    pull_request_id TEXT NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL,
    author TEXT NOT NULL,
    body TEXT NOT NULL,
    relative_path TEXT,
    line INTEGER CHECK (line IS NULL OR line > 0),
    url TEXT NOT NULL,
    provider_created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(pull_request_id, provider_id)
) STRICT;

CREATE TABLE check_runs (
    id TEXT PRIMARY KEY,
    pull_request_id TEXT NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    conclusion TEXT,
    url TEXT NOT NULL,
    started_at TEXT,
    completed_at TEXT,
    updated_at TEXT NOT NULL,
    UNIQUE(pull_request_id, provider_id)
) STRICT;
