use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::AppState;
use crate::models::User;

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub jwt_secret: String,
    pub jwt_expiry_secs: u64,
    pub google_client_id: Option<String>,
    pub microsoft_client_id: Option<String>,
    pub allowed_domains: Vec<String>,
}

impl AuthConfig {
    pub fn from_env() -> Self {
        use std::env;

        let jwt_secret =
            env::var("JWT_SECRET").unwrap_or_else(|_| "dev-secret-change-in-production".into());

        let jwt_expiry_secs = env::var("JWT_EXPIRY_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(86400); // 24 hours

        let google_client_id = env::var("GOOGLE_CLIENT_ID").ok();
        let microsoft_client_id = env::var("MICROSOFT_CLIENT_ID").ok();

        let allowed_domains = env::var("ALLOWED_DOMAINS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Self {
            jwt_secret,
            jwt_expiry_secs,
            google_client_id,
            microsoft_client_id,
            allowed_domains,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionClaims {
    pub sub: String, // user id
    pub exp: u64,
    pub iat: u64,
}

pub fn create_session_token(
    auth_config: &AuthConfig,
    user_id: &str,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let claims = SessionClaims {
        sub: user_id.to_string(),
        iat: now,
        exp: now + auth_config.jwt_expiry_secs,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(auth_config.jwt_secret.as_bytes()),
    )
}

pub fn validate_session_token(
    auth_config: &AuthConfig,
    token: &str,
) -> Result<SessionClaims, jsonwebtoken::errors::Error> {
    let token_data = decode::<SessionClaims>(
        token,
        &DecodingKey::from_secret(auth_config.jwt_secret.as_bytes()),
        &Validation::default(),
    )?;
    Ok(token_data.claims)
}

/// OIDC ID token claims (common fields across Google/Microsoft)
#[derive(Debug, Deserialize)]
pub struct OidcClaims {
    pub sub: String,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub name: Option<String>,
}

/// Validate an OIDC ID token by calling the provider's userinfo endpoint.
/// This is simpler than full JWT/JWKS validation and works for both providers.
pub async fn validate_oidc_token(
    http_client: &reqwest::Client,
    provider: &str,
    id_token: &str,
) -> Result<OidcClaims, String> {
    let userinfo_url = match provider {
        "google" => "https://www.googleapis.com/oauth2/v3/userinfo",
        "microsoft" => "https://graph.microsoft.com/oidc/userinfo",
        _ => return Err(format!("unknown provider: {provider}")),
    };

    let resp = http_client
        .get(userinfo_url)
        .bearer_auth(id_token)
        .send()
        .await
        .map_err(|e| format!("failed to contact {provider}: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("{provider} returned {status}: {body}"));
    }

    let claims: OidcClaims = resp
        .json()
        .await
        .map_err(|e| format!("failed to parse {provider} response: {e}"))?;

    Ok(claims)
}

// --- Axum extractor for authenticated users ---

#[derive(Debug)]
pub enum AuthError {
    MissingToken,
    InvalidToken,
    UserNotFound,
    UserDisabled,
    Forbidden,
    DbError(sqlx::Error),
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            AuthError::MissingToken => (StatusCode::UNAUTHORIZED, "missing authorization header"),
            AuthError::InvalidToken => (StatusCode::UNAUTHORIZED, "invalid or expired token"),
            AuthError::UserNotFound => (StatusCode::UNAUTHORIZED, "user not found"),
            AuthError::UserDisabled => (StatusCode::FORBIDDEN, "user is disabled"),
            AuthError::Forbidden => (StatusCode::FORBIDDEN, "admin access required"),
            AuthError::DbError(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal server error"),
        };
        let body = serde_json::json!({ "error": msg });
        (status, axum::Json(body)).into_response()
    }
}

/// Extractor that resolves the authenticated user from a Bearer token.
/// Use in handler signatures: `async fn handler(user: AuthUser, ...) -> ...`
pub struct AuthUser(pub User);

async fn extract_user(parts: &mut Parts) -> Result<User, AuthError> {
    let app_state = parts
        .extensions
        .get::<AppState>()
        .ok_or(AuthError::InvalidToken)?;

    let auth_header = parts
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(AuthError::MissingToken)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(AuthError::MissingToken)?;

    let claims = validate_session_token(&app_state.auth_config, token)
        .map_err(|_| AuthError::InvalidToken)?;

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
        .bind(&claims.sub)
        .fetch_optional(&app_state.db)
        .await
        .map_err(AuthError::DbError)?
        .ok_or(AuthError::UserNotFound)?;

    if user.disabled != 0 {
        return Err(AuthError::UserDisabled);
    }

    Ok(user)
}

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(AuthUser(extract_user(parts).await?))
    }
}

/// Extractor that requires the authenticated user to have `role = "admin"`.
pub struct AdminUser(pub User);

impl<S> FromRequestParts<S> for AdminUser
where
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let user = extract_user(parts).await?;
        if user.role.as_deref() != Some("admin") {
            return Err(AuthError::Forbidden);
        }
        Ok(AdminUser(user))
    }
}
