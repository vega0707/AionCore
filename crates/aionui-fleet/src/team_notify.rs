//! Team mailbox notification bridge (Strategy A, task B).
//!
//! Mirrors fleet completion/idle events into AionCore's team mailbox so a
//! team's lead agent (Michael) sees them through the existing team wake path
//! (`idle_notification` / `message` mailbox rows — the same mechanism the
//! AionCore team event loop drains). Not Multica source.

use aionui_common::{generate_id, now_ms};
use sqlx::SqlitePool;

use crate::service::TeamNotifyPort;

/// Writes fleet events into the most recent team's mailbox, addressed to the
/// team lead. Fails softly (returns Err string) when no team exists.
pub struct FleetTeamMailboxNotify {
    pool: SqlitePool,
    /// Fixed sender identity for fleet-originated mailbox rows.
    from_agent_id: String,
}

impl FleetTeamMailboxNotify {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            from_agent_id: "fleet".to_string(),
        }
    }
}

#[async_trait::async_trait]
impl TeamNotifyPort for FleetTeamMailboxNotify {
    async fn notify_team(&self, kind: &str, summary: &str) -> Result<(), String> {
        // Most recent team by `updated_at` (AionCore teams table).
        let team: Option<(String, String, Option<String>)> =
            sqlx::query_as("SELECT id, user_id, lead_agent_id FROM teams ORDER BY updated_at DESC LIMIT 1")
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| format!("list teams: {e}"))?;
        let Some((team_id, user_id, lead_agent_id)) = team else {
            return Err("no teams to notify".to_string());
        };
        let to_agent_id = lead_agent_id.unwrap_or_else(|| team_id.clone());
        let msg_type = if kind == "idle" { "idle_notification" } else { "message" };
        sqlx::query(
            "INSERT INTO mailbox (id, team_id, to_agent_id, from_agent_id, type, content, summary, read, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 0, ?)",
        )
        .bind(generate_id())
        .bind(&team_id)
        .bind(&to_agent_id)
        .bind(&self.from_agent_id)
        .bind(msg_type)
        .bind(summary)
        .bind(summary)
        .bind(now_ms())
        .execute(&self.pool)
        .await
        .map_err(|e| format!("write mailbox: {e}"))?;
        let _ = user_id;
        Ok(())
    }
}
