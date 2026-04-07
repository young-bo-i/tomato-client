pub mod pool;

use sqlx::PgPool;

pub type DbPool = PgPool;
