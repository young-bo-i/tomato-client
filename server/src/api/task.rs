use actix_web::{web, HttpResponse};
use crate::db::DbPool;
use crate::errors::AppResult;
use crate::middleware::auth::UserId;
use crate::models::brush_task::*;
use sqlx::Row;

pub async fn get_task_data_grid(
    pool: web::Data<DbPool>,
    user: UserId,
    body: web::Json<TaskQueryRequest>,
) -> AppResult<HttpResponse> {
    let page = body.page.unwrap_or(1).max(1);
    let page_size = body.page_size.unwrap_or(20).min(100);
    let offset = (page - 1) * page_size;

    let date_filter = match body.date_range.as_deref() {
        Some("day") => "AND created_at >= CURRENT_DATE",
        Some("week") => "AND created_at >= CURRENT_DATE - INTERVAL '7 days'",
        Some("month") => "AND created_at >= CURRENT_DATE - INTERVAL '30 days'",
        _ => "",
    };

    let platform_filter = match body.platform {
        Some(p) if p > 0 => format!("AND platform = {}", p),
        _ => String::new(),
    };

    let count_sql = format!(
        "SELECT COUNT(*) as count FROM kol_brush_task WHERE account_id = $1 AND is_deleted = FALSE {} {}",
        date_filter, platform_filter
    );
    let total: (i64,) = sqlx::query_as(&count_sql)
        .bind(user.0)
        .fetch_one(pool.get_ref())
        .await?;

    let query_sql = format!(
        "SELECT * FROM kol_brush_task WHERE account_id = $1 AND is_deleted = FALSE {} {} ORDER BY created_at DESC LIMIT $2 OFFSET $3",
        date_filter, platform_filter
    );
    let items = sqlx::query_as::<_, KolBrushTask>(&query_sql)
        .bind(user.0)
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool.get_ref())
        .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": TaskDataGrid { items, total: total.0, page, page_size }
    })))
}

pub async fn get_task_summary(
    pool: web::Data<DbPool>,
    user: UserId,
) -> AppResult<HttpResponse> {
    let total: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM kol_brush_task WHERE account_id = $1 AND is_deleted = FALSE"
    )
    .bind(user.0)
    .fetch_one(pool.get_ref())
    .await?;

    let today: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM kol_brush_task WHERE account_id = $1 AND is_deleted = FALSE AND created_at >= CURRENT_DATE"
    )
    .bind(user.0)
    .fetch_one(pool.get_ref())
    .await?;

    let no_callback: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM kol_brush_task WHERE account_id = $1 AND is_deleted = FALSE AND write_back_status = 0"
    )
    .bind(user.0)
    .fetch_one(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": TaskSummary {
            total_count: total.0,
            today_count: today.0,
            no_callback_count: no_callback.0,
        }
    })))
}

pub async fn get_recent_tasks(
    pool: web::Data<DbPool>,
    user: UserId,
) -> AppResult<HttpResponse> {
    let rows = sqlx::query(
        r#"SELECT platform, DATE(created_at) as day, COUNT(*) as count
         FROM kol_brush_task
         WHERE account_id = $1 AND is_deleted = FALSE
           AND created_at >= CURRENT_DATE - INTERVAL '7 days'
         GROUP BY platform, DATE(created_at)
         ORDER BY day DESC"#,
    )
    .bind(user.0)
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": rows.iter().map(|r| serde_json::json!({
            "platform": r.get::<i16, _>("platform"),
            "day": r.get::<chrono::NaiveDate, _>("day"),
            "count": r.get::<i64, _>("count"),
        })).collect::<Vec<_>>(),
    })))
}

pub async fn get_recent_income(
    pool: web::Data<DbPool>,
    user: UserId,
) -> AppResult<HttpResponse> {
    let incomes = sqlx::query_as::<_, crate::models::income::KolIncome>(
        "SELECT * FROM kol_income WHERE account_id = $1 ORDER BY last_update_time DESC LIMIT 10",
    )
    .bind(user.0)
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": incomes,
    })))
}

pub async fn get_books(
    pool: web::Data<DbPool>,
    _user: UserId,
    query: web::Query<BookQuery>,
) -> AppResult<HttpResponse> {
    let platform = query.platform.unwrap_or(1);
    let books = sqlx::query_as::<_, crate::models::book::KolBook>(
        "SELECT * FROM kol_book WHERE platform = $1 AND is_deleted = FALSE ORDER BY created_at DESC",
    )
    .bind(platform)
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": books,
    })))
}

#[derive(serde::Deserialize)]
pub struct BookQuery {
    pub platform: Option<i16>,
}
