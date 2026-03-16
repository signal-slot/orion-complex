use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use serde::Serialize;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::AppError;
use crate::models::{AttachUsbRequest, UsbAttachment};
use crate::vm::libvirt;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/usb-devices", get(list_host_devices))
        .route(
            "/v1/environments/{env_id}/usb-devices",
            get(list_env_devices),
        )
        .route(
            "/v1/environments/{env_id}/usb-devices",
            post(attach_device),
        )
        .route(
            "/v1/environments/{env_id}/usb-devices/{attachment_id}",
            delete(detach_device),
        )
}

#[derive(Debug, Serialize)]
struct HostUsbDevice {
    bus: String,
    device: String,
    vendor_id: String,
    product_id: String,
    description: String,
    attached_to_env_id: Option<String>,
}

fn require_libvirt_uri(state: &AppState) -> Result<String, AppError> {
    state
        .libvirt_uri
        .clone()
        .ok_or_else(|| AppError::BadRequest("USB passthrough requires libvirt provider".into()))
}

fn validate_hex4(s: &str, field: &str) -> Result<(), AppError> {
    if s.len() != 4 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::BadRequest(format!(
            "{field} must be a 4-character hex string (e.g. \"1d6b\")"
        )));
    }
    Ok(())
}

async fn list_host_devices(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<HostUsbDevice>>, AppError> {
    let devices = libvirt::list_host_usb_devices()
        .await
        .map_err(|e| AppError::Internal(e))?;

    let attachments =
        sqlx::query_as::<_, UsbAttachment>("SELECT * FROM usb_attachments")
            .fetch_all(&state.db)
            .await?;

    let result: Vec<HostUsbDevice> = devices
        .into_iter()
        .map(|(bus, device, vendor_id, product_id, description)| {
            let attached_to = attachments
                .iter()
                .find(|a| a.vendor_id == vendor_id && a.product_id == product_id)
                .map(|a| a.env_id.clone());
            HostUsbDevice {
                bus,
                device,
                vendor_id,
                product_id,
                description,
                attached_to_env_id: attached_to,
            }
        })
        .collect();

    Ok(Json(result))
}

async fn list_env_devices(
    State(state): State<AppState>,
    user: AuthUser,
    Path(env_id): Path<String>,
) -> Result<Json<Vec<UsbAttachment>>, AppError> {
    let env = super::fetch_env(&state, &env_id).await?;
    super::check_env_owner(&user.0, &env)?;

    let attachments =
        sqlx::query_as::<_, UsbAttachment>("SELECT * FROM usb_attachments WHERE env_id = ?")
            .bind(&env_id)
            .fetch_all(&state.db)
            .await?;

    Ok(Json(attachments))
}

async fn attach_device(
    State(state): State<AppState>,
    user: AuthUser,
    Path(env_id): Path<String>,
    Json(req): Json<AttachUsbRequest>,
) -> Result<(StatusCode, Json<UsbAttachment>), AppError> {
    let uri = require_libvirt_uri(&state)?;
    let env = super::fetch_env(&state, &env_id).await?;
    super::check_env_owner(&user.0, &env)?;

    if env.state.as_deref() != Some("running") {
        return Err(AppError::BadRequest(
            "USB attach requires a running environment".into(),
        ));
    }

    validate_hex4(&req.vendor_id, "vendor_id")?;
    validate_hex4(&req.product_id, "product_id")?;

    // Check device not already attached to another VM
    let existing = sqlx::query_as::<_, UsbAttachment>(
        "SELECT * FROM usb_attachments WHERE vendor_id = ? AND product_id = ?",
    )
    .bind(&req.vendor_id)
    .bind(&req.product_id)
    .fetch_optional(&state.db)
    .await?;

    if let Some(a) = existing {
        return Err(AppError::BadRequest(format!(
            "device {}:{} is already attached to environment {}",
            req.vendor_id, req.product_id, a.env_id
        )));
    }

    // Get device description from lsusb
    let description = libvirt::list_host_usb_devices()
        .await
        .ok()
        .and_then(|devs| {
            devs.into_iter()
                .find(|(_, _, vid, pid, _)| vid == &req.vendor_id && pid == &req.product_id)
                .map(|(_, _, _, _, desc)| desc)
        });

    if description.is_none() {
        return Err(AppError::NotFound(format!(
            "USB device {}:{} not found on host",
            req.vendor_id, req.product_id
        )));
    }

    let domain = libvirt::env_domain_name(&env_id);
    libvirt::attach_usb(&uri, &domain, &req.vendor_id, &req.product_id)
        .await
        .map_err(|e| AppError::Internal(format!("virsh attach-device failed: {e}")))?;

    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::unix_now();

    sqlx::query(
        "INSERT INTO usb_attachments (id, env_id, vendor_id, product_id, description, attached_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&env_id)
    .bind(&req.vendor_id)
    .bind(&req.product_id)
    .bind(&description)
    .bind(now)
    .execute(&state.db)
    .await?;

    crate::events::emit(
        &state.db,
        &env_id,
        "usb_attached",
        None,
        None,
        Some(&user.0.id),
        Some(&format!("{}:{}", req.vendor_id, req.product_id)),
    )
    .await;

    let attachment = UsbAttachment {
        id,
        env_id,
        vendor_id: req.vendor_id,
        product_id: req.product_id,
        description,
        attached_at: now,
    };

    Ok((StatusCode::CREATED, Json(attachment)))
}

async fn detach_device(
    State(state): State<AppState>,
    user: AuthUser,
    Path((env_id, attachment_id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    let uri = require_libvirt_uri(&state)?;
    let env = super::fetch_env(&state, &env_id).await?;
    super::check_env_owner(&user.0, &env)?;

    let attachment = sqlx::query_as::<_, UsbAttachment>(
        "SELECT * FROM usb_attachments WHERE id = ? AND env_id = ?",
    )
    .bind(&attachment_id)
    .bind(&env_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("USB attachment not found".into()))?;

    // Attempt detach — if VM is still running
    if env.state.as_deref() == Some("running") {
        let domain = libvirt::env_domain_name(&env_id);
        if let Err(e) =
            libvirt::detach_usb(&uri, &domain, &attachment.vendor_id, &attachment.product_id).await
        {
            tracing::warn!(
                env_id,
                vendor_id = %attachment.vendor_id,
                product_id = %attachment.product_id,
                error = %e,
                "virsh detach-device failed, removing DB record anyway"
            );
        }
    }

    sqlx::query("DELETE FROM usb_attachments WHERE id = ?")
        .bind(&attachment_id)
        .execute(&state.db)
        .await?;

    crate::events::emit(
        &state.db,
        &env_id,
        "usb_detached",
        None,
        None,
        Some(&user.0.id),
        Some(&format!("{}:{}", attachment.vendor_id, attachment.product_id)),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}
