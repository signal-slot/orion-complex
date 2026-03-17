use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::routing::{get, post};
use serde::Deserialize;

use crate::AppState;
use crate::auth::{AuthUser, create_session_token, validate_oidc_token};
use crate::error::AppError;
use crate::models::User;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/auth/providers", get(list_providers))
        .route("/v1/auth/me", get(me))
        .route("/v1/auth/login", post(login))
}

async fn list_providers(State(state): State<AppState>) -> Json<serde_json::Value> {
    let mut providers = vec!["webauthn"];
    if state.auth_config.google_client_id.is_some() {
        providers.push("google");
    }
    if state.auth_config.microsoft_client_id.is_some() {
        providers.push("microsoft");
    }
    Json(serde_json::json!({ "providers": providers }))
}

async fn me(user: AuthUser) -> Json<User> {
    Json(user.0)
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    provider: String,
    access_token: String,
}

async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Validate the token with the OIDC provider
    let claims = validate_oidc_token(&state.http_client, &req.provider, &req.access_token)
        .await
        .map_err(|e| AppError::BadRequest(format!("authentication failed: {e}")))?;

    // Check email_verified
    if claims.email_verified != Some(true) {
        return Err(AppError::BadRequest("email not verified".into()));
    }

    let email = claims
        .email
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("no email in token".into()))?;

    // Check allowed domains
    let email_domain = email
        .split('@')
        .nth(1)
        .unwrap_or("")
        .to_string();

    if !state.auth_config.allowed_domains.is_empty()
        && !state.auth_config.allowed_domains.contains(&email_domain)
    {
        return Err(AppError::BadRequest(format!(
            "domain '{email_domain}' is not allowed"
        )));
    }

    // Upsert user: find by provider + subject, or create
    let existing = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE provider = ? AND provider_subject = ?",
    )
    .bind(&req.provider)
    .bind(&claims.sub)
    .fetch_optional(&state.db)
    .await?;

    let user = match existing {
        Some(user) => {
            // Update display name if changed
            if user.display_name.as_deref() != claims.name.as_deref() {
                sqlx::query("UPDATE users SET display_name = ? WHERE id = ?")
                    .bind(&claims.name)
                    .bind(&user.id)
                    .execute(&state.db)
                    .await?;
            }
            user
        }
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            let now = crate::unix_now();

            sqlx::query(
                "INSERT INTO users (id, provider, provider_subject, email, email_domain, display_name, role, disabled, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, 'user', 0, ?)",
            )
            .bind(&id)
            .bind(&req.provider)
            .bind(&claims.sub)
            .bind(email)
            .bind(&email_domain)
            .bind(&claims.name)
            .bind(now)
            .execute(&state.db)
            .await?;

            // Generate SSH key for the new user
            let _ = super::ssh_keys::generate_ssh_key_for_user(&state.db, &id).await;

            sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
                .bind(&id)
                .fetch_one(&state.db)
                .await?
        }
    };

    if user.disabled != 0 {
        return Err(AppError::BadRequest("user is disabled".into()));
    }

    // Issue session token
    let token = create_session_token(&state.auth_config, &user.id)
        .map_err(|e| AppError::Internal(format!("failed to create token: {e}")))?;

    Ok(Json(serde_json::json!({
        "token": token,
        "user": user,
    })))
}
