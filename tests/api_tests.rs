use axum::http::StatusCode;
use axum::body::Body;
use axum::http::Request;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use std::sync::Arc;
use tower::ServiceExt;

// Import from the main crate
use orion_complex::AppState;
use orion_complex::auth::AuthConfig;

mod helpers {
    use super::*;
    use orion_complex::vm::stub::StubProvider;

    pub async fn setup() -> (axum::Router, String) {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("failed to create in-memory db");

        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();

        // Run migrations
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("failed to run migrations");

        let auth_config = AuthConfig {
            jwt_secret: "test-secret".into(),
            jwt_expiry_secs: 3600,
            google_client_id: None,
            microsoft_client_id: None,
            allowed_domains: vec![],
            webauthn_rp_id: "localhost".into(),
            webauthn_rp_origin: "http://localhost:3000".into(),
            webauthn_rp_name: "Test".into(),
        };

        // Create a test admin user
        sqlx::query(
            "INSERT INTO users (id, provider, provider_subject, email, email_domain, display_name, role, disabled, created_at)
             VALUES ('test-admin', 'test', 'sub1', 'admin@test.com', 'test.com', 'Test Admin', 'admin', 0, 1700000000)"
        )
        .execute(&pool)
        .await
        .unwrap();

        // Create a test regular user
        sqlx::query(
            "INSERT INTO users (id, provider, provider_subject, email, email_domain, display_name, role, disabled, created_at)
             VALUES ('test-user', 'test', 'sub2', 'user@test.com', 'test.com', 'Test User', 'user', 0, 1700000000)"
        )
        .execute(&pool)
        .await
        .unwrap();

        // Generate admin JWT
        let token = orion_complex::auth::create_session_token(&auth_config, "test-admin").unwrap();

        let vm_provider: Arc<dyn orion_complex::vm::VmProvider> =
            Arc::new(StubProvider::new());

        let webauthn = std::sync::Arc::new(
            webauthn_rs::WebauthnBuilder::new(
                "localhost",
                &url::Url::parse("http://localhost:3000").unwrap(),
            )
            .unwrap()
            .build()
            .unwrap(),
        );

        let state = AppState {
            db: pool,
            auth_config,
            http_client: reqwest::Client::new(),
            vm_provider,
            webauthn,
        };

        let app = orion_complex::api::router()
            .with_state(state.clone())
            .layer(axum::Extension(state));

        (app, token)
    }

    pub fn user_token(auth_config: &AuthConfig, user_id: &str) -> String {
        orion_complex::auth::create_session_token(auth_config, user_id).unwrap()
    }

    pub async fn get(app: &axum::Router, path: &str, token: &str) -> (StatusCode, Value) {
        let req = Request::builder()
            .method("GET")
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        (status, json)
    }

    pub async fn post(app: &axum::Router, path: &str, token: &str, body: Value) -> (StatusCode, Value) {
        let req = Request::builder()
            .method("POST")
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        (status, json)
    }

    pub async fn put(app: &axum::Router, path: &str, token: &str, body: Value) -> (StatusCode, Value) {
        let req = Request::builder()
            .method("PUT")
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        (status, json)
    }

    pub async fn delete(app: &axum::Router, path: &str, token: &str) -> StatusCode {
        let req = Request::builder()
            .method("DELETE")
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        resp.status()
    }
}

// ---- Health check ----

#[tokio::test]
async fn test_healthz() {
    let (app, _) = helpers::setup().await;
    let req = Request::builder()
        .uri("/v1/healthz")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ---- Auth ----

#[tokio::test]
async fn test_auth_me() {
    let (app, token) = helpers::setup().await;
    let (status, body) = helpers::get(&app, "/v1/auth/me", &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], "test-admin");
    assert_eq!(body["role"], "admin");
}

