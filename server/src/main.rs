use actix_cors::Cors;
use actix_web::{middleware::Logger, web, App, HttpResponse, HttpServer};
use tracing_subscriber::fmt::time::ChronoLocal;
use tracing_subscriber::EnvFilter;

mod api;
mod auth;
mod config;
mod db;
mod errors;
mod jobs;
mod models;
mod services;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenvy::dotenv().ok();
    // Logs use the host timezone (TZ env var → Asia/Shanghai in
    // production). Default subscriber uses UTC which made on-call
    // grepping painful.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_timer(ChronoLocal::new("%Y-%m-%dT%H:%M:%S%.6f%:z".into()))
        .init();

    let cfg = config::Config::from_env().unwrap_or_else(|e| {
        eprintln!("config error: {e}");
        std::process::exit(1);
    });

    let pool = db::pool::init(&cfg.database_url, cfg.database_max_connections)
        .await
        .unwrap_or_else(|e| {
            eprintln!("db connect: {e}");
            std::process::exit(1);
        });

    // Migrations are embedded into the binary at compile time and
    // tracked in `_sqlx_migrations`. Each file under `src/db/migrations/`
    // runs exactly once; sqlx verifies the checksum on every boot to
    // catch retroactive edits to applied migrations.
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap_or_else(|e| {
            eprintln!("migrations: {e}");
            std::process::exit(1);
        });

    auth::seed::run(&pool, &cfg.admin_username, &cfg.admin_password)
        .await
        .unwrap_or_else(|e| {
            eprintln!("admin seed: {e}");
            std::process::exit(1);
        });

    let jwt = auth::jwt::JwtConfig::new(&cfg.jwt_secret, cfg.jwt_expiration_hours);
    let pool_data = web::Data::new(pool.clone());
    let jwt_data = web::Data::new(jwt);
    let abogus_url_data = web::Data::new(cfg.abogus_url.clone());

    // Cron scheduler + workers — scheduler must outlive HttpServer.
    let _scheduler = jobs::start(pool.clone(), cfg.abogus_url.clone())
        .await
        .unwrap_or_else(|e| {
            eprintln!("scheduler init: {e}");
            std::process::exit(1);
        });

    let host = cfg.host.clone();
    let port = cfg.port;
    tracing::info!("tomato-server listening on {host}:{port}");

    HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header()
            .max_age(3600);

        App::new()
            .wrap(Logger::default())
            .wrap(cors)
            .app_data(pool_data.clone())
            .app_data(jwt_data.clone())
            .app_data(abogus_url_data.clone())
            // 64 MB — profile-state PUT can carry base64'd local_storage
            // tarballs up to ~tens of MB. Regular endpoints still fit.
            .app_data(web::JsonConfig::default().limit(64 << 20))
            .app_data(web::PayloadConfig::new(64 << 20))
            .route("/health", web::get().to(|| async { HttpResponse::Ok().body("ok") }))
            .configure(api::configure)
    })
    .bind((host.as_str(), port))?
    .run()
    .await
}
