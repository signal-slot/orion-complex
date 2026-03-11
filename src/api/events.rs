use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::routing::get;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::AppError;
use crate::models::EnvironmentEvent;

use super::{PaginationParams, check_env_owner, fetch_env};

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/v1/environments/{env_id}/events",
        get(list_events),
    )
}

async fn list_events(
    State(state): State<AppState>,
    user: AuthUser,
    Path(env_id): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<EnvironmentEvent>>, AppError> {
    let env = fetch_env(&state, &env_id).await?;
    check_env_owner(&user.0, &env)?;

    let events = sqlx::query_as::<_, EnvironmentEvent>(
        "SELECT * FROM environment_events WHERE env_id = ? ORDER BY created_at DESC LIMIT ? OFFSET ?",
    )
    .bind(&env_id)
    .bind(params.limit())
    .bind(params.offset())
    .fetch_all(&state.db)
    .await?;

    Ok(Json(events))
}