#[tokio::test]
async fn test_auth_no_token() {
    let (app, _) = helpers::setup().await;
    let req = Request::builder()
        .uri("/v1/auth/me")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_invalid_token() {
    let (app, _) = helpers::setup().await;
    let (status, _) = helpers::get(&app, "/v1/auth/me", "invalid-token").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---- Nodes ----

#[tokio::test]
async fn test_node_crud() {
    let (app, token) = helpers::setup().await;

    // List (empty)
    let (status, body) = helpers::get(&app, "/v1/nodes", &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 0);

    // Register
    let (status, node) = helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "test-node",
        "host_os": "linux",
        "host_arch": "x86_64",
        "cpu_cores": 8,
        "memory_bytes": 17179869184_i64,
        "disk_bytes_total": 536870912000_i64
    })).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(node["name"], "test-node");
    assert_eq!(node["online"], 1);
    let node_id = node["id"].as_str().unwrap().to_string();

    // Get
    let (status, body) = helpers::get(&app, &format!("/v1/nodes/{node_id}"), &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "test-node");

    // Update
    let (status, body) = helpers::put(&app, &format!("/v1/nodes/{node_id}"), &token, json!({
        "max_running_envs": 4
    })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["max_running_envs"], 4);

    // Delete
    let status = helpers::delete(&app, &format!("/v1/nodes/{node_id}"), &token).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_node_register_requires_admin() {
    let (app, _) = helpers::setup().await;
    let user_token = orion_complex::auth::create_session_token(
        &AuthConfig {
            jwt_secret: "test-secret".into(),
            jwt_expiry_secs: 3600,
            google_client_id: None,
            microsoft_client_id: None,
            allowed_domains: vec![],
            webauthn_rp_id: "localhost".into(),
            webauthn_rp_origin: "http://localhost:3000".into(),
            webauthn_rp_name: "Test".into(),
        },
        "test-user",
    ).unwrap();

    let (status, _) = helpers::post(&app, "/v1/nodes", &user_token, json!({
        "name": "node",
        "host_os": "linux",
        "host_arch": "x86_64",
        "cpu_cores": 1,
        "memory_bytes": 1000000,
        "disk_bytes_total": 1000000
    })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ---- Images ----

#[tokio::test]
async fn test_image_crud() {
    let (app, token) = helpers::setup().await;

    // Register
    let (status, image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "ubuntu-24.04",
        "provider": "libvirt",
        "guest_os": "linux",
        "guest_arch": "x86_64"
    })).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(image["name"], "ubuntu-24.04");
    let image_id = image["id"].as_str().unwrap().to_string();

    // List
    let (status, body) = helpers::get(&app, "/v1/images", &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 1);

    // Delete
    let status = helpers::delete(&app, &format!("/v1/images/{image_id}"), &token).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

// ---- Environments ----

#[tokio::test]
async fn test_environment_lifecycle() {
    let (app, token) = helpers::setup().await;

    // Setup: create a node and image
    let (_, node) = helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "node-1",
        "host_os": "linux",
        "host_arch": "x86_64",
        "cpu_cores": 8,
        "memory_bytes": 17179869184_i64,
        "disk_bytes_total": 536870912000_i64
    })).await;

    let (_, image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "test-image",
        "provider": "libvirt",
        "guest_os": "linux",
        "guest_arch": "x86_64"
    })).await;

    let image_id = image["id"].as_str().unwrap();

    // Create environment
    let (status, env) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image_id
    })).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(env["owner_user_id"], "test-admin");
    assert_eq!(env["node_id"], node["id"]);
    let env_id = env["id"].as_str().unwrap().to_string();

    // Wait for async VM creation
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Check state became running
    let (status, env) = helpers::get(&app, &format!("/v1/environments/{env_id}"), &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(env["state"], "running");

    // Check task was created
    let (_, tasks) = helpers::get(&app, "/v1/tasks", &token).await;
    let tasks = tasks.as_array().unwrap();
    assert!(!tasks.is_empty());
    assert_eq!(tasks[0]["state"], "completed");

    // Suspend
    let (status, env) = helpers::post(
        &app,
        &format!("/v1/environments/{env_id}/suspend"),
        &token,
        json!({}),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(env["state"], "suspending");

    // Invalid transition: can't suspend again
    let (status, _) = helpers::post(
        &app,
        &format!("/v1/environments/{env_id}/suspend"),
        &token,
        json!({}),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Destroy
    let status = helpers::delete(&app, &format!("/v1/environments/{env_id}"), &token).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_environment_create_no_nodes() {
    let (app, token) = helpers::setup().await;

    // Create an image but no nodes
    let (_, image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "test",
        "provider": "libvirt",
        "guest_os": "linux",
        "guest_arch": "x86_64"
    })).await;

    let (status, body) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"]
    })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("no available"));
}

