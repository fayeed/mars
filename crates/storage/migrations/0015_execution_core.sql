-- Phase 3 execution core: agent runs, the activity record, the permission
-- broker, and approved worktree plans.
--
-- The organising idea is that `activity_events` is the single source of truth
-- the UI renders from. Every other table here is either something that produces
-- events (a run, a permission request) or something a human approved before an
-- effect was allowed to happen (a grant, a worktree plan). Nothing here stores
-- derived UI state: reload, crash recovery, and export all replay the log.

-- Content-addressed payloads for anything too large to inline on an event:
-- command output, diffs, test results. The digest is the identity, so a repeated
-- payload is stored once and every event that produced it points at that row.
CREATE TABLE activity_payloads (
    digest TEXT PRIMARY KEY,
    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
    media_type TEXT NOT NULL,
    relative_path TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
) STRICT;

-- One agent execution. `agent_sessions` (migration 5) remains the process-level
-- record; a run is the task-level one that carries role, capability ceiling, and
-- the context pack the human actually approved.
CREATE TABLE agent_runs (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    session_id TEXT REFERENCES agent_sessions(id) ON DELETE SET NULL,
    agent_id TEXT NOT NULL,
    adapter_kind TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('implementer', 'reviewer')),
    capabilities_json TEXT NOT NULL CHECK (json_valid(capabilities_json)),
    context_pack_digest TEXT,
    status TEXT NOT NULL CHECK (status IN (
        'starting', 'running', 'blocked', 'interrupting',
        'completed', 'failed', 'cancelled', 'interrupted'
    )),
    last_sequence INTEGER NOT NULL DEFAULT 0 CHECK (last_sequence >= 0),
    error TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT
) STRICT;

CREATE INDEX agent_runs_task_time ON agent_runs(task_id, started_at, id);
CREATE INDEX agent_runs_status ON agent_runs(status, started_at, id);

-- The activity record. Append-only, per task, with a monotonic sequence.
CREATE TABLE activity_events (
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    id TEXT NOT NULL UNIQUE,
    run_id TEXT REFERENCES agent_runs(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN (
        'message', 'thought', 'tool_call', 'command', 'file_edit', 'test_run',
        'permission_request', 'permission_decision', 'error', 'done',
        'run_started', 'worktree_planned', 'worktree_created',
        'worktree_removed', 'worktree_restored', 'recovery_noted'
    )),
    actor TEXT NOT NULL CHECK (actor IN ('human', 'agent', 'system')),
    actor_label TEXT NOT NULL,
    summary TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    payload_digest TEXT REFERENCES activity_payloads(digest),
    occurred_at TEXT NOT NULL,
    duration_ms INTEGER CHECK (duration_ms >= 0),
    PRIMARY KEY(task_id, sequence)
) STRICT;

CREATE INDEX activity_events_run ON activity_events(run_id, sequence);
CREATE INDEX activity_events_kind ON activity_events(task_id, kind, sequence);

-- Append-only is a property of the table, not of the code that writes to it.
-- Rows may still leave when their task is deleted, which is why the delete
-- guard checks whether the parent task survived.
CREATE TRIGGER activity_events_are_immutable
BEFORE UPDATE ON activity_events
BEGIN
    SELECT RAISE(ABORT, 'activity events are append-only');
END;

CREATE TRIGGER activity_events_delete_only_with_task
BEFORE DELETE ON activity_events
WHEN EXISTS (SELECT 1 FROM tasks WHERE id = OLD.task_id)
BEGIN
    SELECT RAISE(ABORT, 'activity events are append-only');
END;

-- A grant is what a decision leaves behind. Scoped, optionally expiring, and
-- revocable; revocation is a timestamp rather than a delete so the transcript
-- keeps saying that the grant once existed.
CREATE TABLE permission_grants (
    id TEXT PRIMARY KEY,
    task_id TEXT REFERENCES tasks(id) ON DELETE CASCADE,
    capability TEXT NOT NULL,
    scope_json TEXT NOT NULL CHECK (json_valid(scope_json)),
    pattern TEXT,
    decision TEXT NOT NULL CHECK (decision IN ('allow_once', 'allow_task', 'allow_pattern')),
    uses_remaining INTEGER CHECK (uses_remaining >= 0),
    granted_by TEXT NOT NULL,
    granted_at TEXT NOT NULL,
    expires_at TEXT,
    revoked_at TEXT,
    revoked_by TEXT,
    source_request_id TEXT NOT NULL
) STRICT;

CREATE INDEX permission_grants_task ON permission_grants(task_id, granted_at, id);

-- Every side effect an agent asked for, whether or not it was allowed. A blocked
-- agent has a row here in state 'pending' and is not running.
CREATE TABLE permission_requests (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    run_id TEXT REFERENCES agent_runs(id) ON DELETE CASCADE,
    effect_kind TEXT NOT NULL CHECK (effect_kind IN (
        'shell', 'file_write', 'network', 'git_push'
    )),
    classification TEXT NOT NULL CHECK (classification IN (
        'read_only', 'mutating', 'destructive', 'network'
    )),
    capability TEXT NOT NULL,
    scope_json TEXT NOT NULL CHECK (json_valid(scope_json)),
    effect_json TEXT NOT NULL CHECK (json_valid(effect_json)),
    summary TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending', 'decided', 'abandoned')),
    decision TEXT CHECK (decision IN (
        'allow_once', 'allow_task', 'allow_pattern', 'deny'
    )),
    decided_by TEXT,
    decided_at TEXT,
    grant_id TEXT REFERENCES permission_grants(id) ON DELETE SET NULL,
    requested_at TEXT NOT NULL
) STRICT;

CREATE INDEX permission_requests_pending
    ON permission_requests(task_id, state, requested_at, id);

-- The worktree plan a human sees before any Git mutation happens. The
-- fingerprint is over the exact plan that was shown; provisioning revalidates it
-- so an approval cannot be replayed against a plan that changed underneath it.
CREATE TABLE worktree_plans (
    task_id TEXT PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    repository_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    base_ref TEXT NOT NULL,
    base_oid TEXT NOT NULL,
    branch_name TEXT NOT NULL,
    worktree_path TEXT NOT NULL,
    expected_scope_json TEXT NOT NULL CHECK (json_valid(expected_scope_json)),
    setup_commands_json TEXT NOT NULL CHECK (json_valid(setup_commands_json)),
    preflight_json TEXT NOT NULL CHECK (json_valid(preflight_json)),
    disk_json TEXT NOT NULL CHECK (json_valid(disk_json)),
    fingerprint TEXT NOT NULL,
    approved_by TEXT,
    approved_at TEXT,
    approved_fingerprint TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

-- Cleanup is explicit and reversible: the branch tip is archived under
-- refs/mars/archive before the worktree directory goes away.
CREATE TABLE worktree_cleanups (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    removed_path TEXT NOT NULL,
    branch_name TEXT NOT NULL,
    archived_ref TEXT,
    archived_oid TEXT,
    preserved_json TEXT NOT NULL CHECK (json_valid(preserved_json)),
    state TEXT NOT NULL CHECK (state IN ('planned', 'completed', 'restored', 'failed')),
    created_at TEXT NOT NULL,
    completed_at TEXT
) STRICT;

CREATE INDEX worktree_cleanups_task ON worktree_cleanups(task_id, created_at, id);
