//! 管理员接口：定时任务执行统计。
//!
//! `GET /api/admin/jobs` 返回每个 job 的聚合统计（总次数、成功次数、
//! 最后执行时间、平均耗时、累计处理量）。
//!
//! `GET /api/admin/jobs/:name/history?limit=50` 返回单个 job 的最近
//! N 条执行记录（用于查看详细执行历史）。

use actix_web::{web, HttpResponse};
use chrono::{DateTime, Local};
use serde::Serialize;
use sqlx::FromRow;

use crate::auth::AdminUser;
use crate::db::DbPool;
use crate::errors::AppResult;

/// 定时任务概览：所有 job 按首次出现的 job_name 聚合。
#[derive(Debug, Serialize, FromRow)]
pub struct JobSummary {
    pub job_name: String,
    pub total_runs: i64,
    pub successful_runs: i64,
    pub failed_runs: i64,
    pub total_items: i64,
    pub avg_duration_ms: Option<f64>,
    pub last_ran_at: Option<DateTime<Local>>,
    pub last_success: Option<bool>,
    pub last_error: Option<String>,
}

/// 单条执行记录。
#[derive(Debug, Serialize, FromRow)]
pub struct JobRun {
    pub id: i64,
    pub job_name: String,
    pub ran_at: DateTime<Local>,
    pub duration_ms: Option<i32>,
    pub items_processed: i32,
    pub success: bool,
    pub error_reason: Option<String>,
}

/// `GET /api/admin/jobs` — 所有 job 的聚合统计，按 job_name 排序。
pub async fn list(pool: web::Data<DbPool>, _: AdminUser) -> AppResult<HttpResponse> {
    let rows = sqlx::query_as::<_, JobSummary>(
        r#"SELECT
               job_name,
               COUNT(*)                                       AS total_runs,
               COUNT(*) FILTER (WHERE success)               AS successful_runs,
               COUNT(*) FILTER (WHERE NOT success)           AS failed_runs,
               COALESCE(SUM(items_processed), 0)             AS total_items,
               AVG(duration_ms)::FLOAT8                      AS avg_duration_ms,
               MAX(ran_at)                                   AS last_ran_at,
               (ARRAY_AGG(success ORDER BY ran_at DESC))[1]  AS last_success,
               (ARRAY_AGG(error_reason ORDER BY ran_at DESC))[1] AS last_error
           FROM job_runs
           GROUP BY job_name
           ORDER BY job_name"#,
    )
    .fetch_all(pool.get_ref())
    .await?;
    Ok(HttpResponse::Ok().json(rows))
}

/// `GET /api/admin/jobs/:name/history?limit=50` — 单 job 的最近执行记录。
pub async fn history(
    pool: web::Data<DbPool>,
    name: web::Path<String>,
    query: web::Query<HistoryQuery>,
    _: AdminUser,
) -> AppResult<HttpResponse> {
    let limit = query.limit.unwrap_or(50).min(200) as i64;
    let rows = sqlx::query_as::<_, JobRun>(
        r#"SELECT id, job_name, ran_at, duration_ms, items_processed, success, error_reason
           FROM job_runs
           WHERE job_name = $1
           ORDER BY ran_at DESC
           LIMIT $2"#,
    )
    .bind(name.as_str())
    .bind(limit)
    .fetch_all(pool.get_ref())
    .await?;
    Ok(HttpResponse::Ok().json(rows))
}

#[derive(serde::Deserialize)]
pub struct HistoryQuery {
    pub limit: Option<u32>,
}
