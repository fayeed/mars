-- Human-approved, traceable memory.
--
-- Distinct from `knowledge_records`, which is milestone D's projection of one
-- completed agent session onto reviewable claims about that task. These tables
-- hold durable guidance a person has explicitly accepted, anchored to the exact
-- conversation turn that motivated it and carrying a usage trail.
--
-- The separation that matters is enforced by the table split: a proposal is a
-- row in `memory_proposals` and can never become behaviour, while a row in
-- `memories` exists only because a human approved it.

CREATE TABLE memories (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    repository_id TEXT REFERENCES repositories(id) ON DELETE CASCADE,
    task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    scope TEXT NOT NULL CHECK (scope IN ('repo', 'task', 'global')),
    kind TEXT NOT NULL CHECK (kind IN (
        'convention', 'decision', 'preference', 'fact', 'caution'
    )),
    text TEXT NOT NULL,
    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),

    -- Origin: the exact conversation turn this came from. Ids are opaque here
    -- because Mars has no conversation store yet; `origin_verified` records
    -- whether the quoted text was actually checked against that turn.
    origin_conversation_id TEXT NOT NULL,
    origin_message_id TEXT NOT NULL,
    origin_quoted_text TEXT NOT NULL,
    origin_verified INTEGER NOT NULL DEFAULT 0 CHECK (origin_verified IN (0, 1)),

    proposal_id TEXT NOT NULL,
    approved_by TEXT NOT NULL,
    approved_at TEXT NOT NULL,
    supersedes_id TEXT REFERENCES memories(id),
    status TEXT NOT NULL CHECK (status IN ('active', 'superseded', 'revoked')),
    revoked_by TEXT,
    revoked_at TEXT,
    revocation_reason TEXT,

    embedding BLOB NOT NULL,
    embedding_model TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    -- A scope is a promise about where a memory applies, so the row must carry
    -- what that scope needs to be resolvable.
    CHECK (scope <> 'task' OR task_id IS NOT NULL),
    CHECK (scope <> 'repo' OR repository_id IS NOT NULL),
    -- No hard deletes: a revoked record keeps its audit trail and must say who
    -- revoked it and when.
    CHECK (status <> 'revoked' OR (revoked_by IS NOT NULL AND revoked_at IS NOT NULL))
) STRICT;

CREATE INDEX memories_retrieval
    ON memories(project_id, status, scope, kind);
CREATE INDEX memories_repository ON memories(repository_id, status);
CREATE INDEX memories_task ON memories(task_id, status);
CREATE INDEX memories_supersedes ON memories(supersedes_id);
-- One memory may supersede at most one other, matching the single
-- `supersedes` link in the record contract.
CREATE UNIQUE INDEX memories_supersedes_once
    ON memories(supersedes_id) WHERE supersedes_id IS NOT NULL;

CREATE TABLE memory_proposals (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    repository_id TEXT REFERENCES repositories(id) ON DELETE CASCADE,
    task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    scope TEXT NOT NULL CHECK (scope IN ('repo', 'task', 'global')),
    kind TEXT NOT NULL CHECK (kind IN (
        'convention', 'decision', 'preference', 'fact', 'caution'
    )),
    text TEXT NOT NULL,
    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),

    origin_conversation_id TEXT NOT NULL,
    origin_message_id TEXT NOT NULL,
    origin_quoted_text TEXT NOT NULL,
    origin_verified INTEGER NOT NULL DEFAULT 0 CHECK (origin_verified IN (0, 1)),

    proposed_by TEXT NOT NULL,
    proposed_at TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'rejected')),
    resolved_by TEXT,
    resolved_at TEXT,
    rejection_reason TEXT,
    -- Set when approval produced a memory. Not a foreign key declaration
    -- problem: `memories` is created above.
    memory_id TEXT REFERENCES memories(id) ON DELETE SET NULL,

    CHECK (scope <> 'task' OR task_id IS NOT NULL),
    CHECK (scope <> 'repo' OR repository_id IS NOT NULL),
    CHECK (status = 'pending' OR (resolved_by IS NOT NULL AND resolved_at IS NOT NULL)),
    -- The same claim from the same turn is one proposal, however many times an
    -- agent restates it.
    UNIQUE(project_id, origin_conversation_id, origin_message_id, text)
) STRICT;

CREATE INDEX memory_proposals_queue
    ON memory_proposals(project_id, status, proposed_at, id);
CREATE INDEX memory_proposals_task ON memory_proposals(task_id, status);

-- Every injection into an agent's context, so the UI can say which tasks a
-- memory has actually shaped.
CREATE TABLE memory_uses (
    id TEXT PRIMARY KEY,
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    session_id TEXT REFERENCES agent_sessions(id) ON DELETE SET NULL,
    surface TEXT NOT NULL,
    relevance REAL NOT NULL CHECK (relevance >= 0.0 AND relevance <= 1.0),
    used_at TEXT NOT NULL
) STRICT;

CREATE INDEX memory_uses_memory ON memory_uses(memory_id, used_at, id);
CREATE INDEX memory_uses_task ON memory_uses(task_id, memory_id);

-- The audit spine. Every lifecycle transition is an entry, and an approval
-- carries the conversation turn it was spoken in so a transcript view can
-- render it inline as the message it is.
CREATE TABLE memory_events (
    id TEXT PRIMARY KEY,
    proposal_id TEXT REFERENCES memory_proposals(id) ON DELETE CASCADE,
    memory_id TEXT REFERENCES memories(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN (
        'proposed', 'approved', 'edited_and_approved', 'rejected',
        'revised', 'superseded', 'revoked', 'conflict_resolved'
    )),
    actor TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    detail TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    CHECK (proposal_id IS NOT NULL OR memory_id IS NOT NULL)
) STRICT;

CREATE INDEX memory_events_memory ON memory_events(memory_id, occurred_at, id);
CREATE INDEX memory_events_proposal ON memory_events(proposal_id, occurred_at, id);
CREATE INDEX memory_events_conversation
    ON memory_events(conversation_id, message_id);

-- Conflicts a human was shown at approval time, and what they chose. Kept
-- because the choice is part of why the surviving memory survived.
CREATE TABLE memory_conflicts (
    id TEXT PRIMARY KEY,
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    other_memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    similarity REAL NOT NULL,
    signal TEXT NOT NULL CHECK (signal IN ('overlap', 'opposing_polarity')),
    resolution TEXT NOT NULL CHECK (resolution IN (
        'kept_both', 'superseded_other', 'revoked_other'
    )),
    resolved_by TEXT NOT NULL,
    resolved_at TEXT NOT NULL,
    UNIQUE(memory_id, other_memory_id)
) STRICT;

CREATE INDEX memory_conflicts_other ON memory_conflicts(other_memory_id);
