use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub database_max_connections: u32,
    pub redis_url: String,
    pub jwt_secret: String,
    pub jwt_expiration_hours: i64,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<String>,
    pub smtp_from: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();

        Ok(Self {
            host: std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            port: std::env::var("PORT")
                .unwrap_or_else(|_| "8099".into())
                .parse()?,
            database_url: std::env::var("DATABASE_URL")?,
            database_max_connections: std::env::var("DATABASE_MAX_CONNECTIONS")
                .unwrap_or_else(|_| "100".into())
                .parse()?,
            redis_url: std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into()),
            jwt_secret: std::env::var("JWT_SECRET")?,
            jwt_expiration_hours: std::env::var("JWT_EXPIRATION_HOURS")
                .unwrap_or_else(|_| "8760".into())
                .parse()?,
            smtp_host: std::env::var("SMTP_HOST").ok(),
            smtp_port: std::env::var("SMTP_PORT").ok().and_then(|v| v.parse().ok()),
            smtp_username: std::env::var("SMTP_USERNAME").ok(),
            smtp_password: std::env::var("SMTP_PASSWORD").ok(),
            smtp_from: std::env::var("SMTP_FROM").ok(),
        })
    }
}