#[tokio::test]
async fn test_environment_state_update() {
    let (app, token) = helpers::setup().await;

    // Setup
    helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "n1", "host_os": "macos", "host_arch": "arm64",
        "cpu_cores": 10, "memory_bytes": 34359738368_i64, "disk_bytes_total": 1000000000000_i64
    })).await;

    let (_, image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "macos-15", "provider": "macos", "guest_os": "macos", "guest_arch": "arm64"
    })).await;

    // Create env (macOS provider — no in-process dispatch)
    let (status, env) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"]
    })).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(env["state"], "creating"); // stays creating (agent-managed)
    let env_id = env["id"].as_str().unwrap().to_string();

    // Simulate node agent reporting state
    let (status, env) = helpers::put(
        &app,
        &format!("/v1/environments/{env_id}/state"),
        &token,
        json!({"state": "running"}),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(env["state"], "running");

    // Invalid state
    let (status, _) = helpers::put(
        &app,
        &format!("/v1/environments/{env_id}/state"),
        &token,
        json!({"state": "invalid_state"}),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---- Users ----

#[tokio::test]
async fn test_user_management() {
    let (app, token) = helpers::setup().await;

    // List users
    let (status, users) = helpers::get(&app, "/v1/users", &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(users.as_array().unwrap().len(), 2);

    // Get specific user
    let (status, user) = helpers::get(&app, "/v1/users/test-user", &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(user["role"], "user");

    // Promote to admin
    let (status, user) = helpers::put(&app, "/v1/users/test-user/role", &token, json!({
        "role": "admin"
    })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(user["role"], "admin");

    // Disable user
    let (status, user) = helpers::put(&app, "/v1/users/test-user/disabled", &token, json!({
        "disabled": true
    })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(user["disabled"], 1);
}

// ---- Scheduler: macOS vs Linux node matching ----

#[tokio::test]
async fn test_scheduler_matches_host_os() {
    let (app, token) = helpers::setup().await;

    // Create a Linux node and a macOS node
    helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "linux-node", "host_os": "linux", "host_arch": "x86_64",
        "cpu_cores": 8, "memory_bytes": 17179869184_i64, "disk_bytes_total": 500000000000_i64
    })).await;
    helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "macos-node", "host_os": "macos", "host_arch": "arm64",
        "cpu_cores": 10, "memory_bytes": 34359738368_i64, "disk_bytes_total": 1000000000000_i64
    })).await;

    // Create libvirt image -> should land on linux node
    let (_, linux_image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "ubuntu", "provider": "libvirt", "guest_os": "linux", "guest_arch": "x86_64"
    })).await;

    // Create macOS image -> should land on macos node
    let (_, macos_image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "macos-15", "provider": "macos", "guest_os": "macos", "guest_arch": "arm64"
    })).await;

    let (_, linux_env) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": linux_image["id"]
    })).await;

    let (_, macos_env) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": macos_image["id"]
    })).await;

    // Verify correct node assignment
    let linux_node_name = helpers::get(
        &app,
        &format!("/v1/nodes/{}", linux_env["node_id"].as_str().unwrap()),
        &token,
    ).await.1;
    assert_eq!(linux_node_name["host_os"], "linux");

    let macos_node_name = helpers::get(
        &app,
        &format!("/v1/nodes/{}", macos_env["node_id"].as_str().unwrap()),
        &token,
    ).await.1;
    assert_eq!(macos_node_name["host_os"], "macos");
}

// ---- Owner-scoped authorization ----

#[tokio::test]
async fn test_owner_scoped_environments() {
    let (app, admin_token) = helpers::setup().await;

    let user_token = helpers::user_token(
        &AuthConfig {
            jwt_secret: "test-secret".into(),
            jwt_expiry_secs: 3600,
            google_client_id: None,
            microsoft_client_id: None,
            allowed_domains: vec![],
            webauthn_rp_id: "localhost".into(),
            webauthn_rp_origin: "http://localhost:3000".into(),
            webauthn_rp_name: "Test".into(),
        },
        "test-user",
    );

    // Setup: create a node and image
    helpers::post(&app, "/v1/nodes", &admin_token, json!({
        "name": "node-1", "host_os": "linux", "host_arch": "x86_64",
        "cpu_cores": 8, "memory_bytes": 17179869184_i64, "disk_bytes_total": 536870912000_i64
    })).await;

    let (_, image) = helpers::post(&app, "/v1/images", &admin_token, json!({
        "name": "test-image", "provider": "libvirt", "guest_os": "linux", "guest_arch": "x86_64"
    })).await;

    // Admin creates an environment
    let (status, admin_env) = helpers::post(&app, "/v1/environments", &admin_token, json!({
        "image_id": image["id"]
    })).await;
    assert_eq!(status, StatusCode::CREATED);
    let admin_env_id = admin_env["id"].as_str().unwrap().to_string();

    // Regular user creates an environment
    let (status, user_env) = helpers::post(&app, "/v1/environments", &user_token, json!({
        "image_id": image["id"]
    })).await;
    assert_eq!(status, StatusCode::CREATED);
    let user_env_id = user_env["id"].as_str().unwrap().to_string();

    // All users see all environments (shared resource visibility)
    let (_, envs) = helpers::get(&app, "/v1/environments", &user_token).await;
    assert_eq!(envs.as_array().unwrap().len(), 2);

    let (_, envs) = helpers::get(&app, "/v1/environments", &admin_token).await;
    assert_eq!(envs.as_array().unwrap().len(), 2);

    // Regular user can view any environment
    let (status, _) = helpers::get(&app, &format!("/v1/environments/{user_env_id}"), &user_token).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = helpers::get(&app, &format!("/v1/environments/{admin_env_id}"), &user_token).await;
    assert_eq!(status, StatusCode::OK);

    // Shared resource system: any user can destroy any environment
    let status = helpers::delete(&app, &format!("/v1/environments/{admin_env_id}"), &user_token).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let status = helpers::delete(&app, &format!("/v1/environments/{user_env_id}"), &user_token).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

// ---- Environment restart ----

#[tokio::test]
async fn test_restart_clears_endpoints() {
    let (app, token) = helpers::setup().await;

    // Setup: create node and image
    helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "node-1", "host_os": "linux", "host_arch": "x86_64",
        "cpu_cores": 8, "memory_bytes": 17179869184_i64, "disk_bytes_total": 536870912000_i64
    })).await;
    let (_, image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "test-image", "provider": "libvirt", "guest_os": "linux", "guest_arch": "x86_64"
    })).await;

    // Create environment (StubProvider sets it to running immediately)
    let (status, env) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"]
    })).await;
    assert_eq!(status, StatusCode::CREATED);
    let env_id = env["id"].as_str().unwrap();

    // Wait for stub provider to finish creating
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Set fake endpoints via the endpoints API
    let (status, _) = helpers::put(&app, &format!("/v1/environments/{env_id}/endpoints"), &token, json!({
        "ssh_host": "10.0.0.1", "ssh_port": 22, "vnc_host": "10.0.0.1", "vnc_port": 5900
    })).await;
    assert_eq!(status, StatusCode::OK);

    // Verify endpoints are set
    let (_, env_data) = helpers::get(&app, &format!("/v1/environments/{env_id}"), &token).await;
    assert_eq!(env_data["vnc_host"].as_str(), Some("10.0.0.1"));
    assert_eq!(env_data["vnc_port"].as_i64(), Some(5900));

    // Simulate failure: force state to "failed"
    let (status, _) = helpers::put(&app, &format!("/v1/environments/{env_id}/state"), &token, json!({
        "state": "failed"
    })).await;
    assert_eq!(status, StatusCode::OK);

    // Restart: failed → creating
    let (status, restarted) = helpers::post(&app, &format!("/v1/environments/{env_id}/restart"), &token, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(restarted["state"].as_str(), Some("creating"));

    // Endpoints must be cleared
    assert!(restarted["vnc_host"].is_null(), "vnc_host should be cleared after restart");
    assert!(restarted["vnc_port"].is_null(), "vnc_port should be cleared after restart");
    assert!(restarted["ssh_host"].is_null(), "ssh_host should be cleared after restart");
    assert!(restarted["ssh_port"].is_null(), "ssh_port should be cleared after restart");
}

