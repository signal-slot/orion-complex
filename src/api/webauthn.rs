use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::routing::post;
use serde::Deserialize;
use webauthn_rs::prelude::*;

use crate::AppState;
use crate::auth::create_session_token;
use crate::error::AppError;
use crate::models::User;

/// Verify a TOTP code against a secret, checking ±3 time steps (±90 seconds).
/// Uses manual generate+compare because totp_rs::check_current() is unreliable.
fn verify_totp_code(totp: &totp_rs::TOTP, code: &str) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    (-3i64..=3).any(|offset| {
        let t = (now as i64 + offset * 30) as u64;
        totp.generate(t) == code
    })
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/auth/webauthn/register/begin", post(register_begin))
        .route(
            "/v1/auth/webauthn/register/complete",
            post(register_complete),
        )
        .route("/v1/auth/webauthn/login/begin", post(login_begin))
        .route("/v1/auth/webauthn/login/complete", post(login_complete))
        .route("/v1/auth/totp/register", post(totp_register))
        .route("/v1/auth/totp/verify", post(totp_verify))
        .route("/v1/auth/totp/login", post(totp_login))
}

// ── Registration ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RegisterBeginRequest {
    email: String,
    display_name: Option<String>,
}

async fn register_begin(
    State(state): State<AppState>,
    Json(req): Json<RegisterBeginRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let email = req.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return Err(AppError::BadRequest("invalid email".into()));
    }

    // Check allowed domains
    let email_domain = email.split('@').nth(1).unwrap_or("").to_string();
    if !state.auth_config.allowed_domains.is_empty()
        && !state.auth_config.allowed_domains.contains(&email_domain)
    {
        return Err(AppError::BadRequest(format!(
            "domain '{email_domain}' is not allowed"
        )));
    }

    // Check if user already exists
    let existing_user =
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE email = ?")
            .bind(&email)
            .fetch_optional(&state.db)
            .await?;

    // Load existing credentials to exclude (prevent duplicate registrations)
    let exclude_creds: Vec<CredentialID> = if let Some(ref user) = existing_user {
        load_passkeys(&state.db, &user.id)
            .await?
            .into_iter()
            .map(|pk| pk.cred_id().clone())
            .collect()
    } else {
        vec![]
    };

    let user_uuid = if let Some(ref user) = existing_user {
        Uuid::parse_str(&user.id).unwrap_or_else(|_| Uuid::new_v4())
    } else {
        Uuid::new_v4()
    };

    let display_name = req
        .display_name
        .as_deref()
        .unwrap_or(email.split('@').next().unwrap_or(&email));

    let (ccr, passkey_reg) = state
        .webauthn
        .start_passkey_registration(user_uuid, &email, display_name, Some(exclude_creds))
        .map_err(|e| AppError::Internal(format!("webauthn error: {e}")))?;

    // Store challenge state
    let challenge_id = uuid::Uuid::new_v4().to_string();
    let now = crate::unix_now();
    let state_json = serde_json::to_string(&passkey_reg)
        .map_err(|e| AppError::Internal(format!("serialize error: {e}")))?;

    sqlx::query(
        "INSERT INTO webauthn_challenges (id, challenge_type, state_json, user_id, email, display_name, created_at, expires_at)
         VALUES (?, 'registration', ?, ?, ?, ?, ?, ?)",
    )
    .bind(&challenge_id)
    .bind(&state_json)
    .bind(existing_user.as_ref().map(|u| &u.id))
    .bind(&email)
    .bind(display_name)
    .bind(now)
    .bind(now + 300) // 5 minutes
    .execute(&state.db)
    .await?;

    Ok(Json(serde_json::json!({
        "challenge_id": challenge_id,
        "publicKey": ccr,
    })))
}

#[derive(Debug, Deserialize)]
struct RegisterCompleteRequest {
    challenge_id: String,
    credential: RegisterPublicKeyCredential,
}

