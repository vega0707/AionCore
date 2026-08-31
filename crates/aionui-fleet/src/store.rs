//! SQLite access for the Fleet tables (`fleet_runtimes` / `fleet_pending_decisions`
//! / `fleet_execution_logs` — migration `044_fleet_runtime_and_pending_decision.sql`).
//!
//! The overlay migration only defines the three fleet tables from the original
//! overlay. The Strategy-A reference implementation (`munder-fleet-a/src/fleet`)
//! also persists projects/tasks/inbox; this store keeps those in the same
//! migration-family tables prefixed `fleet_` so a single schema owns the whole
//! control plane. All SQL is written here; `FleetService` never opens
//! transactions directly.

use aionui_common::{generate_prefixed_id, now_ms};
use sqlx::SqlitePool;

use crate::service::{DecisionInput, HiveTaskInput, TaskInput};

/// Runtime row (`fleet_runtimes`).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RuntimeRow {
    pub id: String,
    pub host: String,
    pub clis_json: String,
    pub owner_id: String,
    pub status: String,
    pub last_seen_at: String,
    pub max_concurrent_tasks: i64,
    pub daemon_id: Option<String>,
}

/// Project row (`fleet_projects`).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ProjectRow {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

/// Task row (`fleet_tasks`).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TaskRow {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub description: Option<String>,
    pub assignee: Option<String>,
    pub status: String,
    pub prompt: String,
    pub priority: i64,
    pub created_at: String,
    pub claimed_by_runtime_id: Option<String>,
    pub result: Option<String>,
    pub reported_to: Option<String>,
}

/// Pending decision row (`fleet_pending_decisions`).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DecisionRow {
    pub id: String,
    pub task_id: String,
    pub kind: String,
    pub message: String,
    pub owner_id: String,
    pub status: String,
    pub created_at: String,
    pub resolution: Option<String>,
    pub note: Option<String>,
    pub resolved_at: Option<String>,
}

/// Execution log row (`fleet_execution_logs`).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ExecutionLogRow {
    pub id: String,
    pub task_id: String,
    pub runtime_id: String,
    pub event: String,
    pub detail: String,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub created_at: String,
}

/// Michael inbox row (`fleet_inbox`).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct InboxRow {
    pub id: String,
    pub kind: String,
    pub task_id: Option<String>,
    pub runtime_id: Option<String>,
    pub summary: String,
    pub created_at: String,
    pub read: i64,
}

