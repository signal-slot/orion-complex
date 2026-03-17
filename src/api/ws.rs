use axum::Router;
use axum::extract::{Path, Query, State, WebSocketUpgrade};
use axum::extract::ws::{Message, WebSocket};
use axum::response::IntoResponse;
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::AppState;
use crate::auth::validate_session_token;
use crate::error::AppError;
use crate::models::User;

use super::environments::is_agent_managed;
use crate::vm::provider_id_for;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/environments/{env_id}/ws/vnc", get(ws_vnc))
        .route("/v1/environments/{env_id}/ws/ssh", get(ws_ssh))
}

#[derive(Debug, Deserialize)]
struct WsAuth {
    token: String,
}

async fn authenticate_ws(state: &AppState, token: &str) -> Result<User, AppError> {
    let claims = validate_session_token(&state.auth_config, token)
        .map_err(|_| AppError::Unauthorized("invalid token".into()))?;

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
        .bind(&claims.sub)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::Unauthorized("user not found".into()))?;

    if user.disabled != 0 {
        return Err(AppError::Unauthorized("user disabled".into()));
    }

    Ok(user)
}

/// Resolve VNC target (host, port) for an environment.
async fn resolve_vnc_target(
    state: &AppState,
    user: &User,
    env_id: &str,
) -> Result<(String, u16), AppError> {
    let env = super::fetch_env(state, env_id).await?;
    let _ = user; // auth checked, but all users can view

    if env.state.as_deref() != Some("running") {
        return Err(AppError::BadRequest("environment is not running".into()));
    }

    let provider = env.provider.as_deref().unwrap_or("libvirt");
    if is_agent_managed(provider) {
        if let (Some(host), Some(port)) = (&env.vnc_host, env.vnc_port) {
            return Ok((host.clone(), port as u16));
        }
        return Err(AppError::BadRequest(
            "VNC not available — waiting for node agent to report endpoint".into(),
        ));
    }

    let provider_id = provider_id_for(provider, &env.id);
    let info = state
        .vm_provider
        .get_vm_info(&provider_id)
        .await
        .map_err(AppError::Internal)?;

    let host = info
        .vnc_host
        .ok_or_else(|| AppError::BadRequest("VNC not available for this environment".into()))?;
    let port = info
        .vnc_port
        .ok_or_else(|| AppError::BadRequest("VNC not available for this environment".into()))?;

    Ok((host, port))
}

/// Resolve SSH target (host, port, username) for an environment.
async fn resolve_ssh_target(
    state: &AppState,
    user: &User,
    env_id: &str,
) -> Result<(String, u16, String), AppError> {
    let env = super::fetch_env(state, env_id).await?;
    let _ = user; // auth checked, but all users can view

    if env.state.as_deref() != Some("running") {
        return Err(AppError::BadRequest("environment is not running".into()));
    }

    // Determine default username based on image/OS
    let image = if let Some(ref img_id) = env.image_id {
        sqlx::query_as::<_, crate::models::Image>("SELECT * FROM images WHERE id = ?")
            .bind(img_id)
            .fetch_optional(&state.db)
            .await?
    } else {
        None
    };

    let default_user = match image.as_ref().and_then(|i| i.guest_os.as_deref()) {
        Some("macos") => "admin",
        _ => "ubuntu",
    };

    let provider = env.provider.as_deref().unwrap_or("libvirt");
    if is_agent_managed(provider) {
        if let (Some(host), Some(port)) = (&env.ssh_host, env.ssh_port) {
            return Ok((host.clone(), port as u16, default_user.to_string()));
        }
        return Err(AppError::BadRequest(
            "SSH not available — waiting for node agent to report endpoint".into(),
        ));
    }

    let provider_id = provider_id_for(provider, &env.id);
    let info = state
        .vm_provider
        .get_vm_info(&provider_id)
        .await
        .map_err(AppError::Internal)?;

    Ok((info.ssh_host, info.ssh_port, default_user.to_string()))
}

// ── VNC WebSocket Proxy ─────────────────────────────────────────────

