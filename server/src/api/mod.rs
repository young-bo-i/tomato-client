pub mod auth;
pub mod account;
pub mod kol;
pub mod douyin;
pub mod submit;
pub mod task;
pub mod setting;
pub mod profile;

use actix_web::web;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api/v1")
            // Public routes
            .service(
                web::scope("/auth")
                    .route("/login", web::post().to(auth::login))
                    .route("/test", web::get().to(auth::health_check))
                    .route("/version", web::get().to(auth::get_version))
            )
            // Protected routes
            .service(
                web::scope("/account")
                    .route("", web::get().to(account::get_account_info))
                    .route("/create", web::post().to(account::create_sub_account))
                    .route("/subs", web::get().to(account::get_all_sub_accounts))
                    .route("/{id}/renew", web::post().to(account::renew_account))
                    .route("/{id}/disable", web::post().to(account::disable_account))
                    .route("/{id}/enable", web::post().to(account::enable_account))
            )
            .service(
                web::scope("/kol")
                    .route("/cookies", web::post().to(kol::submit_cookies))
                    .route("/cookies", web::put().to(kol::update_cookies))
                    .route("/list", web::get().to(kol::get_kol_accounts))
                    .route("/base", web::get().to(kol::get_kol_base_infos))
                    .route("/{id}", web::get().to(kol::get_kol_by_id))
                    .route("/{id}", web::delete().to(kol::delete_kol_account))
                    .route("/{id}/remark", web::put().to(kol::update_remark))
                    .route("/invitecodes", web::get().to(kol::get_invite_codes))
            )
            .service(
                web::scope("/douyin")
                    .route("/storage", web::post().to(douyin::submit_storage_state))
                    .route("/storage", web::put().to(douyin::update_storage_state))
                    .route("/list", web::get().to(douyin::get_accounts))
                    .route("/base", web::get().to(douyin::get_base_accounts))
                    .route("/{id}", web::get().to(douyin::get_by_id))
                    .route("/{id}", web::delete().to(douyin::delete_account))
                    .route("/{id}/status", web::put().to(douyin::set_status))
                    .route("/{id}/remark", web::put().to(douyin::update_remark))
            )
            .service(
                web::scope("/submit")
                    .route("/brush", web::post().to(submit::submit_brush_task))
                    .route("/frequency", web::get().to(submit::get_request_frequency))
            )
            .service(
                web::scope("/task")
                    .route("/grid", web::post().to(task::get_task_data_grid))
                    .route("/summary", web::get().to(task::get_task_summary))
                    .route("/recent", web::get().to(task::get_recent_tasks))
                    .route("/income", web::get().to(task::get_recent_income))
                    .route("/books", web::get().to(task::get_books))
            )
            .service(
                web::scope("/setting")
                    .route("/all", web::get().to(setting::get_all_settings))
                    .route("/platform", web::post().to(setting::save_platform_types))
                    .route("/limit", web::post().to(setting::save_type_limit))
                    .route("/dom/{dom_type}", web::get().to(setting::get_dom_config))
                    .route("/dom", web::post().to(setting::update_dom_config))
                    .route("/notice", web::get().to(setting::get_income_notice))
                    .route("/notice", web::post().to(setting::set_income_notice))
                    .route("/notice/email", web::post().to(setting::add_notice_email))
                    .route("/notice/child", web::put().to(setting::set_notice_has_child))
                    .route("/authorize/limit", web::get().to(setting::get_third_party_limit))
            )
            .service(
                web::scope("/profile")
                    .route("", web::post().to(profile::create_profile))
                    .route("", web::get().to(profile::list_profiles))
                    .route("/{id}", web::get().to(profile::get_profile))
                    .route("/{id}", web::put().to(profile::update_profile))
                    .route("/{id}", web::delete().to(profile::delete_profile))
                    .route("/{id}/sync/upload", web::post().to(profile::sync_upload))
                    .route("/{id}/sync/download", web::get().to(profile::sync_download))
                    .route("/{id}/sync/status", web::get().to(profile::sync_status))
            )
    );
}
