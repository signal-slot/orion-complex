pub mod api;
pub mod auth;
pub mod background;
pub mod config;
pub mod db;
pub mod error;
pub mod events;
pub mod models;
pub mod tasks;
pub mod vm;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::SqlitePool;

use auth::AuthConfig;
use vm::VmProvider;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub auth_config: AuthConfig,
    pub http_client: reqwest::Client,
    pub vm_provider: Arc<dyn VmProvider>,
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
