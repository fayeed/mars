CREATE TABLE forge_mutations (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    remote_id TEXT NOT NULL REFERENCES remotes(id) ON DELETE CASCADE,
    operation TEXT NOT NULL CHECK (operation IN ('create_pull_request')),
    status TEXT NOT NULL CHECK (status IN ('started', 'completed', 'failed', 'interrupted')),
    plan_fingerprint TEXT NOT NULL,
    push_approved INTEGER NOT NULL CHECK (push_approved IN (0, 1)),
    forge_approved INTEGER NOT NULL CHECK (forge_approved IN (0, 1)),
    details_json TEXT NOT NULL CHECK (json_valid(details_json)),
    error TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT
) STRICT;

CREATE INDEX forge_mutations_task_time
    ON forge_mutations(task_id, started_at, id);

CREATE INDEX forge_mutations_status
    ON forge_mutations(status, started_at, id);
