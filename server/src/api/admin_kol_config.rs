//! KOL 提交词配置 API。
//!
//! 拆成两组端点 (设计见 migration 002 注释):
//!
//!   * **管理员端 — 默认值** (admin only):
//!       GET  /api/admin/kol_config_defaults — 列出所有 (platform, alias_type) 的默认开关 + 限额
//!       PUT  /api/admin/kol_config_defaults — 批量更新默认值
//!     管理员改这里**不会回填**已存在的 profile,只在新建 tomato/qimao
//!     profile 时被读一次作为初始值。
//!
//!   * **用户端 — 自己 profile 的具体配置** (任何登录用户):
//!       GET  /api/users/me/kol_config — 列出我的 tomato/qimao profile 当前配置
//!       PUT  /api/users/me/kol_config — 批量更新我的 profile 配置 (服务端校验所有权)
//!
//! 老的 GET/PUT /api/admin/kol_config (能改任意 profile 的端点) 已废弃 — 现在
//! 只有 profile 的所有者能修改自己的;管理员修改自己的也走用户端。

use actix_web::{web, HttpResponse};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::{AdminUser, AuthUser};
use crate::db::DbPool;
use crate::errors::{AppError, AppResult};

// ──────────────────────────── 默认值 (admin only) ────────────────────────────

/// 默认值表的一行。`updated_at` 让前端能显示「上次修改时间」。
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct DefaultRow {
    pub platform: String,
    pub alias_type: i32,
    pub enabled: bool,
    pub daily_limit: i32,
    pub updated_at: DateTime<Local>,
}

/// PUT 时前端传入的单条默认值。
#[derive(Debug, Deserialize)]
pub struct DefaultUpdate {
    pub platform: String,
    pub alias_type: i32,
    pub enabled: bool,
    pub daily_limit: i32,
}

/// `GET /api/admin/kol_config_defaults` — 列默认值。
pub async fn list_defaults(
    pool: web::Data<DbPool>,
    _: AdminUser,
) -> AppResult<HttpResponse> {
    let rows: Vec<DefaultRow> = sqlx::query_as(
        "SELECT platform, alias_type, enabled, daily_limit, updated_at
         FROM kol_submission_config_defaults
         ORDER BY platform, alias_type",
    )
    .fetch_all(pool.get_ref())
    .await?;
    Ok(HttpResponse::Ok().json(rows))
}

/// `PUT /api/admin/kol_config_defaults` — 批量 upsert 默认值。
pub async fn update_defaults(
    pool: web::Data<DbPool>,
    _: AdminUser,
    body: web::Json<Vec<DefaultUpdate>>,
) -> AppResult<HttpResponse> {
    let items = body.into_inner();
    if items.is_empty() {
        return Ok(HttpResponse::Ok().json(serde_json::json!({ "updated": 0 })));
    }

    let mut updated = 0u64;
    for item in &items {
        if item.daily_limit < 0 {
            return Err(AppError::BadRequest(
                "daily_limit must be >= 0".into(),
            ));
        }
        if !matches!(item.platform.as_str(), "tomato" | "qimao") {
            return Err(AppError::BadRequest(format!(
                "platform must be tomato/qimao, got {}",
                item.platform
            )));
        }
        let r = sqlx::query(
            r#"INSERT INTO kol_submission_config_defaults
                   (platform, alias_type, enabled, daily_limit, updated_at)
               VALUES ($1, $2, $3, $4, NOW())
               ON CONFLICT (platform, alias_type)
               DO UPDATE SET enabled = EXCLUDED.enabled,
                             daily_limit = EXCLUDED.daily_limit,
                             updated_at = NOW()"#,
        )
        .bind(&item.platform)
        .bind(item.alias_type)
        .bind(item.enabled)
        .bind(item.daily_limit)
        .execute(pool.get_ref())
        .await?;
        updated += r.rows_affected();
    }

    // No cache invalidation — defaults table is only read at profile
    // creation. Existing profile configs aren't affected.
    Ok(HttpResponse::Ok().json(serde_json::json!({ "updated": updated })))
}

