//! Fleet business logic — sole location for fleet orchestration (no axum).
//!
//! Semantics aligned with `munder-fleet-a/src/fleet` (Strategy-A TypeScript
//! reference): runtime registration, task claim, DecisionGate (pending blocks
//! new claims), completion → Michael inbox, execution logs.

use std::sync::Arc;

use sqlx::SqlitePool;

use crate::store::{DecisionRow, FleetStore, InboxRow, RuntimeRow, TaskRow};

/// Crate-owned fleet error; mapped to `FleetError` at the route boundary.
#[derive(Debug, thiserror::Error)]
pub enum FleetError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("internal: {0}")]
    Internal(String),
}

impl From<sqlx::Error> for FleetError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(format!("db: {e}"))
    }
}

/// Input for task creation.
#[derive(Debug, Clone)]
pub struct TaskInput {
    pub title: String,
    pub description: Option<String>,
    pub assignee: Option<String>,
    pub prompt: String,
    pub priority: i64,
}

/// Input for decision creation.
#[derive(Debug, Clone)]
pub struct DecisionInput {
    pub task_id: String,
    pub kind: DecisionKind,
    pub message: String,
    pub owner_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionKind {
    Blocker,
    Review,
}

impl DecisionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Blocker => "blocker",
            Self::Review => "review",
        }
    }
}

/// Input for hive task import.
#[derive(Debug, Clone)]
pub struct HiveTaskInput {
    pub id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub assignee: Option<String>,
    pub status: Option<String>,
    pub priority: Option<i64>,
}

/// Result of a claim-and-work round.
#[derive(Debug, Clone)]
pub struct WorkOutcome {
    pub tasks: Vec<TaskRow>,
    pub inbox_items: Vec<InboxRow>,
}

/// Team mailbox notification port (optional). When injected, the fleet service
/// mirrors completion/idle events into AionCore's team mailbox so Michael/Lead
/// agents see them through the existing team wake path.
#[async_trait::async_trait]
pub trait TeamNotifyPort: Send + Sync {
    /// Notify a team's Michael/Lead mailbox about a fleet event.
    /// `kind` is `task_completed` / `task_failed` / `idle`.
    async fn notify_team(&self, kind: &str, summary: &str) -> Result<(), String>;
}

#[derive(Clone)]
pub struct FleetService {
    store: Arc<FleetStore>,
    team_notify: Option<Arc<dyn TeamNotifyPort>>,
}

