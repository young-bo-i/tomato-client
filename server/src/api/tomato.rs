//! Read + admin-trigger endpoints for the tomato_books table.

use actix_web::{web, HttpResponse};
use chrono::{DateTime, Local};
use serde::Serialize;
use serde_json::Value as JsonValue;
use sqlx::FromRow;

use crate::auth::{AdminUser, AuthUser};
use crate::db::DbPool;
use crate::errors::AppResult;
use crate::jobs::tomato_rank;

#[derive(Debug, Serialize, FromRow)]
pub struct TomatoBook {
    pub position: i32,
    pub book_id: String,
    pub book_name: String,
    pub author: Option<String>,
    pub word_num: Option<i64>,
    pub score: Option<f64>,
    pub chapter_num: Option<i32>,
    pub recent_income: Option<i64>,
    pub thumb_url: Option<String>,
    pub book_abstract: Option<String>,
    pub categories: Option<JsonValue>,
    pub promotion_types: Option<JsonValue>,
    pub fetched_at: DateTime<Local>,
}

/// `GET /api/tomato/books` — current snapshot in rank order.
pub async fn list(pool: web::Data<DbPool>, _: AuthUser) -> AppResult<HttpResponse> {
    let rows = sqlx::query_as::<_, TomatoBook>(
        r#"SELECT position, book_id, book_name, author, word_num, score, chapter_num,
                  recent_income, thumb_url, book_abstract, categories, promotion_types,
                  fetched_at
           FROM tomato_books
           ORDER BY position ASC"#,
    )
    .fetch_all(pool.get_ref())
    .await?;
    Ok(HttpResponse::Ok().json(rows))
}

/// `POST /api/tomato/books/refresh` — admin-only, kicks off the daily
/// scrape job synchronously (so the response carries the result count).
/// Useful for testing or one-off "force refresh now" without waiting
/// for the 03:00 cron tick.
pub async fn refresh(
    pool: web::Data<DbPool>,
    abogus_url: web::Data<String>,
    _: AdminUser,
) -> AppResult<HttpResponse> {
    if let Err(e) = tomato_rank::run(pool.get_ref(), abogus_url.get_ref()).await {
        return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
            "ok": false, "error": e
        })));
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tomato_books")
        .fetch_one(pool.get_ref())
        .await?;
    Ok(HttpResponse::Ok().json(serde_json::json!({ "ok": true, "count": count })))
}
