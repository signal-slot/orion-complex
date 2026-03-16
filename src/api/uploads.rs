use axum::Json;
use axum::Router;
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::StatusCode;
use axum::routing::post;
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::AppError;

pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/uploads/iso", post(upload_iso))
        // 16 GB limit for ISO uploads
        .layer(DefaultBodyLimit::max(16 * 1024 * 1024 * 1024))
}

async fn upload_iso(
    State(state): State<AppState>,
    _user: AuthUser,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let upload_dir = PathBuf::from(&state.data_dir).join("uploads");
    tokio::fs::create_dir_all(&upload_dir)
        .await
        .map_err(|e| AppError::Internal(format!("failed to create upload dir: {e}")))?;

    let id = uuid::Uuid::new_v4().to_string();

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
    {
        let filename = field
            .file_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{id}.iso"));

        // Sanitize filename
        let safe_name = filename
            .rsplit('/')
            .next()
            .unwrap_or(&filename)
            .rsplit('\\')
            .next()
            .unwrap_or(&filename)
            .to_string();

        let dest = upload_dir.join(format!("{id}_{safe_name}"));

        // Stream chunks to disk instead of loading entire file into memory
        let mut file = tokio::fs::File::create(&dest)
            .await
            .map_err(|e| AppError::Internal(format!("failed to create file: {e}")))?;

        let mut total_bytes: u64 = 0;
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| AppError::BadRequest(format!("failed to read chunk: {e}")))?
        {
            total_bytes += chunk.len() as u64;
            file.write_all(&chunk)
                .await
                .map_err(|e| AppError::Internal(format!("failed to write chunk: {e}")))?;
        }
        file.flush()
            .await
            .map_err(|e| AppError::Internal(format!("failed to flush file: {e}")))?;

        let path = dest.to_string_lossy().to_string();

        return Ok((
            StatusCode::CREATED,
            Json(serde_json::json!({
                "path": path,
                "filename": safe_name,
                "size": total_bytes,
            })),
        ));
    }

    Err(AppError::BadRequest("no file uploaded".into()))
}