#[tokio::test]
async fn test_restart_only_from_failed() {
    let (app, token) = helpers::setup().await;

    // Setup
    helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "node-1", "host_os": "linux", "host_arch": "x86_64",
        "cpu_cores": 8, "memory_bytes": 17179869184_i64, "disk_bytes_total": 536870912000_i64
    })).await;
    let (_, image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "test-image", "provider": "libvirt", "guest_os": "linux", "guest_arch": "x86_64"
    })).await;

    let (_, env) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"]
    })).await;
    let env_id = env["id"].as_str().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Running → restart should fail
    let (status, body) = helpers::post(&app, &format!("/v1/environments/{env_id}/restart"), &token, json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("cannot transition"));
}

// ---- Environment TTL / expiration ----

#[tokio::test]
async fn test_environment_ttl() {
    let (app, token) = helpers::setup().await;

    // Setup
    helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "node-1", "host_os": "linux", "host_arch": "x86_64",
        "cpu_cores": 8, "memory_bytes": 17179869184_i64, "disk_bytes_total": 536870912000_i64
    })).await;

    let (_, image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "test-image", "provider": "libvirt", "guest_os": "linux", "guest_arch": "x86_64"
    })).await;

    // Create environment with TTL
    let (status, env) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"],
        "ttl_seconds": 3600
    })).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(env["expires_at"].as_i64().is_some());

    let created_at = env["created_at"].as_i64().unwrap();
    let expires_at = env["expires_at"].as_i64().unwrap();
    assert_eq!(expires_at - created_at, 3600);

    // Create environment without TTL — no expires_at
    let (status, env2) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"]
    })).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(env2["expires_at"].is_null());
}

// ---- Node heartbeat ----

#[tokio::test]
async fn test_node_heartbeat() {
    let (app, token) = helpers::setup().await;

    // Register a node
    let (_, node) = helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "hb-node", "host_os": "linux", "host_arch": "x86_64",
        "cpu_cores": 4, "memory_bytes": 8589934592_i64, "disk_bytes_total": 107374182400_i64
    })).await;
    let node_id = node["id"].as_str().unwrap().to_string();

    // Initially no heartbeat
    assert!(node["last_heartbeat_at"].is_null());

    // Send heartbeat
    let (status, _) = helpers::post(
        &app,
        &format!("/v1/nodes/{node_id}/heartbeat"),
        &token,
        json!({}),
    ).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Verify heartbeat was recorded
    let (_, updated_node) = helpers::get(&app, &format!("/v1/nodes/{node_id}"), &token).await;
    assert!(updated_node["last_heartbeat_at"].as_i64().is_some());
    assert_eq!(updated_node["online"], 1);

    // Heartbeat for non-existent node returns 404
    let (status, _) = helpers::post(
        &app,
        "/v1/nodes/nonexistent/heartbeat",
        &token,
        json!({}),
    ).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---- SSH key provisioning ----

