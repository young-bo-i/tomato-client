mod api;
mod config;
mod db;
mod errors;
mod middleware;
mod models;
mod queue;
mod scheduler;
mod services;

use actix_cors::Cors;
use actix_web::{web, App, HttpServer};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("tomato_server=info,actix_web=info")),
        )
        .json()
        .init();

    tracing::info!("Starting Tomato KOL Server v{}", env!("CARGO_PKG_VERSION"));

    // Load config
    let config = config::AppConfig::from_env()?;
    let bind_addr = format!("{}:{}", config.host, config.port);

    // Initialize database pool
    let pool = db::pool::create_pool(&config).await?;
    tracing::info!("Database pool initialized");

    // Initialize Redis
    let redis = redis::Client::open(config.redis_url.as_str())?;
    tracing::info!("Redis client initialized");

    // JWT config
    let jwt_config = middleware::auth::JwtConfig {
        secret: config.jwt_secret.clone(),
        expiration_hours: config.jwt_expiration_hours,
    };

    // Start background consumers (Redis Stream workers)
    let consumer_count = num_cpus::get().max(4);
    queue::start_consumers(pool.clone(), redis.clone(), consumer_count).await;

    // Start scheduler
    scheduler::start_scheduler(pool.clone(), redis.clone()).await?;

    // Start HTTP server
    tracing::info!("HTTP server listening on {}", bind_addr);

    HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header()
            .max_age(3600);

        App::new()
            .wrap(cors)
            .wrap(middleware::logging::request_logger())
            .wrap(tracing_actix_web::TracingLogger::default())
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(redis.clone()))
            .app_data(web::Data::new(jwt_config.clone()))
            .app_data(web::JsonConfig::default().limit(10 * 1024 * 1024)) // 10MB
            .configure(api::configure)
    })
    .workers(num_cpus::get() * 2)
    .backlog(10240)
    .max_connections(100_000)
    .bind(&bind_addr)?
    .run()
    .await?;

    Ok(())
}
