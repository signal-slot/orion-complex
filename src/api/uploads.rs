use axum::Json;
use axum::Router;
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::AppError;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/uploads/iso", get(list_isos))
        .route("/v1/uploads/iso", post(upload_iso))
        // 16 GB limit for ISO uploads
        .layer(DefaultBodyLimit::max(16 * 1024 * 1024 * 1024))
}

async fn list_isos(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let upload_dir = PathBuf::from(&state.data_dir).join("uploads");

    let mut entries = Vec::new();
    let mut seen_names = std::collections::HashSet::new();

    if let Ok(mut dir) = tokio::fs::read_dir(&upload_dir).await {
        while let Ok(Some(entry)) = dir.next_entry().await {
            let path = entry.path();
            let fname = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            // Only show ISO/IMG files
            let lower = fname.to_lowercase();
            if !lower.ends_with(".iso") && !lower.ends_with(".img") {
                continue;
            }

            // Extract display name (strip UUID prefix: "uuid_filename.iso" -> "filename.iso")
            let display_name = if let Some(pos) = fname.find('_') {
                &fname[pos + 1..]
            } else {
                &fname
            };

            // Deduplicate by display name — keep the newest
            if seen_names.contains(display_name) {
                continue;
            }

            let meta = entry.metadata().await.ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);

            seen_names.insert(display_name.to_string());
            entries.push(serde_json::json!({
                "path": path.to_string_lossy(),
                "filename": display_name,
                "size": size,
            }));
        }
    }

    // Sort by filename
    entries.sort_by(|a, b| {
        a["filename"]
            .as_str()
            .unwrap_or("")
            .cmp(b["filename"].as_str().unwrap_or(""))
    });

    Ok(Json(entries))
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

        // Check if same filename already exists — reuse it
        let mut existing_path = None;
        if let Ok(mut dir) = tokio::fs::read_dir(&upload_dir).await {
            while let Ok(Some(entry)) = dir.next_entry().await {
                let entry_name = entry
                    .file_name()
                    .to_string_lossy()
                    .to_string();
                if entry_name.ends_with(&format!("_{safe_name}")) {
                    existing_path = Some(entry.path());
                    break;
                }
            }
        }

        if let Some(existing) = existing_path {
            // Same file already uploaded — skip upload, return existing path
            let path = existing.to_string_lossy().to_string();
            // Drain the field to avoid connection errors
            while field.chunk().await.ok().flatten().is_some() {}
            return Ok((
                StatusCode::OK,
                Json(serde_json::json!({
                    "path": path,
                    "filename": safe_name,
                    "size": 0,
                    "reused": true,
                })),
            ));
        }

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