#[tokio::test]
async fn test_user_ssh_key_provisioning() {
    let (app, token) = helpers::setup().await;

    // Add SSH keys for the admin user
    helpers::post(&app, "/v1/ssh-keys", &token, json!({
        "public_key": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5 admin@test"
    })).await;

    helpers::post(&app, "/v1/ssh-keys", &token, json!({
        "public_key": "ssh-rsa AAAAB3NzaC1yc2EAAA admin@test2"
    })).await;

    // Fetch keys via provisioning endpoint
    let (status, keys) = helpers::get(&app, "/v1/users/test-admin/ssh-keys", &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(keys.as_array().unwrap().len(), 2);

    // Fetch keys for user with no keys
    let (status, keys) = helpers::get(&app, "/v1/users/test-user/ssh-keys", &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(keys.as_array().unwrap().len(), 0);
}

// ---- Environment resources ----

#[tokio::test]
async fn test_environment_resource_allocation() {
    let (app, token) = helpers::setup().await;

    helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "node-1", "host_os": "linux", "host_arch": "x86_64",
        "cpu_cores": 8, "memory_bytes": 17179869184_i64, "disk_bytes_total": 536870912000_i64
    })).await;

    let (_, image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "test-image", "provider": "libvirt", "guest_os": "linux", "guest_arch": "x86_64"
    })).await;

    // Create with explicit resources
    let (status, env) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"],
        "vcpus": 2,
        "memory_bytes": 4294967296_i64,
        "disk_bytes": 21474836480_i64
    })).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(env["vcpus"], 2);
    assert_eq!(env["memory_bytes"], 4294967296_i64);
    assert_eq!(env["disk_bytes"], 21474836480_i64);

    // Create with defaults (2 vCPUs, 4GB RAM, 20GB disk)
    let (status, env2) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"]
    })).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(env2["vcpus"], 2); // default
    assert_eq!(env2["memory_bytes"], 4294967296_i64); // 4 GB default
}

// ---- Resource-aware scheduling ----

#[tokio::test]
async fn test_scheduler_respects_resource_limits() {
    let (app, token) = helpers::setup().await;

    // Create a small node: 4 cores, 8GB RAM, 100GB disk
    // With default utilization ratios: cpu=0.7, mem=0.6, disk=0.8
    // Effective capacity: ~2.8 vCPUs, ~4.8GB RAM, ~80GB disk
    helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "small-node", "host_os": "linux", "host_arch": "x86_64",
        "cpu_cores": 4, "memory_bytes": 8589934592_i64, "disk_bytes_total": 107374182400_i64
    })).await;

    let (_, image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "test", "provider": "libvirt", "guest_os": "linux", "guest_arch": "x86_64"
    })).await;

    // First env: 2 vCPUs, 4GB — should fit
    let (status, _) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"],
        "vcpus": 2,
        "memory_bytes": 4294967296_i64,
        "disk_bytes": 21474836480_i64
    })).await;
    assert_eq!(status, StatusCode::CREATED);

    // Second env: 2 vCPUs, 4GB — should fail (exceeds memory capacity: 4.8GB available - 4GB used = 0.8GB left < 4GB requested)
    let (status, body) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"],
        "vcpus": 2,
        "memory_bytes": 4294967296_i64,
        "disk_bytes": 21474836480_i64
    })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("no available"));
}

// ---- Environment events ----

#[tokio::test]
async fn test_environment_events() {
    let (app, token) = helpers::setup().await;

    helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "n1", "host_os": "linux", "host_arch": "x86_64",
        "cpu_cores": 8, "memory_bytes": 17179869184_i64, "disk_bytes_total": 536870912000_i64
    })).await;

    let (_, image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "test", "provider": "libvirt", "guest_os": "linux", "guest_arch": "x86_64"
    })).await;

    let (_, env) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"]
    })).await;
    let env_id = env["id"].as_str().unwrap().to_string();

    // Wait for async create to complete
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Fetch events
    let (status, events) = helpers::get(
        &app,
        &format!("/v1/environments/{env_id}/events"),
        &token,
    ).await;
    assert_eq!(status, StatusCode::OK);
    let events = events.as_array().unwrap();
    assert!(events.len() >= 2); // "created" + "state_change" (creating->running)

    // Should have a "created" event and a state_change to "running"
    assert!(events.iter().any(|e| e["event_type"] == "created"));
    assert!(events.iter().any(|e| e["event_type"] == "state_change" && e["to_state"] == "running"));
}

// ---- Task error tracking ----

#[tokio::test]
async fn test_task_completed_at() {
    let (app, token) = helpers::setup().await;

    helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "n1", "host_os": "linux", "host_arch": "x86_64",
        "cpu_cores": 8, "memory_bytes": 17179869184_i64, "disk_bytes_total": 536870912000_i64
    })).await;

    let (_, image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "test", "provider": "libvirt", "guest_os": "linux", "guest_arch": "x86_64"
    })).await;

    helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"]
    })).await;

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let (_, tasks) = helpers::get(&app, "/v1/tasks", &token).await;
    let tasks = tasks.as_array().unwrap();
    assert!(!tasks.is_empty());
    assert_eq!(tasks[0]["state"], "completed");
    assert!(tasks[0]["completed_at"].as_i64().is_some());
}