async fn register_complete(
    State(state): State<AppState>,
    Json(req): Json<RegisterCompleteRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let now = crate::unix_now();

    // Look up and consume challenge
    let challenge = sqlx::query_as::<_, crate::models::WebauthnChallenge>(
        "SELECT * FROM webauthn_challenges WHERE id = ? AND challenge_type = 'registration' AND expires_at > ?",
    )
    .bind(&req.challenge_id)
    .bind(now)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::BadRequest("invalid or expired challenge".into()))?;

    // Delete challenge (single-use)
    sqlx::query("DELETE FROM webauthn_challenges WHERE id = ?")
        .bind(&req.challenge_id)
        .execute(&state.db)
        .await?;

    // Deserialize registration state
    let passkey_reg: PasskeyRegistration = serde_json::from_str(&challenge.state_json)
        .map_err(|e| AppError::Internal(format!("deserialize error: {e}")))?;

    // Complete registration
    let passkey = state
        .webauthn
        .finish_passkey_registration(&req.credential, &passkey_reg)
        .map_err(|e| AppError::BadRequest(format!("registration failed: {e}")))?;

    // Get or create user
    let user = if let Some(user_id) = &challenge.user_id {
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_one(&state.db)
            .await?
    } else {
        // Check if this is the first user (auto-admin)
        let user_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&state.db)
            .await?;
        let role = if user_count.0 == 0 { "admin" } else { "user" };

        let user_id = uuid::Uuid::new_v4().to_string();
        let email = challenge.email.as_deref().unwrap_or("");
        let email_domain = email.split('@').nth(1).unwrap_or("");
        let display_name = challenge.display_name.as_deref().unwrap_or("");

        sqlx::query(
            "INSERT INTO users (id, provider, provider_subject, email, email_domain, display_name, role, disabled, created_at)
             VALUES (?, 'webauthn', ?, ?, ?, ?, ?, 0, ?)",
        )
        .bind(&user_id)
        .bind(&user_id)
        .bind(email)
        .bind(email_domain)
        .bind(display_name)
        .bind(role)
        .bind(now)
        .execute(&state.db)
        .await?;

        sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
            .bind(&user_id)
            .fetch_one(&state.db)
            .await?
    };

    if user.disabled != 0 {
        return Err(AppError::BadRequest("user is disabled".into()));
    }

    // Store credential
    let cred_id = uuid::Uuid::new_v4().to_string();
    let credential_id_b64 = base64url_encode(passkey.cred_id());
    let credential_json = serde_json::to_string(&passkey)
        .map_err(|e| AppError::Internal(format!("serialize error: {e}")))?;

    sqlx::query(
        "INSERT INTO webauthn_credentials (id, user_id, credential_id, credential_json, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&cred_id)
    .bind(&user.id)
    .bind(&credential_id_b64)
    .bind(&credential_json)
    .bind(now)
    .execute(&state.db)
    .await?;

    // Issue session token
    let token = create_session_token(&state.auth_config, &user.id)
        .map_err(|e| AppError::Internal(format!("token error: {e}")))?;

    Ok(Json(serde_json::json!({
        "token": token,
        "user": user,
    })))
}

// ── Authentication ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct LoginBeginRequest {
    email: String,
}

async fn login_begin(
    State(state): State<AppState>,
    Json(req): Json<LoginBeginRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let email = req.email.trim().to_lowercase();

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE email = ?")
        .bind(&email)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::BadRequest("no account found for this email".into()))?;

    if user.disabled != 0 {
        return Err(AppError::BadRequest("user is disabled".into()));
    }

    let passkeys = load_passkeys(&state.db, &user.id).await?;
    if passkeys.is_empty() {
        return Err(AppError::BadRequest(
            "no passkeys registered for this account".into(),
        ));
    }

    let (rcr, passkey_auth) = state
        .webauthn
        .start_passkey_authentication(&passkeys)
        .map_err(|e| AppError::Internal(format!("webauthn error: {e}")))?;

    // Store challenge state
    let challenge_id = uuid::Uuid::new_v4().to_string();
    let now = crate::unix_now();
    let state_json = serde_json::to_string(&passkey_auth)
        .map_err(|e| AppError::Internal(format!("serialize error: {e}")))?;

    sqlx::query(
        "INSERT INTO webauthn_challenges (id, challenge_type, state_json, user_id, created_at, expires_at)
         VALUES (?, 'authentication', ?, ?, ?, ?)",
    )
    .bind(&challenge_id)
    .bind(&state_json)
    .bind(&user.id)
    .bind(now)
    .bind(now + 300)
    .execute(&state.db)
    .await?;

    Ok(Json(serde_json::json!({
        "challenge_id": challenge_id,
        "publicKey": rcr,
    })))
}

#[derive(Debug, Deserialize)]
struct LoginCompleteRequest {
    challenge_id: String,
    credential: PublicKeyCredential,
}

