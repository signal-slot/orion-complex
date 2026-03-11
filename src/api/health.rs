use axum::Router;
use axum::routing::get;

use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/healthz", get(healthz))
}

async fn healthz() -> &'static str {
    "ok"
}