const FLEET_EXTRA_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS fleet_projects (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE IF NOT EXISTS fleet_tasks (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT,
    assignee TEXT,
    status TEXT NOT NULL DEFAULT 'todo',
    prompt TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    claimed_by_runtime_id TEXT,
    result TEXT,
    reported_to TEXT
);
CREATE INDEX IF NOT EXISTS idx_fleet_tasks_status ON fleet_tasks(status);
CREATE TABLE IF NOT EXISTS fleet_inbox (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL,
    task_id TEXT,
    runtime_id TEXT,
    summary TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    read INTEGER NOT NULL DEFAULT 0
);
";

#[derive(Clone)]
pub struct FleetStore {
    pool: SqlitePool,
}

impl FleetStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Ensure the fleet tables exist (migration 044 covers three tables; the
    /// project/task/inbox tables are created idempotently here).
    pub async fn ensure_schema(&self) -> Result<(), sqlx::Error> {
        sqlx::raw_sql(FLEET_EXTRA_SCHEMA).execute(&self.pool).await?;
        Ok(())
    }

    // -- runtimes -----------------------------------------------------------

    pub async fn upsert_runtime(&self, rt: &RuntimeRow) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO fleet_runtimes (\
                id, host, clis_json, owner_id, status, last_seen_at, max_concurrent_tasks, daemon_id\
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)\
            ON CONFLICT(id) DO UPDATE SET \
                host = excluded.host, clis_json = excluded.clis_json, owner_id = excluded.owner_id,\
                status = excluded.status, last_seen_at = excluded.last_seen_at,\
                max_concurrent_tasks = excluded.max_concurrent_tasks, daemon_id = excluded.daemon_id",
        )
        .bind(&rt.id)
        .bind(&rt.host)
        .bind(&rt.clis_json)
        .bind(&rt.owner_id)
        .bind(&rt.status)
        .bind(&rt.last_seen_at)
        .bind(rt.max_concurrent_tasks)
        .bind(&rt.daemon_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_runtimes(&self) -> Result<Vec<RuntimeRow>, sqlx::Error> {
        sqlx::query_as::<_, RuntimeRow>("SELECT * FROM fleet_runtimes ORDER BY last_seen_at DESC")
            .fetch_all(&self.pool)
            .await
    }

    pub async fn get_runtime(&self, id: &str) -> Result<Option<RuntimeRow>, sqlx::Error> {
        sqlx::query_as::<_, RuntimeRow>("SELECT * FROM fleet_runtimes WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn heartbeat_runtime(&self, id: &str, now: &str) -> Result<Option<RuntimeRow>, sqlx::Error> {
        let updated = sqlx::query("UPDATE fleet_runtimes SET status = 'online', last_seen_at = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if updated.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_runtime(id).await
    }

    // -- projects -----------------------------------------------------------

    pub async fn create_project(&self, name: &str) -> Result<ProjectRow, sqlx::Error> {
        let id = generate_prefixed_id("proj");
        sqlx::query("INSERT INTO fleet_projects (id, name) VALUES (?, ?)")
            .bind(&id)
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(ProjectRow {
            id,
            name: name.to_string(),
            created_at: now_str(),
        })
    }

    pub async fn list_projects(&self) -> Result<Vec<ProjectRow>, sqlx::Error> {
        sqlx::query_as::<_, ProjectRow>("SELECT * FROM fleet_projects ORDER BY created_at")
            .fetch_all(&self.pool)
            .await
    }

    // -- tasks --------------------------------------------------------------

    pub async fn create_task(&self, input: &TaskInput, project_id: &str) -> Result<TaskRow, sqlx::Error> {
        let id = generate_prefixed_id("task");
        sqlx::query(
            "INSERT INTO fleet_tasks (\
                id, project_id, title, description, assignee, status, prompt, priority\
            ) VALUES (?, ?, ?, ?, ?, 'todo', ?, ?)",
        )
        .bind(&id)
        .bind(project_id)
        .bind(&input.title)
        .bind(&input.description)
        .bind(&input.assignee)
        .bind(&input.prompt)
        .bind(input.priority)
        .execute(&self.pool)
        .await?;
        self.get_task(&id).await.map(|t| t.unwrap())
    }

    pub async fn get_task(&self, id: &str) -> Result<Option<TaskRow>, sqlx::Error> {
        sqlx::query_as::<_, TaskRow>("SELECT * FROM fleet_tasks WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn list_tasks(&self, project_id: Option<&str>) -> Result<Vec<TaskRow>, sqlx::Error> {
        let rows = match project_id {
            Some(pid) => {
                sqlx::query_as::<_, TaskRow>("SELECT * FROM fleet_tasks WHERE project_id = ? ORDER BY created_at")
                    .bind(pid)
                    .fetch_all(&self.pool)
                    .await?
            }
            None => {
                sqlx::query_as::<_, TaskRow>("SELECT * FROM fleet_tasks ORDER BY created_at")
                    .fetch_all(&self.pool)
                    .await?
            }
        };
        Ok(rows)
    }

    /// Claim up to `max_tasks` todo tasks (optionally exactly `task_id`).
    /// Returns claimed rows (empty when nothing claimable).
    pub async fn claim_tasks(
        &self,
        runtime_id: &str,
        max_tasks: i64,
        task_id: Option<&str>,
    ) -> Result<Vec<TaskRow>, sqlx::Error> {
        let claimed: Vec<TaskRow> = if let Some(tid) = task_id {
            let updated = sqlx::query(
                "UPDATE fleet_tasks SET status = 'claimed', claimed_by_runtime_id = ? \
                 WHERE id = ? AND status = 'todo'",
            )
            .bind(runtime_id)
            .bind(tid)
            .execute(&self.pool)
            .await?;
            if updated.rows_affected() == 0 {
                vec![]
            } else {
                vec![self.get_task(tid).await?.unwrap()]
            }
        } else {
            // SQLite single-statement claim: update the N lowest-created todo
            // tasks in one query, then read them back.
            sqlx::query(
                "UPDATE fleet_tasks SET status = 'claimed', claimed_by_runtime_id = ? \
                 WHERE id IN (SELECT id FROM fleet_tasks WHERE status = 'todo' \
                              ORDER BY created_at LIMIT ?)",
            )
            .bind(runtime_id)
            .bind(max_tasks)
            .execute(&self.pool)
            .await?;
            sqlx::query_as::<_, TaskRow>(
                "SELECT * FROM fleet_tasks WHERE claimed_by_runtime_id = ? AND status = 'claimed' \
                 ORDER BY created_at",
            )
            .bind(runtime_id)
            .fetch_all(&self.pool)
            .await?
        };
        Ok(claimed)
    }

    pub async fn start_task(&self, id: &str) -> Result<Option<TaskRow>, sqlx::Error> {
        let updated = sqlx::query("UPDATE fleet_tasks SET status = 'doing' WHERE id = ? AND status = 'claimed'")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if updated.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_task(id).await
    }

    pub async fn complete_task(
        &self,
        id: &str,
        output: &str,
        report_to: Option<&str>,
        tokens_in: i64,
        tokens_out: i64,
    ) -> Result<Option<TaskRow>, sqlx::Error> {
        let updated = sqlx::query("UPDATE fleet_tasks SET status = 'done', result = ?, reported_to = ? WHERE id = ?")
            .bind(output)
            .bind(report_to)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if updated.rows_affected() == 0 {
            return Ok(None);
        }
        self.log_event(id, "completed", output, Some(tokens_in), Some(tokens_out))
            .await?;
        self.get_task(id).await
    }

    pub async fn fail_task(&self, id: &str, error: &str) -> Result<Option<TaskRow>, sqlx::Error> {
        let updated = sqlx::query("UPDATE fleet_tasks SET status = 'failed', result = ? WHERE id = ?")
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if updated.rows_affected() == 0 {
            return Ok(None);
        }
        self.log_event(id, "failed", error, None, None).await?;
        self.get_task(id).await
    }

    pub async fn import_hive_tasks(
        &self,
        project_id: &str,
        tasks: &[HiveTaskInput],
    ) -> Result<Vec<TaskRow>, sqlx::Error> {
        let mut out = Vec::with_capacity(tasks.len());
        for t in tasks {
            let id = t.id.clone().unwrap_or_else(|| generate_prefixed_id("task"));
            sqlx::query(
                "INSERT OR IGNORE INTO fleet_tasks (\
                    id, project_id, title, description, assignee, status, prompt, priority\
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(project_id)
            .bind(&t.title)
            .bind(&t.description)
            .bind(&t.assignee)
            .bind(t.status.as_deref().unwrap_or("todo"))
            .bind("") // prompt not part of hive import shape; empty prompt tasks stay manual
            .bind(t.priority.unwrap_or(0))
            .execute(&self.pool)
            .await?;
            if let Some(row) = self.get_task(&id).await? {
                out.push(row);
            }
        }
        Ok(out)
    }

    // -- decisions ----------------------------------------------------------

    pub async fn create_decision(&self, input: &DecisionInput) -> Result<DecisionRow, sqlx::Error> {
        let id = generate_prefixed_id("dec");
        sqlx::query(
            "INSERT INTO fleet_pending_decisions (\
                id, task_id, kind, message, owner_id, status\
            ) VALUES (?, ?, ?, ?, ?, 'pending')",
        )
        .bind(&id)
        .bind(&input.task_id)
        .bind(input.kind.as_str())
        .bind(&input.message)
        .bind(&input.owner_id)
        .execute(&self.pool)
        .await?;
        self.get_decision(&id).await.map(|d| d.unwrap())
    }

    pub async fn get_decision(&self, id: &str) -> Result<Option<DecisionRow>, sqlx::Error> {
        sqlx::query_as::<_, DecisionRow>("SELECT * FROM fleet_pending_decisions WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn list_pending_decisions(&self, owner_id: &str) -> Result<Vec<DecisionRow>, sqlx::Error> {
        sqlx::query_as::<_, DecisionRow>(
            "SELECT * FROM fleet_pending_decisions WHERE owner_id = ? AND status = 'pending' ORDER BY created_at",
        )
        .bind(owner_id)
        .fetch_all(&self.pool)
        .await
    }

    /// DecisionGate: does `owner_id` hold any unresolved pending decision?
    pub async fn has_pending_decision(&self, owner_id: &str) -> Result<bool, sqlx::Error> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT COUNT(*) FROM fleet_pending_decisions WHERE owner_id = ? AND status = 'pending'")
                .bind(owner_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(c,)| c > 0).unwrap_or(false))
    }

    pub async fn resolve_decision(
        &self,
        id: &str,
        resolution: &str,
        note: Option<&str>,
    ) -> Result<Option<DecisionRow>, sqlx::Error> {
        let now = now_str();
        let updated = sqlx::query(
            "UPDATE fleet_pending_decisions SET status = 'resolved', resolution = ?, note = ?, resolved_at = ? \
             WHERE id = ? AND status = 'pending'",
        )
        .bind(resolution)
        .bind(note)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_decision(id).await
    }

    // -- logs / inbox -------------------------------------------------------

    pub async fn log_event(
        &self,
        task_id: &str,
        event: &str,
        detail: &str,
        tokens_in: Option<i64>,
        tokens_out: Option<i64>,
    ) -> Result<(), sqlx::Error> {
        let id = generate_prefixed_id("log");
        sqlx::query(
            "INSERT INTO fleet_execution_logs (\
                id, task_id, runtime_id, event, detail, tokens_in, tokens_out\
            ) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(task_id)
        .bind("")
        .bind(event)
        .bind(detail)
        .bind(tokens_in)
        .bind(tokens_out)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_logs(&self, task_id: Option<&str>) -> Result<Vec<ExecutionLogRow>, sqlx::Error> {
        let rows = match task_id {
            Some(tid) => {
                sqlx::query_as::<_, ExecutionLogRow>(
                    "SELECT * FROM fleet_execution_logs WHERE task_id = ? ORDER BY created_at",
                )
                .bind(tid)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, ExecutionLogRow>("SELECT * FROM fleet_execution_logs ORDER BY created_at")
                    .fetch_all(&self.pool)
                    .await?
            }
        };
        Ok(rows)
    }

    pub async fn push_inbox(
        &self,
        kind: &str,
        task_id: Option<&str>,
        runtime_id: Option<&str>,
        summary: &str,
    ) -> Result<(), sqlx::Error> {
        let id = generate_prefixed_id("inb");
        sqlx::query("INSERT INTO fleet_inbox (id, kind, task_id, runtime_id, summary) VALUES (?, ?, ?, ?, ?)")
            .bind(&id)
            .bind(kind)
            .bind(task_id)
            .bind(runtime_id)
            .bind(summary)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_inbox(&self) -> Result<Vec<InboxRow>, sqlx::Error> {
        sqlx::query_as::<_, InboxRow>("SELECT * FROM fleet_inbox ORDER BY created_at DESC LIMIT 50")
            .fetch_all(&self.pool)
            .await
    }
}

pub fn now_str() -> String {
    // datetime('now') in SQLite is UTC "YYYY-MM-DD HH:MM:SS"; the TypeScript
    // reference uses ISO8601. Keep ISO8601 with 'T' for shell parity.
    let ms = now_ms();
    let secs = ms / 1000;
    let sub_ms = ms % 1000;
    format!("{}.{:03}Z", epoch_secs_to_sqlite(secs), sub_ms)
}

fn epoch_secs_to_sqlite(secs: i64) -> String {
    // SQLite strftime('%Y-%m-%dT%H:%M:%fZ', secs, 'unixepoch') would be
    // simplest but we avoid a query here; format via chrono.
    use chrono::{TimeZone, Utc};
    Utc.timestamp_opt(secs, 0)
        .single()
        .map(|t| t.format("%Y-%m-%dT%H:%M:%S").to_string())
        .unwrap_or_else(|| "1970-01-01T00:00:00".to_string())
}
