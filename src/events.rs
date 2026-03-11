use sqlx::SqlitePool;

pub async fn emit(
    db: &SqlitePool,
    env_id: &str,
    event_type: &str,
    from_state: Option<&str>,
    to_state: Option<&str>,
    triggered_by: Option<&str>,
    message: Option<&str>,
) {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::unix_now();

    let result = sqlx::query(
        "INSERT INTO environment_events (id, env_id, event_type, from_state, to_state, triggered_by, message, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(env_id)
    .bind(event_type)
    .bind(from_state)
    .bind(to_state)
    .bind(triggered_by)
    .bind(message)
    .bind(now)
    .execute(db)
    .await;

    if let Err(e) = result {
        tracing::warn!(env_id = %env_id, error = %e, "failed to emit environment event");
    }
}
