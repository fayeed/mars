CREATE TABLE agents (
    id TEXT PRIMARY KEY,
    adapter_kind TEXT NOT NULL,
    display_name TEXT NOT NULL,
    executable_path TEXT NOT NULL,
    version TEXT,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(adapter_kind, executable_path)
) STRICT;

CREATE TABLE agent_sessions (
    id TEXT PRIMARY KEY,
    task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    worktree_id TEXT REFERENCES worktrees(id) ON DELETE SET NULL,
    adapter_kind TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('running', 'cancelling', 'completed', 'failed', 'cancelled', 'interrupted')),
    command_json TEXT NOT NULL CHECK (json_valid(command_json)),
    pid INTEGER,
    exit_code INTEGER,
    error TEXT,
    created_at TEXT NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT
) STRICT;

CREATE INDEX agent_sessions_task_time ON agent_sessions(task_id, created_at, id);

CREATE TABLE agent_events (
    session_id TEXT NOT NULL REFERENCES agent_sessions(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    occurred_at TEXT NOT NULL,
    PRIMARY KEY(session_id, sequence)
) STRICT;

CREATE TABLE permission_decisions (
    id TEXT PRIMARY KEY,
    session_id TEXT REFERENCES agent_sessions(id) ON DELETE CASCADE,
    request_id TEXT NOT NULL,
    capability TEXT NOT NULL,
    scope_json TEXT NOT NULL CHECK (json_valid(scope_json)),
    decision TEXT NOT NULL CHECK (decision IN ('deny', 'ask', 'allow_once', 'allow_session')),
    decided_at TEXT NOT NULL
) STRICT;

CREATE INDEX permission_decisions_session_time ON permission_decisions(session_id, decided_at, id);
