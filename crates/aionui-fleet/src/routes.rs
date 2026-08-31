// `ApiError` is the intended error type at this HTTP boundary, so the
// disallowed_types lint steering service code away from `ApiError` does not
// apply here.
#![allow(clippy::disallowed_types)]

//! Munder Fleet control-plane HTTP routes (`/api/fleet/*`).
//!
//! Wire contract mirrors `munder-fleet-a/src/fleet/server.ts` (Strategy-A
//! reference). Handlers only transform request/response; business logic lives
//! in [`FleetService`].

use std::sync::Arc;

use aionui_api_types::{
    ApiResponse, FleetClaimRequest, FleetCompleteRequest, FleetCreateDecisionRequest, FleetCreateProjectRequest,
    FleetCreateTaskRequest, FleetDecisionKind, FleetDecisionsResponse, FleetFailRequest, FleetHiveImportRequest,
    FleetInboxResponse, FleetLogsResponse, FleetPendingDecisionDto, FleetProjectDto, FleetProjectsResponse,
    FleetRegisterRuntimeRequest, FleetResolveDecisionRequest, FleetRuntimeDto, FleetRuntimesResponse, FleetTaskDto,
    FleetTaskStatus, FleetTasksResponse,
};
use aionui_auth::CurrentUser;
use aionui_common::ApiError;
use axum::extract::{Json, Path, Query, State};
use axum::routing::{get, post};
use axum::{Extension, Router};
use serde::Deserialize;

use crate::service::{DecisionInput, DecisionKind, FleetError, FleetService, HiveTaskInput, TaskInput};
use crate::store::{DecisionRow, ExecutionLogRow, InboxRow, ProjectRow, RuntimeRow, TaskRow};

/// Wire mapping: `FleetError` → `ApiError` with stable codes at the HTTP
/// boundary (decision gate conflicts surface as 409).
impl From<FleetError> for ApiError {
    fn from(e: FleetError) -> Self {
        match e {
            FleetError::NotFound(msg) => ApiError::NotFound(msg),
            FleetError::Conflict(msg) => ApiError::Conflict(msg),
            FleetError::BadRequest(msg) => ApiError::BadRequest(msg),
            FleetError::Internal(msg) => ApiError::Internal(msg),
        }
    }
}

/// Shared state for fleet route handlers.
#[derive(Clone)]
pub struct FleetRouterState {
    pub fleet: Arc<FleetService>,
}