async fn ws_vnc(
    State(state): State<AppState>,
    Path(env_id): Path<String>,
    Query(auth): Query<WsAuth>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, AppError> {
    let user = authenticate_ws(&state, &auth.token).await?;
    let (host, port) = resolve_vnc_target(&state, &user, &env_id).await?;

    Ok(ws.on_upgrade(move |socket| handle_vnc_proxy(socket, host, port)))
}

async fn handle_vnc_proxy(ws: WebSocket, host: String, port: u16) {
    tracing::info!("VNC proxy: connecting to {host}:{port}");
    // On macOS 15+, non-system binaries are blocked from local VM network access
    // by Local Network Privacy. Use /usr/bin/nc as a bridge since system binaries
    // are exempt from this restriction.
    let mut child = match tokio::process::Command::new("/usr/bin/nc")
        .arg(&host)
        .arg(port.to_string())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => {
            tracing::info!("VNC proxy: nc bridge spawned to {host}:{port}");
            c
        }
        Err(e) => {
            tracing::error!("VNC proxy: failed to spawn nc to {host}:{port}: {e}");
            return;
        }
    };

    let mut tcp_read = match child.stdout.take() {
        Some(s) => s,
        None => {
            tracing::error!("VNC proxy: no stdout from nc");
            return;
        }
    };
    let mut tcp_write = match child.stdin.take() {
        Some(s) => s,
        None => {
            tracing::error!("VNC proxy: no stdin from nc");
            return;
        }
    };
    let (mut ws_sink, mut ws_stream) = ws.split();

    let ws_to_tcp = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_stream.next().await {
            match msg {
                Message::Binary(data) => {
                    if tcp_write.write_all(&data).await.is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    let tcp_to_ws = tokio::spawn(async move {
        let mut buf = vec![0u8; 16384];
        loop {
            match tcp_read.read(&mut buf).await {
                Ok(0) => {
                    tracing::info!("VNC proxy: TCP read EOF");
                    break;
                }
                Ok(n) => {
                    if ws_sink
                        .send(Message::Binary(buf[..n].to_vec().into()))
                        .await
                        .is_err()
                    {
                        tracing::info!("VNC proxy: WS send failed");
                        break;
                    }
                }
                Err(e) => {
                    tracing::error!("VNC proxy: TCP read error: {e}");
                    break;
                }
            }
        }
    });

    tokio::select! {
        r = ws_to_tcp => { tracing::info!("VNC proxy: ws_to_tcp ended: {r:?}"); },
        r = tcp_to_ws => { tracing::info!("VNC proxy: tcp_to_ws ended: {r:?}"); },
    }
}

// ── SSH WebSocket Terminal ──────────────────────────────────────────

async fn ws_ssh(
    State(state): State<AppState>,
    Path(env_id): Path<String>,
    Query(auth): Query<WsAuth>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, AppError> {
    let user = authenticate_ws(&state, &auth.token).await?;
    let (host, port, ssh_user) = resolve_ssh_target(&state, &user, &env_id).await?;

    Ok(ws.on_upgrade(move |socket| {
        handle_ssh_proxy(socket, host, port, ssh_user)
    }))
}

struct SshHandler;

#[async_trait::async_trait]
impl russh::client::Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true) // Accept all keys (internal VMs)
    }
}

/// SSH credentials sent by the browser as the first WebSocket text message.
#[derive(Deserialize)]
struct SshCredentials {
    username: String,
    password: String,
    cols: Option<u32>,
    rows: Option<u32>,
}

async fn handle_ssh_proxy(ws: WebSocket, host: String, port: u16, _default_user: String) {
    let (mut ws_sink, mut ws_stream) = ws.split();

    // Wait for credentials from the browser (first text message)
    let creds: SshCredentials = loop {
        match ws_stream.next().await {
            Some(Ok(Message::Text(text))) => {
                match serde_json::from_str::<SshCredentials>(&text) {
                    Ok(c) => break c,
                    Err(e) => {
                        let _ = ws_sink
                            .send(Message::Text(
                                format!("\r\nInvalid credentials format: {e}\r\n").into(),
                            ))
                            .await;
                        return;
                    }
                }
            }
            Some(Ok(Message::Close(_))) | None => return,
            _ => continue,
        }
    };

    tracing::info!(
        "SSH proxy: connecting as {}@{host}:{port}",
        creds.username
    );

    // Spawn nc to bypass macOS Local Network Privacy
    let mut child = match tokio::process::Command::new("/usr/bin/nc")
        .arg(&host)
        .arg(port.to_string())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = ws_sink
                .send(Message::Text(
                    format!("\r\nSSH connection failed: {e}\r\n").into(),
                ))
                .await;
            return;
        }
    };

    let nc_stdin = child.stdin.take().unwrap();
    let nc_stdout = child.stdout.take().unwrap();
    let nc_stream = tokio::io::join(nc_stdout, nc_stdin);

    let config = Arc::new(russh::client::Config::default());
    let mut session = match russh::client::connect_stream(config, nc_stream, SshHandler).await {
        Ok(s) => s,
        Err(e) => {
            let _ = ws_sink
                .send(Message::Text(
                    format!("\r\nSSH connection failed: {e}\r\n").into(),
                ))
                .await;
            return;
        }
    };

    // Authenticate with user-provided credentials
    match session
        .authenticate_password(&creds.username, &creds.password)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            let _ = ws_sink
                .send(Message::Text(
                    "\r\nAuthentication failed.\r\n".into(),
                ))
                .await;
            return;
        }
        Err(e) => {
            let _ = ws_sink
                .send(Message::Text(
                    format!("\r\nAuthentication error: {e}\r\n").into(),
                ))
                .await;
            return;
        }
    }

    let cols = creds.cols.unwrap_or(80);
    let rows = creds.rows.unwrap_or(24);

    let channel = match session.channel_open_session().await {
        Ok(ch) => ch,
        Err(e) => {
            tracing::error!("SSH channel open failed: {e}");
            return;
        }
    };

    if let Err(e) = channel
        .request_pty(false, "xterm-256color", cols, rows, 0, 0, &[])
        .await
    {
        tracing::error!("SSH PTY request failed: {e}");
        return;
    }

    if let Err(e) = channel.request_shell(false).await {
        tracing::error!("SSH shell request failed: {e}");
        return;
    }

    let mut stream = channel.into_stream();
    let (mut ssh_read, mut ssh_write) = tokio::io::split(&mut stream);

    let ws_to_ssh = async {
        while let Some(Ok(msg)) = ws_stream.next().await {
            match msg {
                Message::Text(text) => {
                    if ssh_write.write_all(text.as_bytes()).await.is_err() {
                        break;
                    }
                }
                Message::Binary(data) => {
                    if ssh_write.write_all(&data).await.is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    };

    let ssh_to_ws = async {
        let mut buf = vec![0u8; 8192];
        loop {
            match ssh_read.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if ws_sink
                        .send(Message::Binary(buf[..n].to_vec().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    };

    tokio::select! {
        _ = ws_to_ssh => {},
        _ = ssh_to_ws => {},
    }
}
