use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};

use crate::AppState;
use crate::auth::{AdminUser, AuthUser};
use crate::error::AppError;
use crate::models::{Image, RegisterImageRequest};

use super::PaginationParams;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/images", get(list_images))
        .route("/v1/images", post(register_image))
        .route("/v1/images/{image_id}", get(get_image))
        .route(
            "/v1/images/{image_id}",
            axum::routing::delete(delete_image),
        )
}

async fn list_images(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<Image>>, AppError> {
    let images = sqlx::query_as::<_, Image>(
        "SELECT * FROM images ORDER BY created_at DESC LIMIT ? OFFSET ?",
    )
    .bind(params.limit())
    .bind(params.offset())
    .fetch_all(&state.db)
    .await?;
    Ok(Json(images))
}

async fn get_image(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(image_id): Path<String>,
) -> Result<Json<Image>, AppError> {
    let image = sqlx::query_as::<_, Image>("SELECT * FROM images WHERE id = ?")
        .bind(&image_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(image))
}

async fn register_image(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(req): Json<RegisterImageRequest>,
) -> Result<(StatusCode, Json<Image>), AppError> {
    if let Some(ref base_id) = req.base_image_id {
        sqlx::query_as::<_, Image>("SELECT * FROM images WHERE id = ?")
            .bind(base_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| AppError::BadRequest(format!("base image {base_id} not found")))?;
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::unix_now();

    sqlx::query(
        "INSERT INTO images (id, name, provider, guest_os, guest_arch, base_image_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&req.name)
    .bind(&req.provider)
    .bind(&req.guest_os)
    .bind(&req.guest_arch)
    .bind(&req.base_image_id)
    .bind(now)
    .execute(&state.db)
    .await?;

    let image = sqlx::query_as::<_, Image>("SELECT * FROM images WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await?;

    Ok((StatusCode::CREATED, Json(image)))
}

async fn delete_image(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(image_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let env_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM environments WHERE image_id = ?")
            .bind(&image_id)
            .fetch_one(&state.db)
            .await?;

    if env_count.0 > 0 {
        return Err(AppError::BadRequest(format!(
            "image {image_id} is in use by {} environments",
            env_count.0
        )));
    }

    let child_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM images WHERE base_image_id = ?")
            .bind(&image_id)
            .fetch_one(&state.db)
            .await?;

    if child_count.0 > 0 {
        return Err(AppError::BadRequest(format!(
            "image {image_id} has {} child images",
            child_count.0
        )));
    }

    let result = sqlx::query("DELETE FROM images WHERE id = ?")
        .bind(&image_id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("image {image_id} not found")));
    }

    Ok(StatusCode::NO_CONTENT)
}
