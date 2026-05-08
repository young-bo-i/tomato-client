use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub database_max_connections: u32,
    pub jwt_secret: String,
    pub jwt_expiration_hours: i64,
    pub admin_username: String,
    pub admin_password: String,
    /// URL of the bundled abogus signing service. Provided by
    /// docker-compose as `http://abogus:3000/api/get-a-bogus`.
    pub abogus_url: String,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Ok(Self {
            host: env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            port: env::var("PORT")
                .unwrap_or_else(|_| "8099".into())
                .parse()
                .map_err(|e| format!("PORT: {e}"))?,
            database_url: env::var("DATABASE_URL")
                .map_err(|_| "DATABASE_URL is required".to_string())?,
            // 6 background workers + bulk_create handler + spawned
            // enqueue tasks routinely use 12–18 connections in steady
            // state. 50 leaves comfortable headroom for the multi-client
            // scrape scenario (2× clients × 50 profiles each) without
            // straining Postgres (~10 MB/connection).
            database_max_connections: env::var("DATABASE_MAX_CONNECTIONS")
                .unwrap_or_else(|_| "50".into())
                .parse()
                .map_err(|e| format!("DATABASE_MAX_CONNECTIONS: {e}"))?,
            jwt_secret: env::var("JWT_SECRET")
                .map_err(|_| "JWT_SECRET is required".to_string())?,
            jwt_expiration_hours: env::var("JWT_EXPIRATION_HOURS")
                .unwrap_or_else(|_| "8760".into())
                .parse()
                .map_err(|e| format!("JWT_EXPIRATION_HOURS: {e}"))?,
            admin_username: env::var("ADMIN_USERNAME").unwrap_or_else(|_| "admin".into()),
            admin_password: env::var("ADMIN_PASSWORD").unwrap_or_else(|_| "admin123".into()),
            abogus_url: env::var("ABOGUS_URL")
                .unwrap_or_else(|_| "http://abogus:3000/api/get-a-bogus".into()),
        })
    }
}