async fn login_complete(
    State(state): State<AppState>,
    Json(req): Json<LoginCompleteRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let now = crate::unix_now();

    // Look up and consume challenge
    let challenge = sqlx::query_as::<_, crate::models::WebauthnChallenge>(
        "SELECT * FROM webauthn_challenges WHERE id = ? AND challenge_type = 'authentication' AND expires_at > ?",
    )
    .bind(&req.challenge_id)
    .bind(now)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::BadRequest("invalid or expired challenge".into()))?;

    sqlx::query("DELETE FROM webauthn_challenges WHERE id = ?")
        .bind(&req.challenge_id)
        .execute(&state.db)
        .await?;

    let passkey_auth: PasskeyAuthentication = serde_json::from_str(&challenge.state_json)
        .map_err(|e| AppError::Internal(format!("deserialize error: {e}")))?;

    let auth_result = state
        .webauthn
        .finish_passkey_authentication(&req.credential, &passkey_auth)
        .map_err(|e| AppError::BadRequest(format!("authentication failed: {e}")))?;

    // Find the user via challenge
    let user_id = challenge
        .user_id
        .ok_or_else(|| AppError::Internal("challenge missing user_id".into()))?;

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
        .bind(&user_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::BadRequest("user not found".into()))?;

    if user.disabled != 0 {
        return Err(AppError::BadRequest("user is disabled".into()));
    }

    // Update credential counter to prevent cloning attacks
    let cred_id_b64 = base64url_encode(auth_result.cred_id());
    let cred_row = sqlx::query_as::<_, crate::models::WebauthnCredential>(
        "SELECT * FROM webauthn_credentials WHERE credential_id = ?",
    )
    .bind(&cred_id_b64)
    .fetch_optional(&state.db)
    .await?;

    if let Some(cred_row) = cred_row {
        // Re-serialize the passkey with updated counter
        if let Ok(mut passkey) =
            serde_json::from_str::<Passkey>(&cred_row.credential_json)
        {
            passkey.update_credential(&auth_result);
            if let Ok(updated_json) = serde_json::to_string(&passkey) {
                let _ = sqlx::query(
                    "UPDATE webauthn_credentials SET credential_json = ? WHERE id = ?",
                )
                .bind(&updated_json)
                .bind(&cred_row.id)
                .execute(&state.db)
                .await;
            }
        }
    }

    // Issue session token
    let token = create_session_token(&state.auth_config, &user.id)
        .map_err(|e| AppError::Internal(format!("token error: {e}")))?;

    Ok(Json(serde_json::json!({
        "token": token,
        "user": user,
    })))
}

// ── TOTP (fallback for non-WebAuthn browsers) ─────────────────────

#[derive(Debug, Deserialize)]
struct TotpRegisterRequest {
    username: String,
    display_name: Option<String>,
}

async fn totp_register(
    State(state): State<AppState>,
    Json(req): Json<TotpRegisterRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let username = req.username.trim().to_lowercase();
    if username.is_empty() {
        return Err(AppError::BadRequest("username is required".into()));
    }

    // Check if user already has TOTP set up (look up by display_name as username)
    let existing = sqlx::query_as::<_, User>("SELECT * FROM users WHERE display_name = ?")
        .bind(&username)
        .fetch_optional(&state.db)
        .await?;

    if let Some(ref user) = existing {
        if user.totp_secret.is_some() {
            return Err(AppError::BadRequest(
                "account already exists — use login instead".into(),
            ));
        }
    }

    // Generate TOTP secret
    let secret = totp_rs::Secret::generate_secret();
    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        secret.to_bytes().map_err(|e| AppError::Internal(format!("secret error: {e}")))?,
        Some("Orion Complex".to_string()),
        username.clone(),
    )
    .map_err(|e| AppError::Internal(format!("totp error: {e}")))?;

    let otpauth_url = totp.get_url();
    let secret_base32 = secret.to_encoded().to_string();

    // Store pending registration in challenges table
    let challenge_id = uuid::Uuid::new_v4().to_string();
    let now = crate::unix_now();

    let display_name = req.display_name.as_deref().unwrap_or(&username);

    sqlx::query(
        "INSERT INTO webauthn_challenges (id, challenge_type, state_json, user_id, email, display_name, created_at, expires_at)
         VALUES (?, 'totp_register', ?, ?, ?, ?, ?, ?)",
    )
    .bind(&challenge_id)
    .bind(&secret_base32)
    .bind(existing.as_ref().map(|u| &u.id))
    .bind(&username) // store username in email column for now
    .bind(display_name)
    .bind(now)
    .bind(now + 600) // 10 minutes to scan QR
    .execute(&state.db)
    .await?;

    Ok(Json(serde_json::json!({
        "challenge_id": challenge_id,
        "otpauth_url": otpauth_url,
        "secret": secret_base32,
    })))
}

#[derive(Debug, Deserialize)]
struct TotpVerifyRequest {
    challenge_id: String,
    code: String,
}

