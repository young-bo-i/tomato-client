use sqlx::postgres::PgPoolOptions;
use sqlx::migrate::Migrator;
use std::path::Path;
use crate::config::AppConfig;
use crate::db::DbPool;

pub async fn create_pool(config: &AppConfig) -> anyhow::Result<DbPool> {
    let pool = PgPoolOptions::new()
        .max_connections(config.database_max_connections)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .idle_timeout(std::time::Duration::from_secs(300))
        .connect(&config.database_url)
        .await?;

    let migrator = Migrator::new(Path::new("./src/db/migrations")).await?;
    migrator.run(&pool).await?;

    tracing::info!("Database connected, migrations applied");
    Ok(pool)
}