// ---- Migrate endpoint ----

#[tokio::test]
async fn test_migrate_environment() {
    let (app, token) = helpers::setup().await;

    // Create two linux nodes
    let (_, node1) = helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "node-1", "host_os": "linux", "host_arch": "x86_64",
        "cpu_cores": 8, "memory_bytes": 17179869184_i64, "disk_bytes_total": 536870912000_i64
    })).await;
    let (_, node2) = helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "node-2", "host_os": "linux", "host_arch": "x86_64",
        "cpu_cores": 8, "memory_bytes": 17179869184_i64, "disk_bytes_total": 536870912000_i64
    })).await;

    let (_, image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "test", "provider": "libvirt", "guest_os": "linux", "guest_arch": "x86_64"
    })).await;

    // Create env on node1
    let (_, env) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"],
        "node_id": node1["id"]
    })).await;
    let env_id = env["id"].as_str().unwrap().to_string();

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Suspend first (migration requires suspended state)
    helpers::post(&app, &format!("/v1/environments/{env_id}/suspend"), &token, json!({})).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Migrate to node2
    let (status, env) = helpers::post(
        &app,
        &format!("/v1/environments/{env_id}/migrate"),
        &token,
        json!({"target_node_id": node2["id"]}),
    ).await;
    assert_eq!(status, StatusCode::OK);
    // State may be "migrating" or already "suspended" (stub completes instantly)
    let state_val = env["state"].as_str().unwrap();
    assert!(state_val == "migrating" || state_val == "suspended", "unexpected state: {state_val}");

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // After migration completes, env should be on node2 in suspended state
    let (_, env) = helpers::get(&app, &format!("/v1/environments/{env_id}"), &token).await;
    assert_eq!(env["node_id"], node2["id"]);
    assert_eq!(env["state"], "suspended");
}

#[tokio::test]
async fn test_migrate_validation() {
    let (app, token) = helpers::setup().await;

    helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "linux-node", "host_os": "linux", "host_arch": "x86_64",
        "cpu_cores": 8, "memory_bytes": 17179869184_i64, "disk_bytes_total": 536870912000_i64
    })).await;
    let (_, macos_node) = helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "macos-node", "host_os": "macos", "host_arch": "arm64",
        "cpu_cores": 10, "memory_bytes": 34359738368_i64, "disk_bytes_total": 1000000000000_i64
    })).await;

    let (_, image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "test", "provider": "libvirt", "guest_os": "linux", "guest_arch": "x86_64"
    })).await;

    let (_, env) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"]
    })).await;
    let env_id = env["id"].as_str().unwrap().to_string();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Can't migrate running env
    let (status, _) = helpers::post(
        &app,
        &format!("/v1/environments/{env_id}/migrate"),
        &token,
        json!({"target_node_id": macos_node["id"]}),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Suspend it
    helpers::post(&app, &format!("/v1/environments/{env_id}/suspend"), &token, json!({})).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Can't migrate to wrong host_os
    let (status, body) = helpers::post(
        &app,
        &format!("/v1/environments/{env_id}/migrate"),
        &token,
        json!({"target_node_id": macos_node["id"]}),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("host_os"));
}

// ---- Pagination ----

#[tokio::test]
async fn test_pagination() {
    let (app, token) = helpers::setup().await;

    // Create 3 nodes
    for i in 0..3 {
        helpers::post(&app, "/v1/nodes", &token, json!({
            "name": format!("node-{i}"), "host_os": "linux", "host_arch": "x86_64",
            "cpu_cores": 8, "memory_bytes": 17179869184_i64, "disk_bytes_total": 536870912000_i64
        })).await;
    }

    // Default pagination returns all 3
    let (_, nodes) = helpers::get(&app, "/v1/nodes", &token).await;
    assert_eq!(nodes.as_array().unwrap().len(), 3);

    // Limit to 2
    let (_, nodes) = helpers::get(&app, "/v1/nodes?limit=2", &token).await;
    assert_eq!(nodes.as_array().unwrap().len(), 2);

    // Offset 2, should get 1
    let (_, nodes) = helpers::get(&app, "/v1/nodes?limit=2&offset=2", &token).await;
    assert_eq!(nodes.as_array().unwrap().len(), 1);
}

// ---- TTL extension ----

