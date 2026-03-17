use std::sync::Arc;
use std::time::Duration;

use sqlx::SqlitePool;
use tokio::sync::watch;

use crate::api::environments::is_agent_managed;
use crate::events;
use crate::vm::{VmProvider, provider_id_for};

pub fn spawn_reaper(
    db: SqlitePool,
    vm_provider: Arc<dyn VmProvider>,
    mut shutdown: watch::Receiver<bool>,
    interval_secs: u64,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(interval_secs)) => {
                    if let Err(e) = reap_expired(&db, &vm_provider).await {
                        tracing::error!(error = %e, "environment reaper error");
                    }
                }
                _ = shutdown.changed() => {
                    tracing::info!("reaper shutting down");
                    break;
                }
            }
        }
    });
}

async fn reap_expired(
    db: &SqlitePool,
    vm_provider: &Arc<dyn VmProvider>,
) -> Result<(), sqlx::Error> {
    let now = crate::unix_now();

    let expired: Vec<crate::models::Environment> = sqlx::query_as(
        "SELECT * FROM environments WHERE expires_at IS NOT NULL AND expires_at <= ? AND state NOT IN ('destroying', 'failed')",
    )
    .bind(now)
    .fetch_all(db)
    .await?;

    for env in expired {
        tracing::info!(env_id = %env.id, "reaping expired environment");
        let old_state = env.state.clone();

        let _ = sqlx::query("UPDATE environments SET state = 'destroying' WHERE id = ?")
            .bind(&env.id)
            .execute(db)
            .await;

        events::emit(
            db,
            &env.id,
            "expired",
            old_state.as_deref(),
            Some("destroying"),
            Some("system"),
            Some("TTL expired"),
        )
        .await;

        let provider = env.provider.as_deref().unwrap_or("libvirt");
        if !is_agent_managed(provider) {
            let provider_id = provider_id_for(provider, &env.id);
            let vm = vm_provider.clone();
            let db = db.clone();
            let env_id = env.id.clone();
            tokio::spawn(async move {
                match vm.destroy_vm(&provider_id).await {
                    Ok(()) => {
                        crate::delete_environment_cascade(&db, &env_id).await;
                        tracing::info!(env_id = %env_id, "expired environment destroyed");
                    }
                    Err(e) => {
                        let _ = sqlx::query(
                            "UPDATE environments SET state = 'failed' WHERE id = ?",
                        )
                        .bind(&env_id)
                        .execute(&db)
                        .await;
                        events::emit(
                            &db,
                            &env_id,
                            "state_change",
                            Some("destroying"),
                            Some("failed"),
                            Some("system"),
                            Some(&e),
                        )
                        .await;
                        tracing::error!(env_id = %env_id, error = %e, "failed to destroy expired environment");
                    }
                }
            });
        }
    }
    // Clean up expired WebAuthn challenges
    sqlx::query("DELETE FROM webauthn_challenges WHERE expires_at < ?")
        .bind(now)
        .execute(db)
        .await
        .ok();

    Ok(())
}

pub fn spawn_heartbeat_checker(
    db: SqlitePool,
    mut shutdown: watch::Receiver<bool>,
    interval_secs: u64,
    stale_threshold_secs: i64,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(interval_secs)) => {
                    if let Err(e) = check_heartbeats(&db, stale_threshold_secs).await {
                        tracing::error!(error = %e, "heartbeat checker error");
                    }
                }
                _ = shutdown.changed() => {
                    tracing::info!("heartbeat checker shutting down");
                    break;
                }
            }
        }
    });
}

async fn check_heartbeats(db: &SqlitePool, stale_threshold_secs: i64) -> Result<(), sqlx::Error> {
    let now = crate::unix_now();

    let stale_threshold = now - stale_threshold_secs;

    let result = sqlx::query(
        "UPDATE nodes SET online = 0 WHERE online = 1 AND last_heartbeat_at IS NOT NULL AND last_heartbeat_at < ?",
    )
    .bind(stale_threshold)
    .execute(db)
    .await?;

    if result.rows_affected() > 0 {
        tracing::warn!(
            count = result.rows_affected(),
            "marked stale nodes as offline"
        );
    }

    Ok(())
}

/// On startup, reconcile environments stuck in transient states from a previous crash.
/// - `destroying`: finish the destroy (best-effort VM cleanup + delete DB row)
/// - Other transient states: mark as failed
pub async fn reconcile_stuck_environments(db: &SqlitePool, vm_provider: &Arc<dyn VmProvider>) {
    let stuck: Vec<crate::models::Environment> = sqlx::query_as(
        "SELECT * FROM environments WHERE state IN ('creating', 'suspending', 'resuming', 'rebooting', 'migrating', 'destroying')",
    )
    .fetch_all(db)
    .await
    .unwrap_or_default();

    let mut count = 0u64;
    for env in &stuck {
        let provider = env.provider.as_deref().unwrap_or("libvirt");
        if is_agent_managed(provider) {
            continue;
        }

        let state = env.state.as_deref().unwrap_or("");

        if state == "destroying" {
            // The destroy was interrupted — finish it.
            // Best-effort VM cleanup (may already be gone).
            let provider_id = provider_id_for(provider, &env.id);
            if let Err(e) = vm_provider.destroy_vm(&provider_id).await {
                tracing::debug!(env_id = %env.id, error = %e, "VM already gone or destroy failed during reconciliation");
            }
            crate::delete_environment_cascade(db, &env.id).await;
            events::emit(
                db,
                &env.id,
                "state_change",
                Some("destroying"),
                None,
                Some("system"),
                Some("completed interrupted destroy on server restart"),
            )
            .await;
            tracing::info!(env_id = %env.id, "completed interrupted destroy on startup");
        } else {
            tracing::warn!(
                env_id = %env.id,
                state = %state,
                "marking stuck environment as failed (server restart recovery)"
            );
            let _ = sqlx::query("UPDATE environments SET state = 'failed' WHERE id = ?")
                .bind(&env.id)
                .execute(db)
                .await;
            events::emit(
                db,
                &env.id,
                "state_change",
                Some(state),
                Some("failed"),
                Some("system"),
                Some("stuck in transient state after server restart"),
            )
            .await;
        }
        count += 1;
    }

    if count > 0 {
        tracing::info!(count, "reconciled stuck environments on startup");
    }
}