/// Build the fleet router (`/api/fleet/*`). Auth applied by the caller.
pub fn fleet_routes(state: FleetRouterState) -> Router {
    Router::new()
        .route("/api/fleet/runtimes", get(list_runtimes))
        .route("/api/fleet/runtimes/register", post(register_runtime))
        .route("/api/fleet/runtimes/{runtime_id}/heartbeat", post(heartbeat))
        .route("/api/fleet/projects", get(list_projects).post(create_project))
        .route("/api/fleet/tasks", get(list_tasks).post(create_task))
        .route("/api/fleet/tasks/claim", post(claim_tasks))
        .route("/api/fleet/tasks/claim-and-work", post(claim_and_work))
        .route("/api/fleet/tasks/{task_id}/start", post(start_task))
        .route("/api/fleet/tasks/{task_id}/complete", post(complete_task))
        .route("/api/fleet/tasks/{task_id}/fail", post(fail_task))
        .route("/api/fleet/decisions", get(list_decisions).post(create_decision))
        .route("/api/fleet/decisions/{decision_id}/resolve", post(resolve_decision))
        .route("/api/fleet/michael/inbox", get(michael_inbox))
        .route("/api/fleet/logs", get(list_logs))
        .route("/api/fleet/import/hive", post(import_hive))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
pub struct ListTasksQuery {
    pub project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListDecisionsQuery {
    pub owner_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListLogsQuery {
    pub task_id: Option<String>,
}

// -- handlers ---------------------------------------------------------------

async fn list_runtimes(
    State(state): State<FleetRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<FleetRuntimesResponse>>, ApiError> {
    let runtimes = state.fleet.list_runtimes().await?;
    Ok(Json(ApiResponse::ok(FleetRuntimesResponse {
        runtimes: runtimes.iter().map(to_runtime_dto).collect(),
    })))
}

async fn register_runtime(
    State(state): State<FleetRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(body): Json<FleetRegisterRuntimeRequest>,
) -> Result<Json<ApiResponse<FleetRuntimesResponse>>, ApiError> {
    let clis_json = serde_json::to_string(&body.clis).unwrap_or_else(|_| "[]".to_string());
    let rt = state
        .fleet
        .register_runtime(
            body.id.as_deref().unwrap_or("runtime:local"),
            body.owner_id.as_deref().unwrap_or(&user.id),
            body.host.as_deref().unwrap_or("local"),
            body.daemon_id.as_deref(),
            body.max_concurrent_tasks,
            &clis_json,
        )
        .await?;
    Ok(Json(ApiResponse::ok(FleetRuntimesResponse {
        runtimes: vec![to_runtime_dto(&rt)],
    })))
}

async fn heartbeat(
    State(state): State<FleetRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(runtime_id): Path<String>,
) -> Result<Json<ApiResponse<FleetRuntimeDto>>, ApiError> {
    let rt = state.fleet.heartbeat(&runtime_id).await?;
    Ok(Json(ApiResponse::ok(to_runtime_dto(&rt))))
}

async fn list_projects(
    State(state): State<FleetRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<FleetProjectsResponse>>, ApiError> {
    let projects = state.fleet.list_projects().await?;
    Ok(Json(ApiResponse::ok(FleetProjectsResponse {
        projects: projects.iter().map(to_project_dto).collect(),
    })))
}

async fn create_project(
    State(state): State<FleetRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Json(body): Json<FleetCreateProjectRequest>,
) -> Result<Json<ApiResponse<FleetProjectDto>>, ApiError> {
    if body.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name required".to_string()));
    }
    let p = state.fleet.create_project_row(body.name.trim()).await?;
    Ok(Json(ApiResponse::ok(to_project_dto(&p))))
}

async fn list_tasks(
    State(state): State<FleetRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Query(q): Query<ListTasksQuery>,
) -> Result<Json<ApiResponse<FleetTasksResponse>>, ApiError> {
    let tasks = state.fleet.list_tasks(q.project_id.as_deref()).await?;
    Ok(Json(ApiResponse::ok(FleetTasksResponse {
        tasks: tasks.into_iter().map(|t| to_task_dto(&t)).collect(),
    })))
}

async fn create_task(
    State(state): State<FleetRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Json(body): Json<FleetCreateTaskRequest>,
) -> Result<Json<ApiResponse<FleetTaskDto>>, ApiError> {
    if body.project_id.trim().is_empty() || body.title.trim().is_empty() || body.prompt.trim().is_empty() {
        return Err(ApiError::BadRequest("projectId, title, prompt required".to_string()));
    }
    let task = state
        .fleet
        .create_task(
            body.project_id.trim(),
            TaskInput {
                title: body.title.trim().to_string(),
                description: body.description,
                assignee: body.assignee,
                prompt: body.prompt,
                priority: 0,
            },
        )
        .await?;
    Ok(Json(ApiResponse::ok(to_task_dto(&task))))
}

async fn claim_tasks(
    State(state): State<FleetRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(body): Json<FleetClaimRequest>,
) -> Result<Json<ApiResponse<FleetTasksResponse>>, ApiError> {
    let tasks = state
        .fleet
        .claim_tasks(
            body.runtime_id.as_deref().unwrap_or("runtime:local"),
            &user.id,
            body.max_tasks.unwrap_or(1),
            body.task_id.as_deref(),
        )
        .await?;
    Ok(Json(ApiResponse::ok(FleetTasksResponse {
        tasks: tasks.into_iter().map(|t| to_task_dto(&t)).collect(),
    })))
}

async fn claim_and_work(
    State(state): State<FleetRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(body): Json<FleetClaimRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, ApiError> {
    let outcome = state
        .fleet
        .claim_and_work(
            body.runtime_id.as_deref().unwrap_or("runtime:local"),
            &user.id,
            body.max_tasks.unwrap_or(1),
            body.task_id.as_deref(),
        )
        .await?;
    let tasks: Vec<FleetTaskDto> = outcome.tasks.iter().map(to_task_dto).collect();
    let inbox: Vec<crate::store::InboxRow> = outcome.inbox_items;
    Ok(Json(ApiResponse::ok(serde_json::json!({
        "tasks": tasks,
        "michaelInbox": inbox.iter().map(to_inbox_dto).collect::<Vec<_>>(),
    }))))
}

async fn start_task(
    State(state): State<FleetRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(task_id): Path<String>,
) -> Result<Json<ApiResponse<FleetTaskDto>>, ApiError> {
    let task = state.fleet.start_task(&task_id, &user.id).await?;
    Ok(Json(ApiResponse::ok(to_task_dto(&task))))
}

async fn complete_task(
    State(state): State<FleetRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(task_id): Path<String>,
    Json(body): Json<FleetCompleteRequest>,
) -> Result<Json<ApiResponse<FleetTaskDto>>, ApiError> {
    let task = state
        .fleet
        .complete_task(
            &task_id,
            body.output.as_deref().unwrap_or(""),
            body.report_to.as_deref(),
        )
        .await?;
    Ok(Json(ApiResponse::ok(to_task_dto(&task))))
}

async fn fail_task(
    State(state): State<FleetRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(task_id): Path<String>,
    Json(body): Json<FleetFailRequest>,
) -> Result<Json<ApiResponse<FleetTaskDto>>, ApiError> {
    let task = state
        .fleet
        .fail_task(&task_id, body.error.as_deref().unwrap_or("failed"))
        .await?;
    Ok(Json(ApiResponse::ok(to_task_dto(&task))))
}

async fn list_decisions(
    State(state): State<FleetRouterState>,
    Extension(user): Extension<CurrentUser>,
    Query(q): Query<ListDecisionsQuery>,
) -> Result<Json<ApiResponse<FleetDecisionsResponse>>, ApiError> {
    let owner = q.owner_id.as_deref().unwrap_or(&user.id);
    let decisions = state.fleet.list_pending_decisions(owner).await?;
    Ok(Json(ApiResponse::ok(FleetDecisionsResponse {
        decisions: decisions.into_iter().map(|d| to_decision_dto(&d)).collect(),
    })))
}

async fn create_decision(
    State(state): State<FleetRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(body): Json<FleetCreateDecisionRequest>,
) -> Result<Json<ApiResponse<FleetPendingDecisionDto>>, ApiError> {
    if body.task_id.trim().is_empty() || body.message.trim().is_empty() {
        return Err(ApiError::BadRequest("taskId, kind, message required".to_string()));
    }
    let kind = match body.kind {
        FleetDecisionKind::Blocker => DecisionKind::Blocker,
        FleetDecisionKind::Review => DecisionKind::Review,
    };
    let d = state
        .fleet
        .create_decision(DecisionInput {
            task_id: body.task_id,
            kind,
            message: body.message,
            owner_id: body.owner_id.unwrap_or(user.id),
        })
        .await?;
    Ok(Json(ApiResponse::ok(to_decision_dto(&d))))
}

async fn resolve_decision(
    State(state): State<FleetRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(decision_id): Path<String>,
    Json(body): Json<FleetResolveDecisionRequest>,
) -> Result<Json<ApiResponse<FleetPendingDecisionDto>>, ApiError> {
    let d = state
        .fleet
        .resolve_decision(
            &decision_id,
            body.resolution.as_deref().unwrap_or("answered"),
            body.note.as_deref(),
        )
        .await?;
    Ok(Json(ApiResponse::ok(to_decision_dto(&d))))
}

async fn michael_inbox(
    State(state): State<FleetRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<FleetInboxResponse>>, ApiError> {
    let items = state.fleet.list_inbox().await?;
    Ok(Json(ApiResponse::ok(FleetInboxResponse {
        items: items.into_iter().map(|i| to_inbox_dto(&i)).collect(),
    })))
}

async fn list_logs(
    State(state): State<FleetRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Query(q): Query<ListLogsQuery>,
) -> Result<Json<ApiResponse<FleetLogsResponse>>, ApiError> {
    let logs = state.fleet.list_logs(q.task_id.as_deref()).await?;
    Ok(Json(ApiResponse::ok(FleetLogsResponse {
        logs: logs.into_iter().map(|l| to_log_dto(&l)).collect(),
    })))
}

async fn import_hive(
    State(state): State<FleetRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Json(body): Json<FleetHiveImportRequest>,
) -> Result<Json<ApiResponse<FleetTasksResponse>>, ApiError> {
    let inputs: Vec<HiveTaskInput> = body
        .tasks
        .into_iter()
        .map(|t| HiveTaskInput {
            id: t.id,
            title: t.title,
            description: t.description,
            assignee: t.assignee,
            status: t.status,
            priority: t.priority,
        })
        .collect();
    let tasks = state.fleet.import_hive_tasks(&body.project_id, &inputs).await?;
    Ok(Json(ApiResponse::ok(FleetTasksResponse {
        tasks: tasks.into_iter().map(|t| to_task_dto(&t)).collect(),
    })))
}

// -- DTO conversion ---------------------------------------------------------

fn to_runtime_dto(rt: &RuntimeRow) -> FleetRuntimeDto {
    let clis: Vec<aionui_api_types::FleetCliInfo> = serde_json::from_str(&rt.clis_json).unwrap_or_default();
    FleetRuntimeDto {
        id: rt.id.clone(),
        host: rt.host.clone(),
        clis,
        owner_id: rt.owner_id.clone(),
        status: rt.status.clone(),
        last_seen_at: rt.last_seen_at.clone(),
        max_concurrent_tasks: rt.max_concurrent_tasks,
        daemon_id: rt.daemon_id.clone(),
    }
}

fn to_project_dto(p: &ProjectRow) -> FleetProjectDto {
    FleetProjectDto {
        id: p.id.clone(),
        name: p.name.clone(),
        created_at: p.created_at.clone(),
    }
}
fn to_task_dto(t: &TaskRow) -> FleetTaskDto {
    let status = match t.status.as_str() {
        "claimed" => FleetTaskStatus::Claimed,
        "doing" => FleetTaskStatus::Doing,
        "blocked" => FleetTaskStatus::Blocked,
        "done" => FleetTaskStatus::Done,
        "failed" => FleetTaskStatus::Failed,
        _ => FleetTaskStatus::Todo,
    };
    FleetTaskDto {
        id: t.id.clone(),
        project_id: t.project_id.clone(),
        title: t.title.clone(),
        description: t.description.clone(),
        assignee: t.assignee.clone(),
        status,
        prompt: t.prompt.clone(),
        priority: t.priority,
        created_at: t.created_at.clone(),
        claimed_by_runtime_id: t.claimed_by_runtime_id.clone(),
        result: t.result.clone(),
        reported_to: t.reported_to.clone(),
    }
}

fn to_decision_dto(d: &DecisionRow) -> FleetPendingDecisionDto {
    FleetPendingDecisionDto {
        id: d.id.clone(),
        task_id: d.task_id.clone(),
        kind: if d.kind == "review" {
            FleetDecisionKind::Review
        } else {
            FleetDecisionKind::Blocker
        },
        message: d.message.clone(),
        owner_id: d.owner_id.clone(),
        status: d.status.clone(),
        created_at: d.created_at.clone(),
        resolution: d.resolution.clone(),
        note: d.note.clone(),
        resolved_at: d.resolved_at.clone(),
    }
}

fn to_log_dto(l: &ExecutionLogRow) -> aionui_api_types::FleetExecutionLogDto {
    aionui_api_types::FleetExecutionLogDto {
        id: l.id.clone(),
        task_id: l.task_id.clone(),
        runtime_id: l.runtime_id.clone(),
        event: l.event.clone(),
        detail: l.detail.clone(),
        tokens_in: l.tokens_in,
        tokens_out: l.tokens_out,
        created_at: l.created_at.clone(),
    }
}

fn to_inbox_dto(i: &InboxRow) -> aionui_api_types::FleetInboxItemDto {
    aionui_api_types::FleetInboxItemDto {
        id: i.id.clone(),
        kind: i.kind.clone(),
        task_id: i.task_id.clone(),
        runtime_id: i.runtime_id.clone(),
        summary: i.summary.clone(),
        created_at: i.created_at.clone(),
        read: i.read != 0,
    }
}