#[tokio::test]
async fn test_extend_ttl() {
    let (app, token) = helpers::setup().await;

    helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "n1", "host_os": "linux", "host_arch": "x86_64",
        "cpu_cores": 8, "memory_bytes": 17179869184_i64, "disk_bytes_total": 536870912000_i64
    })).await;

    let (_, image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "test", "provider": "libvirt", "guest_os": "linux", "guest_arch": "x86_64"
    })).await;

    // Create with TTL
    let (_, env) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"],
        "ttl_seconds": 1800
    })).await;
    let env_id = env["id"].as_str().unwrap().to_string();
    let original_expires = env["expires_at"].as_i64().unwrap();

    // Extend TTL by 1 hour
    let (status, env) = helpers::post(
        &app,
        &format!("/v1/environments/{env_id}/extend-ttl"),
        &token,
        json!({"ttl_seconds": 3600}),
    ).await;
    assert_eq!(status, StatusCode::OK);
    let new_expires = env["expires_at"].as_i64().unwrap();
    assert!(new_expires > original_expires);

    // Extend TTL on env without TTL (adds one)
    let (_, env2) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"]
    })).await;
    let env2_id = env2["id"].as_str().unwrap().to_string();
    assert!(env2["expires_at"].is_null());

    let (status, env2) = helpers::post(
        &app,
        &format!("/v1/environments/{env2_id}/extend-ttl"),
        &token,
        json!({"ttl_seconds": 7200}),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert!(env2["expires_at"].as_i64().is_some());

    // Invalid: negative TTL
    let (status, body) = helpers::post(
        &app,
        &format!("/v1/environments/{env_id}/extend-ttl"),
        &token,
        json!({"ttl_seconds": -100}),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("positive"));
}

// ---- Snapshot restore ----

#[tokio::test]
async fn test_snapshot_restore() {
    let (app, token) = helpers::setup().await;

    helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "n1", "host_os": "linux", "host_arch": "x86_64",
        "cpu_cores": 8, "memory_bytes": 17179869184_i64, "disk_bytes_total": 536870912000_i64
    })).await;

    let (_, image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "test", "provider": "libvirt", "guest_os": "linux", "guest_arch": "x86_64"
    })).await;

    let (_, env) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"]
    })).await;
    let env_id = env["id"].as_str().unwrap().to_string();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Create snapshot with name
    let (status, snap) = helpers::post(
        &app,
        &format!("/v1/environments/{env_id}/snapshots"),
        &token,
        json!({"name": "before-update"}),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(snap["name"], "before-update");
    let snap_id = snap["id"].as_str().unwrap().to_string();

    // Restore snapshot
    let (status, _) = helpers::post(
        &app,
        &format!("/v1/environments/{env_id}/snapshots/{snap_id}/restore"),
        &token,
        json!({}),
    ).await;
    assert_eq!(status, StatusCode::OK);

    // Restore non-existent snapshot
    let (status, _) = helpers::post(
        &app,
        &format!("/v1/environments/{env_id}/snapshots/nonexistent/restore"),
        &token,
        json!({}),
    ).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---- Node utilization ----

#[tokio::test]
async fn test_node_usage() {
    let (app, token) = helpers::setup().await;

    let (_, node) = helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "n1", "host_os": "linux", "host_arch": "x86_64",
        "cpu_cores": 8, "memory_bytes": 17179869184_i64, "disk_bytes_total": 536870912000_i64
    })).await;
    let node_id = node["id"].as_str().unwrap().to_string();

    // Initially no usage
    let (status, usage) = helpers::get(&app, &format!("/v1/nodes/{node_id}/usage"), &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(usage["environment_count"], 0);
    assert_eq!(usage["vcpus_used"], 0);
    assert_eq!(usage["memory_bytes_used"], 0);

    // Create an environment
    let (_, image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "test", "provider": "libvirt", "guest_os": "linux", "guest_arch": "x86_64"
    })).await;

    helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"],
        "vcpus": 4,
        "memory_bytes": 8589934592_i64,
        "disk_bytes": 21474836480_i64
    })).await;

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Now should show usage
    let (_, usage) = helpers::get(&app, &format!("/v1/nodes/{node_id}/usage"), &token).await;
    assert_eq!(usage["environment_count"], 1);
    assert_eq!(usage["vcpus_used"], 4);
    assert_eq!(usage["memory_bytes_used"], 8589934592_i64);
    assert_eq!(usage["disk_bytes_used"], 21474836480_i64);

    // Capacity values should be based on utilization ratios
    // CPU: 8 * 0.7 = 5.6 -> 5
    assert_eq!(usage["vcpus_capacity"], 5);
    // Memory: 17179869184 * 0.6 = 10307921510
    assert_eq!(usage["memory_bytes_capacity"], 10307921510_i64);
}

// ---- Environment-scoped tasks ----

