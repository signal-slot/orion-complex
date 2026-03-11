use sqlx::SqlitePool;

pub async fn create_task(db: &SqlitePool, kind: &str) -> Result<String, sqlx::Error> {
    create_task_for_env(db, kind, None).await
}

pub async fn create_task_for_env(
    db: &SqlitePool,
    kind: &str,
    env_id: Option<&str>,
) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::unix_now();

    sqlx::query(
        "INSERT INTO tasks (id, kind, state, created_at, env_id) VALUES (?, ?, 'pending', ?, ?)",
    )
    .bind(&id)
    .bind(kind)
    .bind(now)
    .bind(env_id)
    .execute(db)
    .await?;

    Ok(id)
}

pub async fn update_task_state(
    db: &SqlitePool,
    task_id: &str,
    state: &str,
) -> Result<(), sqlx::Error> {
    let completed_at = if state == "completed" || state == "failed" {
        Some(crate::unix_now())
    } else {
        None
    };

    sqlx::query("UPDATE tasks SET state = ?, completed_at = COALESCE(?, completed_at) WHERE id = ?")
        .bind(state)
        .bind(completed_at)
        .bind(task_id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn fail_task(
    db: &SqlitePool,
    task_id: &str,
    error_message: &str,
) -> Result<(), sqlx::Error> {
    let now = crate::unix_now();

    sqlx::query(
        "UPDATE tasks SET state = 'failed', error_message = ?, completed_at = ? WHERE id = ?",
    )
    .bind(error_message)
    .bind(now)
    .bind(task_id)
    .execute(db)
    .await?;
    Ok(())
}