impl FleetService {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            store: Arc::new(FleetStore::new(pool)),
            team_notify: None,
        }
    }

    pub fn with_team_notify(mut self, port: Arc<dyn TeamNotifyPort>) -> Self {
        self.team_notify = Some(port);
        self
    }

    pub async fn init(&self) -> Result<(), FleetError> {
        self.store
            .ensure_schema()
            .await
            .map_err(|e| FleetError::Internal(format!("fleet schema init: {e}")))?;
        Ok(())
    }

    // -- runtimes -----------------------------------------------------------

    pub async fn register_runtime(
        &self,
        id: &str,
        owner_id: &str,
        host: &str,
        daemon_id: Option<&str>,
        max_concurrent_tasks: Option<i64>,
        clis_json: &str,
    ) -> Result<RuntimeRow, FleetError> {
        let now = crate::store::now_str();
        let max = max_concurrent_tasks.unwrap_or(2);
        let rt = RuntimeRow {
            id: id.to_string(),
            host: host.to_string(),
            clis_json: clis_json.to_string(),
            owner_id: owner_id.to_string(),
            status: "online".to_string(),
            last_seen_at: now,
            max_concurrent_tasks: max,
            daemon_id: daemon_id.map(str::to_string),
        };
        self.store
            .upsert_runtime(&rt)
            .await
            .map_err(|e| FleetError::Internal(format!("register runtime: {e}")))?;
        Ok(rt)
    }

    pub async fn list_runtimes(&self) -> Result<Vec<RuntimeRow>, FleetError> {
        self.store
            .list_runtimes()
            .await
            .map_err(|e| FleetError::Internal(format!("list runtimes: {e}")))
    }

    pub async fn heartbeat(&self, runtime_id: &str) -> Result<RuntimeRow, FleetError> {
        let now = crate::store::now_str();
        self.store
            .heartbeat_runtime(runtime_id, &now)
            .await
            .map_err(|e| FleetError::Internal(format!("heartbeat: {e}")))?
            .ok_or_else(|| FleetError::NotFound(format!("runtime {runtime_id}")))
    }

    // -- projects -----------------------------------------------------------

    pub async fn create_project_row(&self, name: &str) -> Result<crate::store::ProjectRow, FleetError> {
        self.store
            .create_project(name)
            .await
            .map_err(|e| FleetError::Internal(format!("create project: {e}")))
    }

    pub async fn list_projects(&self) -> Result<Vec<crate::store::ProjectRow>, FleetError> {
        self.store
            .list_projects()
            .await
            .map_err(|e| FleetError::Internal(format!("list projects: {e}")))
    }

    // -- tasks --------------------------------------------------------------

    pub async fn create_task(&self, project_id: &str, input: TaskInput) -> Result<TaskRow, FleetError> {
        self.store
            .create_task(&input, project_id)
            .await
            .map_err(|e| FleetError::Internal(format!("create task: {e}")))
    }

    pub async fn list_tasks(&self, project_id: Option<&str>) -> Result<Vec<TaskRow>, FleetError> {
        self.store
            .list_tasks(project_id)
            .await
            .map_err(|e| FleetError::Internal(format!("list tasks: {e}")))
    }

    /// Claim up to `max_tasks` tasks for `runtime_id`. DecisionGate: if the
    /// owner holds any pending decision, claims are rejected (409).
    pub async fn claim_tasks(
        &self,
        runtime_id: &str,
        owner_id: &str,
        max_tasks: i64,
        task_id: Option<&str>,
    ) -> Result<Vec<TaskRow>, FleetError> {
        self.check_decision_gate(owner_id).await?;
        let claimed = self
            .store
            .claim_tasks(runtime_id, max_tasks, task_id)
            .await
            .map_err(|e| FleetError::Internal(format!("claim tasks: {e}")))?;
        for t in &claimed {
            self.store
                .log_event(&t.id, "claimed", "claimed by runtime", None, None)
                .await
                .ok();
        }
        Ok(claimed)
    }

    pub async fn start_task(&self, id: &str, owner_id: &str) -> Result<TaskRow, FleetError> {
        self.check_decision_gate(owner_id).await?;
        self.store
            .start_task(id)
            .await
            .map_err(|e| FleetError::Internal(format!("start task: {e}")))?
            .ok_or_else(|| FleetError::Conflict(format!("task {id} not in claimed state")))
    }

    /// Claim → start → complete (stub subprocess semantics: a real worker
    /// invocation is out of scope — see ROADMAP "真实 Agent CLI"). Reports to
    /// Michael inbox on completion/failure.
    pub async fn claim_and_work(
        &self,
        runtime_id: &str,
        owner_id: &str,
        max_tasks: i64,
        task_id: Option<&str>,
    ) -> Result<WorkOutcome, FleetError> {
        self.check_decision_gate(owner_id).await?;
        let claimed = self
            .store
            .claim_tasks(runtime_id, max_tasks, task_id)
            .await
            .map_err(|e| FleetError::Internal(format!("claim-and-work claim: {e}")))?;
        let mut done_tasks = Vec::new();
        for t in &claimed {
            self.store.start_task(&t.id).await.ok();
            let (tokens_in, tokens_out) = stub_tokens(&t.prompt);
            let task = self
                .store
                .complete_task(
                    &t.id,
                    "munder-worker: done (stub)",
                    Some("michael"),
                    tokens_in,
                    tokens_out,
                )
                .await
                .ok()
                .flatten();
            if let Some(done) = task {
                done_tasks.push(done);
            }
        }
        for t in &done_tasks {
            self.store
                .push_inbox(
                    "task_completed",
                    Some(&t.id),
                    Some(runtime_id),
                    &format!("task {} completed: {}", t.id, t.title),
                )
                .await
                .ok();
            if let Some(port) = &self.team_notify {
                let summary = format!("fleet task {} completed: {}", t.id, t.title);
                port.notify_team("task_completed", &summary).await.ok();
            }
        }
        if done_tasks.is_empty() {
            // idle notification so Michael can see a no-op round
            self.store
                .push_inbox("idle", None, Some(runtime_id), "claim-and-work: no claimable tasks")
                .await
                .ok();
            if let Some(port) = &self.team_notify {
                port.notify_team("idle", "fleet claim-and-work: no claimable tasks")
                    .await
                    .ok();
            }
        }
        let inbox_items = self.store.list_inbox().await.unwrap_or_default();
        Ok(WorkOutcome {
            tasks: done_tasks,
            inbox_items,
        })
    }

    pub async fn complete_task(&self, id: &str, output: &str, report_to: Option<&str>) -> Result<TaskRow, FleetError> {
        self.store
            .complete_task(id, output, report_to, 0, 0)
            .await
            .map_err(|e| FleetError::Internal(format!("complete task: {e}")))?
            .ok_or_else(|| FleetError::NotFound(format!("task {id}")))
    }

    pub async fn fail_task(&self, id: &str, error: &str) -> Result<TaskRow, FleetError> {
        self.store
            .fail_task(id, error)
            .await
            .map_err(|e| FleetError::Internal(format!("fail task: {e}")))?
            .ok_or_else(|| FleetError::NotFound(format!("task {id}")))
    }

    pub async fn import_hive_tasks(
        &self,
        project_id: &str,
        tasks: &[HiveTaskInput],
    ) -> Result<Vec<TaskRow>, FleetError> {
        self.store
            .import_hive_tasks(project_id, tasks)
            .await
            .map_err(|e| FleetError::Internal(format!("import hive: {e}")))
    }

    // -- decisions ----------------------------------------------------------

    pub async fn create_decision(&self, input: DecisionInput) -> Result<DecisionRow, FleetError> {
        self.store
            .create_decision(&input)
            .await
            .map_err(|e| FleetError::Internal(format!("create decision: {e}")))
    }

    pub async fn list_pending_decisions(&self, owner_id: &str) -> Result<Vec<DecisionRow>, FleetError> {
        self.store
            .list_pending_decisions(owner_id)
            .await
            .map_err(|e| FleetError::Internal(format!("list decisions: {e}")))
    }

    pub async fn resolve_decision(
        &self,
        id: &str,
        resolution: &str,
        note: Option<&str>,
    ) -> Result<DecisionRow, FleetError> {
        self.store
            .resolve_decision(id, resolution, note)
            .await
            .map_err(|e| FleetError::Internal(format!("resolve decision: {e}")))?
            .ok_or_else(|| FleetError::Conflict(format!("decision {id} not pending")))
    }

    // -- logs / inbox -------------------------------------------------------

    pub async fn list_logs(&self, task_id: Option<&str>) -> Result<Vec<crate::store::ExecutionLogRow>, FleetError> {
        self.store
            .list_logs(task_id)
            .await
            .map_err(|e| FleetError::Internal(format!("list logs: {e}")))
    }

    pub async fn list_inbox(&self) -> Result<Vec<InboxRow>, FleetError> {
        self.store
            .list_inbox()
            .await
            .map_err(|e| FleetError::Internal(format!("list inbox: {e}")))
    }

    // -- DecisionGate -------------------------------------------------------

    /// Owner with an unresolved pending decision cannot claim/start new work.
    async fn check_decision_gate(&self, owner_id: &str) -> Result<(), FleetError> {
        let pending = self
            .store
            .has_pending_decision(owner_id)
            .await
            .map_err(|e| FleetError::Internal(format!("decision gate: {e}")))?;
        if pending {
            return Err(FleetError::Conflict(
                "DecisionGate: owner has pending decisions; resolve them before claiming new work".to_string(),
            ));
        }
        Ok(())
    }
}

/// Stub token accounting (4 chars ≈ 1 token, same as reference worker.ts).
fn stub_tokens(prompt: &str) -> (i64, i64) {
    let tin = (prompt.chars().count() / 4) as i64;
    let tout = 8;
    (tin, tout)
}