#[tokio::test]
async fn test_environment_tasks() {
    let (app, token) = helpers::setup().await;

    helpers::post(&app, "/v1/nodes", &token, json!({
        "name": "n1", "host_os": "linux", "host_arch": "x86_64",
        "cpu_cores": 8, "memory_bytes": 17179869184_i64, "disk_bytes_total": 536870912000_i64
    })).await;

    let (_, image) = helpers::post(&app, "/v1/images", &token, json!({
        "name": "test", "provider": "libvirt", "guest_os": "linux", "guest_arch": "x86_64"
    })).await;

    // Create two environments
    let (_, env1) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"]
    })).await;
    let env1_id = env1["id"].as_str().unwrap().to_string();

    let (_, env2) = helpers::post(&app, "/v1/environments", &token, json!({
        "image_id": image["id"]
    })).await;
    let env2_id = env2["id"].as_str().unwrap().to_string();

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Each env should have its own tasks
    let (status, tasks1) = helpers::get(
        &app,
        &format!("/v1/environments/{env1_id}/tasks"),
        &token,
    ).await;
    assert_eq!(status, StatusCode::OK);
    let tasks1 = tasks1.as_array().unwrap();
    assert_eq!(tasks1.len(), 1);
    assert_eq!(tasks1[0]["env_id"], env1_id);

    let (_, tasks2) = helpers::get(
        &app,
        &format!("/v1/environments/{env2_id}/tasks"),
        &token,
    ).await;
    let tasks2 = tasks2.as_array().unwrap();
    assert_eq!(tasks2.len(), 1);
    assert_eq!(tasks2[0]["env_id"], env2_id);

    // Global tasks list should have both
    let (_, all_tasks) = helpers::get(&app, "/v1/tasks", &token).await;
    assert!(all_tasks.as_array().unwrap().len() >= 2);
}

// ---- TOTP Authentication ----

#[tokio::test]
async fn test_totp_register_verify_login() {
    let (app, _admin_token) = helpers::setup().await;

    // Step 1: Register — get challenge + secret
    let (status, body) = helpers::post(
        &app,
        "/v1/auth/totp/register",
        "", // no auth needed
        json!({ "username": "alice" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "register failed: {body}");
    let challenge_id = body["challenge_id"].as_str().unwrap();
    let secret_base32 = body["secret"].as_str().unwrap();
    assert!(body["otpauth_url"].as_str().unwrap().starts_with("otpauth://totp/"));

    // Step 2: Generate valid code from the secret
    let secret = totp_rs::Secret::Encoded(secret_base32.to_string());
    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        3,
        30,
        secret.to_bytes().unwrap(),
        Some("Orion Complex".to_string()),
        "alice".to_string(),
    )
    .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let code = totp.generate(now);

    // Step 3: Verify — complete registration
    let (status, body) = helpers::post(
        &app,
        "/v1/auth/totp/verify",
        "",
        json!({ "challenge_id": challenge_id, "code": code }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "verify failed: {body}");
    assert!(body["token"].as_str().is_some(), "no token in verify response");
    assert!(body["user"]["id"].as_str().is_some(), "no user in verify response");

    // Step 4: Login with a fresh code
    let code = totp.generate(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    );
    let (status, body) = helpers::post(
        &app,
        "/v1/auth/totp/login",
        "",
        json!({ "code": code }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "login failed: {body}");
    assert!(body["token"].as_str().is_some(), "no token in login response");

    // Step 5: Login with wrong code should fail
    let (status, body) = helpers::post(
        &app,
        "/v1/auth/totp/login",
        "",
        json!({ "code": "000000" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"].as_str().unwrap(), "incorrect code");
}

#[tokio::test]
async fn test_totp_register_duplicate_username() {
    let (app, _admin_token) = helpers::setup().await;

    // Register alice
    let (status, body) = helpers::post(
        &app,
        "/v1/auth/totp/register",
        "",
        json!({ "username": "bob" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let challenge_id = body["challenge_id"].as_str().unwrap();
    let secret_base32 = body["secret"].as_str().unwrap();

    // Complete registration
    let secret = totp_rs::Secret::Encoded(secret_base32.to_string());
    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1, 6, 3, 30,
        secret.to_bytes().unwrap(),
        Some("Orion Complex".to_string()), "bob".to_string(),
    ).unwrap();
    let code = totp.generate(
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
    );
    let (status, _) = helpers::post(
        &app, "/v1/auth/totp/verify", "",
        json!({ "challenge_id": challenge_id, "code": code }),
    ).await;
    assert_eq!(status, StatusCode::OK);

    // Try to register same username again — should fail
    let (status, body) = helpers::post(
        &app,
        "/v1/auth/totp/register",
        "",
        json!({ "username": "bob" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("already exists"));
}

#[tokio::test]
async fn test_totp_verify_wrong_code() {
    let (app, _admin_token) = helpers::setup().await;

    let (_, body) = helpers::post(
        &app,
        "/v1/auth/totp/register",
        "",
        json!({ "username": "charlie" }),
    )
    .await;
    let challenge_id = body["challenge_id"].as_str().unwrap();

    // Verify with wrong code
    let (status, body) = helpers::post(
        &app,
        "/v1/auth/totp/verify",
        "",
        json!({ "challenge_id": challenge_id, "code": "999999" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("incorrect code"));
}

#[tokio::test]
async fn test_totp_login_no_users() {
    let (app, _admin_token) = helpers::setup().await;

    // Login when no TOTP users exist — should fail gracefully
    let (status, body) = helpers::post(
        &app,
        "/v1/auth/totp/login",
        "",
        json!({ "code": "123456" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"].as_str().unwrap(), "incorrect code");
}
