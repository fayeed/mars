CREATE TABLE normalized_agent_events (
    session_id TEXT NOT NULL REFERENCES agent_sessions(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    occurred_at TEXT NOT NULL,
    PRIMARY KEY(session_id, sequence)
) STRICT;

CREATE INDEX normalized_agent_events_type
    ON normalized_agent_events(session_id, event_type, sequence);

CREATE TABLE reviews (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    implementation_session_id TEXT NOT NULL REFERENCES agent_sessions(id),
    reviewer_adapter TEXT NOT NULL CHECK (reviewer_adapter IN ('claude', 'codex')),
    status TEXT NOT NULL CHECK (status IN ('candidate', 'accepted', 'dismissed')),
    verdict TEXT NOT NULL CHECK (verdict IN ('pass', 'changes_requested')),
    summary TEXT NOT NULL,
    handoff_digest TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE INDEX reviews_task_time ON reviews(task_id, created_at, id);

CREATE TABLE review_findings (
    id TEXT PRIMARY KEY,
    review_id TEXT NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
    severity TEXT NOT NULL CHECK (severity IN ('info', 'warning', 'error', 'blocker')),
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    relative_path TEXT,
    line INTEGER CHECK (line IS NULL OR line > 0),
    evidence_json TEXT NOT NULL CHECK (json_valid(evidence_json)),
    status TEXT NOT NULL CHECK (status IN ('open', 'sent', 'resolved', 'dismissed')),
    fingerprint TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(review_id, fingerprint)
) STRICT;

CREATE INDEX review_findings_review_status
    ON review_findings(review_id, status, severity, id);
