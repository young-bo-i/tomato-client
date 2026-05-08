pub mod admin_api_log;
pub mod admin_jobs;
pub mod admin_kol_config;
pub mod admin_settings;
pub mod admin_users;
pub mod auth;
pub mod douyin_videos;
pub mod email_settings;
pub mod income;
pub mod profile_state;
pub mod profiles;
pub mod qimao;
pub mod qimao_notice;
pub mod qimao_stats;
pub mod tomato;
pub mod tomato_stats;
pub mod users_self;

use actix_web::web;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api")
            .service(
                web::scope("/auth")
                    .route("/login", web::post().to(auth::login))
                    .route("/me", web::get().to(auth::me)),
            )
            .service(
                // User self-edit endpoints. Distinct scope from
                // /admin/users so it's clear who's allowed to call
                // what (any auth user vs admin-only).
                web::scope("/users/me")
                    .route(
                        "/tier2_contribution",
                        web::put().to(users_self::update_my_tier2_contribution),
                    )
                    .route(
                        "/subordinates",
                        web::get().to(users_self::list_my_subordinates),
                    )
                    .route(
                        "/password",
                        web::put().to(users_self::change_my_password),
                    )
                    .route(
                        "/kol_config",
                        web::get().to(admin_kol_config::list_mine),
                    )
                    .route(
                        "/kol_config",
                        web::put().to(admin_kol_config::update_mine),
                    )
                    // 番茄收益 & 七猫收益通知 — caller-scoped (each user
                    // sees only their own profiles' rows; admins use
                    // the [管理员速览] email digest for全员视角).
                    .route("/income", web::get().to(income::list))
                    .route("/income/overview", web::get().to(income::overview))
                    .route("/qimao_notices", web::get().to(qimao_notice::list)),
            )
            .service(
                web::scope("/admin/users")
                    .route("", web::get().to(admin_users::list))
                    .route("", web::post().to(admin_users::create))
                    .route("/{id}", web::patch().to(admin_users::update))
                    .route("/{id}", web::delete().to(admin_users::delete)),
            )
            .service(
                web::scope("/admin/email_settings")
                    .route("", web::get().to(email_settings::get))
                    .route("", web::put().to(email_settings::put))
                    .route("/test", web::post().to(email_settings::send_test)),
            )
            .service(
                web::scope("/admin/settings")
                    .route("", web::get().to(admin_settings::get))
                    .route("", web::put().to(admin_settings::put)),
            )
            .service(
                web::scope("/admin/jobs")
                    .route("", web::get().to(admin_jobs::list))
                    .route("/{name}/history", web::get().to(admin_jobs::history)),
            )
            .service(
                // Admin-only defaults editor. Used as initial values
                // when creating new tomato/qimao profiles; doesn't
                // touch existing per-profile rows.
                web::scope("/admin/kol_config_defaults")
                    .route("", web::get().to(admin_kol_config::list_defaults))
                    .route("", web::put().to(admin_kol_config::update_defaults)),
            )
            .service(
                web::scope("/admin/api_log")
                    .route("", web::get().to(admin_api_log::list))
                    .route("", web::delete().to(admin_api_log::delete))
                    .route("/mark", web::post().to(admin_api_log::mark))
                    .route("/export", web::get().to(admin_api_log::export)),
            )
            .service(
                web::scope("/profiles")
                    .route("", web::get().to(profiles::list))
                    .route("", web::post().to(profiles::create))
                    .route("/{id}", web::get().to(profiles::get))
                    .route("/{id}", web::patch().to(profiles::update))
                    .route("/{id}", web::delete().to(profiles::delete))
                    .route("/{id}/state", web::get().to(profile_state::get))
                    .route("/{id}/state", web::put().to(profile_state::put))
                    .route(
                        "/{id}/qimao_refresh_token",
                        web::post().to(qimao::refresh_token),
                    )
                    .route(
                        "/{id}/douyin_state",
                        web::post().to(profiles::set_douyin_state),
                    ),
            )
            .service(
                web::scope("/tomato/books")
                    .route("", web::get().to(tomato::list))
                    .route("/refresh", web::post().to(tomato::refresh)),
            )
            .service(
                web::scope("/qimao/books")
                    .route("", web::get().to(qimao::list))
                    .route("/refresh", web::post().to(qimao::refresh)),
            )
            .service(
                web::scope("/qimao/stats")
                    .route("/overview", web::get().to(qimao_stats::overview))
                    .route("/accounts", web::get().to(qimao_stats::accounts)),
            )
            .service(
                web::scope("/tomato/stats")
                    .route("/overview", web::get().to(tomato_stats::overview))
                    .route("/accounts", web::get().to(tomato_stats::accounts)),
            )
            .service(
                web::scope("/douyin/videos")
                    .route("", web::get().to(douyin_videos::list))
                    .route("/bulk", web::post().to(douyin_videos::bulk_create)),
            ),
    );
}