async fn totp_verify(
    State(state): State<AppState>,
    Json(req): Json<TotpVerifyRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let now = crate::unix_now();

    let challenge = sqlx::query_as::<_, crate::models::WebauthnChallenge>(
        "SELECT * FROM webauthn_challenges WHERE id = ? AND challenge_type = 'totp_register' AND expires_at > ?",
    )
    .bind(&req.challenge_id)
    .bind(now)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::BadRequest("invalid or expired registration".into()))?;

    let secret_base32 = &challenge.state_json;
    let email = challenge.email.as_deref().unwrap_or("");

    // Verify the code
    let secret = totp_rs::Secret::Encoded(secret_base32.clone());
    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        secret.to_bytes().map_err(|e| AppError::Internal(format!("secret error: {e}")))?,
        Some("Orion Complex".to_string()),
        email.to_string(),
    )
    .map_err(|e| AppError::Internal(format!("totp error: {e}")))?;

    if !verify_totp_code(&totp, &req.code) {
        return Err(AppError::BadRequest("incorrect code — try again".into()));
    }

    // Delete challenge
    sqlx::query("DELETE FROM webauthn_challenges WHERE id = ?")
        .bind(&req.challenge_id)
        .execute(&state.db)
        .await?;

    // Get or create user, store TOTP secret
    let username = challenge.email.as_deref().unwrap_or("");
    let user = if let Some(user_id) = &challenge.user_id {
        sqlx::query("UPDATE users SET totp_secret = ? WHERE id = ?")
            .bind(secret_base32)
            .bind(user_id)
            .execute(&state.db)
            .await?;
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_one(&state.db)
            .await?
    } else {
        let user_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&state.db)
            .await?;
        let role = if user_count.0 == 0 { "admin" } else { "user" };

        let user_id = uuid::Uuid::new_v4().to_string();
        let display_name = challenge.display_name.as_deref().unwrap_or(username);

        sqlx::query(
            "INSERT INTO users (id, provider, provider_subject, email, email_domain, display_name, role, disabled, totp_secret, created_at)
             VALUES (?, 'totp', ?, ?, '', ?, ?, 0, ?, ?)",
        )
        .bind(&user_id)
        .bind(&user_id)
        .bind(username)
        .bind(display_name)
        .bind(role)
        .bind(secret_base32)
        .bind(now)
        .execute(&state.db)
        .await?;

        sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
            .bind(&user_id)
            .fetch_one(&state.db)
            .await?
    };

    if user.disabled != 0 {
        return Err(AppError::BadRequest("user is disabled".into()));
    }

    let token = create_session_token(&state.auth_config, &user.id)
        .map_err(|e| AppError::Internal(format!("token error: {e}")))?;

    Ok(Json(serde_json::json!({
        "token": token,
        "user": user,
    })))
}

#[derive(Debug, Deserialize)]
struct TotpLoginRequest {
    code: String,
}

async fn totp_login(
    State(state): State<AppState>,
    Json(req): Json<TotpLoginRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let code = req.code.trim();
    if code.len() != 6 {
        return Err(AppError::BadRequest("code must be 6 digits".into()));
    }

    // Fetch all users with TOTP configured and find the matching one
    let users = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE totp_secret IS NOT NULL AND disabled = 0",
    )
    .fetch_all(&state.db)
    .await?;

    for user in &users {
        let secret_base32 = match user.totp_secret.as_deref() {
            Some(s) => s,
            None => continue,
        };

        let secret = totp_rs::Secret::Encoded(secret_base32.to_string());
        let totp = match totp_rs::TOTP::new(
            totp_rs::Algorithm::SHA1,
            6,
            3,
            30,
            match secret.to_bytes() {
                Ok(b) => b,
                Err(_) => continue,
            },
            Some("Orion Complex".to_string()),
            user.display_name.clone().unwrap_or_default(),
        ) {
            Ok(t) => t,
            Err(_) => continue,
        };

        if verify_totp_code(&totp, code) {
            let token = create_session_token(&state.auth_config, &user.id)
                .map_err(|e| AppError::Internal(format!("token error: {e}")))?;

            return Ok(Json(serde_json::json!({
                "token": token,
                "user": user,
            })));
        }
    }

    Err(AppError::BadRequest("incorrect code".into()))
}

// ── Helpers ─────────────────────────────────────────────────────────

async fn load_passkeys(db: &sqlx::SqlitePool, user_id: &str) -> Result<Vec<Passkey>, AppError> {
    let rows = sqlx::query_as::<_, crate::models::WebauthnCredential>(
        "SELECT * FROM webauthn_credentials WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;

    let mut passkeys = Vec::new();
    for row in rows {
        let pk: Passkey = serde_json::from_str(&row.credential_json)
            .map_err(|e| AppError::Internal(format!("credential deserialize error: {e}")))?;
        passkeys.push(pk);
    }
    Ok(passkeys)
}

fn base64url_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}
