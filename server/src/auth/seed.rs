use crate::auth::password;
use crate::db::DbPool;

/// Idempotently seed the initial admin. On conflict (username already
/// exists) we leave the row alone — protects any password change the
/// admin made via UI. Editing `ADMIN_PASSWORD` in `.env` after the first
/// run has no effect unless that row is manually removed.
pub async fn run(pool: &DbPool, username: &str, password: &str) -> Result<(), String> {
    if username.is_empty() || password.is_empty() {
        return Err("ADMIN_USERNAME / ADMIN_PASSWORD must be non-empty".into());
    }

    let hash = password::hash(password)?;

    let res = sqlx::query(
        r#"INSERT INTO users (username, password_hash, role, is_active)
           VALUES ($1, $2, 'admin', TRUE)
           ON CONFLICT (username) DO NOTHING"#,
    )
    .bind(username)
    .bind(&hash)
    .execute(pool)
    .await
    .map_err(|e| format!("seed admin: {e}"))?;

    if res.rows_affected() == 1 {
        tracing::info!("seeded initial admin user '{username}'");
    } else {
        tracing::debug!("admin user '{username}' already exists, not touching");
    }
    Ok(())
}
