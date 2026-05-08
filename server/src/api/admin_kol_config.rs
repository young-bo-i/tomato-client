//! 管理员接口：KOL 提交词配置。
//!
//! GET  /api/admin/kol_config          — 所有 profile 的配置列表
//! PUT  /api/admin/kol_config          — 批量更新配置（upsert）
//!
//! 配置颗粒度：(profile_id, platform, alias_type)
//! 每个 profile 的每个平台/类型组合有独立的开关和日限额。

use actix_web::{web, HttpResponse};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::AdminUser;
use crate::db::DbPool;
use crate::errors::AppResult;

/// 一条配置行
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct KolConfigRow {
    pub profile_id: Uuid,
    pub platform: String,
    pub alias_type: i32,
    pub enabled: bool,
    pub daily_limit: i32,
    pub updated_at: DateTime<Local>,
}

/// 前端传入的单条更新
#[derive(Debug, Deserialize)]
pub struct ConfigUpdate {
    pub profile_id: Uuid,
    pub platform: String,
    pub alias_type: i32,
    pub enabled: bool,
    pub daily_limit: i32,
}

/// Profile 信息 + 其所有配置（聚合后返回给前端）
#[derive(Debug, Serialize)]
pub struct ProfileConfig {
    pub profile_id: Uuid,
    pub profile_name: String,
    pub kol_platform: String,
    pub user_id: i32,
    pub username: String,
    pub is_admin: bool,
    pub configs: Vec<KolConfigRow>,
}

/// `GET /api/admin/kol_config` — 返回所有 kol_platform 账号的配置聚合。
/// 只返回有 kol_platform 字段（tomato/qimao）的 profile，其他忽略。
pub async fn list(
    pool: web::Data<DbPool>,
    _: AdminUser,
) -> AppResult<HttpResponse> {
    #[derive(sqlx::FromRow)]
    struct ProfileRow {
        profile_id: Uuid,
        profile_name: String,
        kol_platform: String,
        user_id: i32,
        username: String,
        is_admin: bool,
    }
    let profiles: Vec<ProfileRow> = sqlx::query_as(
        r#"SELECT bp.id AS profile_id, bp.name AS profile_name,
                  COALESCE(bp.kol_platform, '') AS kol_platform,
                  bp.user_id, u.username,
                  (u.role = 'admin') AS is_admin
           FROM browser_profiles bp
           JOIN users u ON u.id = bp.user_id
           WHERE bp.kol_platform IN ('tomato', 'qimao')
           ORDER BY u.role DESC, u.id, bp.kol_platform, bp.name"#,
    )
    .fetch_all(pool.get_ref())
    .await?;

    // 现有配置行
    let existing: Vec<KolConfigRow> = sqlx::query_as(
        "SELECT profile_id, platform, alias_type, enabled, daily_limit, updated_at
         FROM kol_submission_config
         ORDER BY profile_id, platform, alias_type",
    )
    .fetch_all(pool.get_ref())
    .await?;

    let result: Vec<ProfileConfig> = profiles
        .into_iter()
        .map(|p| {
            let configs: Vec<KolConfigRow> = existing
                .iter()
                .filter(|c| c.profile_id == p.profile_id)
                .cloned()
                .collect();
            ProfileConfig {
                profile_id: p.profile_id,
                profile_name: p.profile_name,
                kol_platform: p.kol_platform,
                user_id: p.user_id,
                username: p.username,
                is_admin: p.is_admin,
                configs,
            }
        })
        .collect();

    Ok(HttpResponse::Ok().json(result))
}

/// `PUT /api/admin/kol_config` — 批量 upsert。每次传入完整的变更列表。
pub async fn update(
    pool: web::Data<DbPool>,
    _: AdminUser,
    body: web::Json<Vec<ConfigUpdate>>,
) -> AppResult<HttpResponse> {
    let items = body.into_inner();
    if items.is_empty() {
        return Ok(HttpResponse::Ok().json(serde_json::json!({ "updated": 0 })));
    }

    let mut updated = 0u64;
    for item in &items {
        if item.daily_limit < 0 {
            continue;
        }
        let r = sqlx::query(
            r#"INSERT INTO kol_submission_config
                   (profile_id, platform, alias_type, enabled, daily_limit, updated_at)
               VALUES ($1, $2, $3, $4, $5, NOW())
               ON CONFLICT (profile_id, platform, alias_type)
               DO UPDATE SET enabled = EXCLUDED.enabled,
                             daily_limit = EXCLUDED.daily_limit,
                             updated_at = NOW()"#,
        )
        .bind(item.profile_id)
        .bind(&item.platform)
        .bind(item.alias_type)
        .bind(item.enabled)
        .bind(item.daily_limit)
        .execute(pool.get_ref())
        .await?;
        updated += r.rows_affected();
    }

    // Invalidate submission config cache so next enqueue sees fresh values.
    crate::services::cache::invalidate_submission_config();
    Ok(HttpResponse::Ok().json(serde_json::json!({ "updated": updated })))
}
