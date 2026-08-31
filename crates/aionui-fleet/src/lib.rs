//! Munder Fleet control plane on AionCore (Strategy A).
//!
//! Multica-semantic runtime/claim/decision/inbox implemented as a Rust crate
//! inside the AionCore fork. Wire shape mirrors `munder-fleet-a/src/fleet`
//! (TypeScript reference semantics); NOT Multica source.

pub mod routes;
mod service;
mod store;
mod team_notify;

pub use routes::{FleetRouterState, fleet_routes};
pub use service::{DecisionInput, DecisionKind, FleetService, HiveTaskInput, TaskInput, TeamNotifyPort};
pub use store::FleetStore;
pub use team_notify::FleetTeamMailboxNotify;
