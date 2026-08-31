//! Fleet service integration tests against an in-memory SQLite pool.
//!
//! Exercises the Strategy-A semantics: register → claim → claim-and-work →
//! inbox, plus the DecisionGate (pending blocks new claims).

use aionui_fleet::{FleetService, TeamNotifyPort};

async fn service() -> FleetService {
    let db = aionui_db::init_database_memory().await.unwrap();
    let svc = FleetService::new(db.pool().clone());
    svc.init().await.unwrap();
    svc
}

#[tokio::test]
async fn register_and_list_runtimes() {
    let svc = service().await;
    let rt = svc
        .register_runtime(
            "runtime:test",
            "user-1",
            "host-a",
            None,
            Some(2),
            r#"[{"provider":"claude"}]"#,
        )
        .await
        .unwrap();
    assert_eq!(rt.id, "runtime:test");
    assert_eq!(rt.status, "online");
    assert_eq!(rt.max_concurrent_tasks, 2);

    let all = svc.list_runtimes().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].host, "host-a");
}

#[tokio::test]
async fn claim_start_complete_reports_to_michael() {
    let svc = service().await;
    let _p = svc.create_project_row("p1").await.unwrap();
    let task = svc
        .create_task(
            "p1",
            aionui_fleet::TaskInput {
                title: "t1".to_string(),
                description: None,
                assignee: None,
                prompt: "do the thing".to_string(),
                priority: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(task.status, "todo");

    let claimed = svc.claim_tasks("runtime:local", "user-1", 1, None).await.unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].status, "claimed");

    let started = svc.start_task(&task.id, "user-1").await.unwrap();
    assert_eq!(started.status, "doing");

    let done = svc.complete_task(&task.id, "result ok", Some("michael")).await.unwrap();
    assert_eq!(done.status, "done");
    assert_eq!(done.result.as_deref(), Some("result ok"));
}

#[tokio::test]
async fn claim_and_work_creates_inbox_items() {
    let svc = service().await;
    let _p = svc.create_project_row("p2").await.unwrap();
    svc.create_task(
        "p2",
        aionui_fleet::TaskInput {
            title: "work".to_string(),
            description: None,
            assignee: None,
            prompt: "execute".to_string(),
            priority: 0,
        },
    )
    .await
    .unwrap();

    let outcome = svc.claim_and_work("runtime:local", "user-1", 1, None).await.unwrap();
    assert_eq!(outcome.tasks.len(), 1);
    assert_eq!(outcome.tasks[0].status, "done");
    let inbox = svc.list_inbox().await.unwrap();
    assert!(!inbox.is_empty());
    assert!(inbox.iter().any(|i| i.kind == "task_completed"));
}

#[tokio::test]
async fn decision_gate_blocks_new_claims() {
    let svc = service().await;
    let _p = svc.create_project_row("p3").await.unwrap();
    let task = svc
        .create_task(
            "p3",
            aionui_fleet::TaskInput {
                title: "gated".to_string(),
                description: None,
                assignee: None,
                prompt: "prompt".to_string(),
                priority: 0,
            },
        )
        .await
        .unwrap();

    svc.create_decision(aionui_fleet::DecisionInput {
        task_id: task.id.clone(),
        kind: aionui_fleet::DecisionKind::Blocker,
        message: "need human".to_string(),
        owner_id: "user-1".to_string(),
    })
    .await
    .unwrap();

    let err = svc.claim_tasks("runtime:local", "user-1", 1, None).await.unwrap_err();
    assert!(
        err.to_string().contains("DecisionGate"),
        "expected gate error, got {err}"
    );

    // Resolve → gate opens
    let decisions = svc.list_pending_decisions("user-1").await.unwrap();
    assert_eq!(decisions.len(), 1);
    svc.resolve_decision(&decisions[0].id, "approved", None).await.unwrap();
    let ok = svc.claim_tasks("runtime:local", "user-1", 1, None).await.unwrap();
    assert_eq!(ok.len(), 1);
}

#[tokio::test]
async fn hive_import_creates_tasks() {
    let svc = service().await;
    let _p = svc.create_project_row("p4").await.unwrap();
    let tasks = svc
        .import_hive_tasks(
            "p4",
            &[aionui_fleet::HiveTaskInput {
                id: Some("hive-1".to_string()),
                title: "imported".to_string(),
                description: None,
                assignee: Some("user-2".to_string()),
                status: Some("todo".to_string()),
                priority: Some(1),
            }],
        )
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, "hive-1");
    assert_eq!(tasks[0].assignee.as_deref(), Some("user-2"));
}

#[tokio::test]
async fn team_notify_writes_idle_notification_to_mailbox() {
    // AionCore teams + mailbox tables are part of migration 001; create a team
    // row and verify the notify port writes an `idle_notification` mailbox row
    // addressed to the team lead.
    let db = aionui_db::init_database_memory().await.unwrap();
    let pool = db.pool().clone();

    sqlx::query(
        "INSERT INTO teams (id, user_id, name, workspace, workspace_mode, agents, lead_agent_id, \
         agents_version, created_at, updated_at) \
         VALUES ('team-1', 'user-1', 't', '/tmp/w', 'workspace', '[]', 'lead-1', 'v1', 1, 2)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let port = aionui_fleet::FleetTeamMailboxNotify::new(pool.clone());
    port.notify_team("idle", "fleet: nothing to do").await.unwrap();

    let rows: Vec<(String, String, String, String)> =
        sqlx::query_as("SELECT team_id, to_agent_id, from_agent_id, type FROM mailbox")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "team-1");
    assert_eq!(rows[0].1, "lead-1");
    assert_eq!(rows[0].2, "fleet");
    assert_eq!(rows[0].3, "idle_notification");
}
