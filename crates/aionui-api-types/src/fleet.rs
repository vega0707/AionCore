//! Munder Fleet control-plane DTOs (Strategy A).
//!
//! Multica-semantic rewrite of runtime/claim/decision/inbox — NOT Multica source.
//! Wire shape mirrors `munder-fleet-a/src/types.ts` (the P0–P3 TypeScript
//! reference implementation) so a Munder shell can talk to either backend.

use serde::{Deserialize, Serialize};

/// Runtime: a machine + CLI set that claims fleet tasks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FleetRuntimeDto {
    pub id: String,
    pub host: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clis: Vec<FleetCliInfo>,
    pub owner_id: String,
    pub status: String, // online | offline
    pub last_seen_at: String,
    pub max_concurrent_tasks: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FleetCliInfo {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Project: a task container (Munder hive import target).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FleetProjectDto {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

/// Task status values align with `TaskStatus` in munder-fleet-a.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FleetTaskStatus {
    Todo,
    Claimed,
    Doing,
    Blocked,
    Done,
    Failed,
}

impl FleetTaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Claimed => "claimed",
            Self::Doing => "doing",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

/// Task: a unit of work claimed by a runtime and reported back to Michael.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FleetTaskDto {
    pub id: String,
    pub project_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    pub status: FleetTaskStatus,
    pub prompt: String,
    pub priority: i64,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_by_runtime_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_to: Option<String>,
}

/// Pending decision kind: blocker routes to role owner; review routes to Michael.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FleetDecisionKind {
    Blocker,
    Review,
}

/// Pending decision: a HITL gate that blocks new claims while unresolved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FleetPendingDecisionDto {
    pub id: String,
    pub task_id: String,
    pub kind: FleetDecisionKind,
    pub message: String,
    pub owner_id: String,
    pub status: String, // pending | resolved
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<String>,
}

/// Execution log row (usage accounting).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FleetExecutionLogDto {
    pub id: String,
    pub task_id: String,
    pub runtime_id: String,
    pub event: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_in: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_out: Option<i64>,
    pub created_at: String,
}

/// Michael inbox item: completed/failed task or idle notification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FleetInboxItemDto {
    pub id: String,
    pub kind: String, // task_completed | task_failed | idle
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    pub summary: String,
    pub created_at: String,
    pub read: bool,
}

// ---------------------------------------------------------------------------
// Request DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetRegisterRuntimeRequest {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub daemon_id: Option<String>,
    #[serde(default)]
    pub max_concurrent_tasks: Option<i64>,
    #[serde(default)]
    pub clis: Vec<FleetCliInfo>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetCreateProjectRequest {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetCreateTaskRequest {
    pub project_id: String,
    pub title: String,
    #[serde(default)]
    pub assignee: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetClaimRequest {
    #[serde(default)]
    pub runtime_id: Option<String>,
    #[serde(default)]
    pub max_tasks: Option<i64>,
    #[serde(default)]
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetCompleteRequest {
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub report_to: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetFailRequest {
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetCreateDecisionRequest {
    pub task_id: String,
    pub kind: FleetDecisionKind,
    pub message: String,
    #[serde(default)]
    pub owner_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetResolveDecisionRequest {
    #[serde(default)]
    pub resolution: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetHiveTaskImport {
    #[serde(default)]
    pub id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub priority: Option<i64>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub result: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetHiveImportRequest {
    pub project_id: String,
    pub tasks: Vec<FleetHiveTaskImport>,
}

// ---------------------------------------------------------------------------
// Response DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetRuntimesResponse {
    pub runtimes: Vec<FleetRuntimeDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetProjectsResponse {
    pub projects: Vec<FleetProjectDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetTasksResponse {
    pub tasks: Vec<FleetTaskDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetDecisionsResponse {
    pub decisions: Vec<FleetPendingDecisionDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetInboxResponse {
    pub items: Vec<FleetInboxItemDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetLogsResponse {
    pub logs: Vec<FleetExecutionLogDto>,
}