// ──────────────────────────── 单 profile (任意已登录用户管自己) ────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct KolConfigRow {
    pub profile_id: Uuid,
    pub platform: String,
    pub alias_type: i32,
    pub enabled: bool,
    pub daily_limit: i32,
    pub updated_at: DateTime<Local>,
}

#[derive(Debug, Deserialize)]
pub struct ConfigUpdate {
    pub profile_id: Uuid,
    pub platform: String,
    pub alias_type: i32,
    pub enabled: bool,
    pub daily_limit: i32,
}

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

/// `GET /api/users/me/kol_config` — 列出**调用者自己**的 tomato/qimao
/// profile 配置。Admin 调这个端点也只看到自己的 profile (admin 想看
/// 别人的请用 admin defaults 或将来的支持工具)。
pub async fn list_mine(
    pool: web::Data<DbPool>,
    user: AuthUser,
) -> AppResult<HttpResponse> {
    list_for_user(pool.get_ref(), user.0.sub).await
}

async fn list_for_user(pool: &DbPool, user_id: i32) -> AppResult<HttpResponse> {
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
           WHERE bp.user_id = $1
             AND bp.kol_platform IN ('tomato', 'qimao')
           ORDER BY bp.kol_platform, bp.name"#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    if profiles.is_empty() {
        return Ok(HttpResponse::Ok().json(Vec::<ProfileConfig>::new()));
    }

    let profile_ids: Vec<Uuid> = profiles.iter().map(|p| p.profile_id).collect();
    let configs: Vec<KolConfigRow> = sqlx::query_as(
        "SELECT profile_id, platform, alias_type, enabled, daily_limit, updated_at
         FROM kol_submission_config
         WHERE profile_id = ANY($1::uuid[])
         ORDER BY profile_id, platform, alias_type",
    )
    .bind(&profile_ids)
    .fetch_all(pool)
    .await?;

    let result: Vec<ProfileConfig> = profiles
        .into_iter()
        .map(|p| {
            let cfgs: Vec<KolConfigRow> = configs
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
                configs: cfgs,
            }
        })
        .collect();
    Ok(HttpResponse::Ok().json(result))
}

/// `PUT /api/users/me/kol_config` — 批量 upsert 调用者自己 profile 的配置。
/// 服务端在写入前用 EXISTS 校验每个 profile_id 的所有权。任意一条
/// 不属于调用者就整体 403,避免静默部分成功。
pub async fn update_mine(
    pool: web::Data<DbPool>,
    user: AuthUser,
    body: web::Json<Vec<ConfigUpdate>>,
) -> AppResult<HttpResponse> {
    let items = body.into_inner();
    if items.is_empty() {
        return Ok(HttpResponse::Ok().json(serde_json::json!({ "updated": 0 })));
    }

    // Ownership gate: every profile_id in the batch must belong to caller.
    // Single round-trip via the `= ANY(...)` filter.
    let target_ids: Vec<Uuid> = items.iter().map(|i| i.profile_id).collect();
    let owned_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM browser_profiles
         WHERE id = ANY($1::uuid[]) AND user_id = $2",
    )
    .bind(&target_ids)
    .bind(user.0.sub)
    .fetch_one(pool.get_ref())
    .await?;

    // De-dup ids before counting expected
    let unique_ids: std::collections::BTreeSet<Uuid> = target_ids.iter().copied().collect();
    if (owned_count as usize) < unique_ids.len() {
        return Err(AppError::Forbidden);
    }

    let mut updated = 0u64;
    for item in &items {
        if item.daily_limit < 0 {
            return Err(AppError::BadRequest("daily_limit must be >= 0".into()));
        }
        if !matches!(item.platform.as_str(), "tomato" | "qimao") {
            return Err(AppError::BadRequest(format!(
                "platform must be tomato/qimao, got {}",
                item.platform
            )));
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

    crate::services::cache::invalidate_submission_config();
    Ok(HttpResponse::Ok().json(serde_json::json!({ "updated": updated })))
}
