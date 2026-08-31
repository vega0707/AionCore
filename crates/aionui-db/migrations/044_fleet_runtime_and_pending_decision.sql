-- Munder Fleet overlay (Strategy A)
-- Fork point: see docs/munder-fleet/FORK_POINT.md / munder-fleet-a docs/VERSIONS.md
-- Semantically aligned with Multica runtime/claim; not Multica source.

CREATE TABLE IF NOT EXISTS fleet_runtimes (
    id TEXT PRIMARY KEY NOT NULL,
    host TEXT NOT NULL,
    clis_json TEXT NOT NULL DEFAULT '[]',
    owner_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'online'
        CHECK (status IN ('online', 'offline')),
    last_seen_at TEXT NOT NULL,
    max_concurrent_tasks INTEGER NOT NULL DEFAULT 2,
    daemon_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS fleet_pending_decisions (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('blocker', 'review')),
    message TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'resolved')),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    resolution TEXT,
    note TEXT,
    resolved_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_fleet_pending_owner_status
    ON fleet_pending_decisions(owner_id, status);

CREATE TABLE IF NOT EXISTS fleet_execution_logs (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL,
    runtime_id TEXT NOT NULL,
    event TEXT NOT NULL,
    detail TEXT NOT NULL,
    tokens_in INTEGER,
    tokens_out INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
