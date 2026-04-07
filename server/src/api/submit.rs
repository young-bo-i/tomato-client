use actix_web::{web, HttpResponse};
use crate::db::DbPool;
use crate::errors::AppResult;
use crate::middleware::auth::UserId;
use crate::models::brush_task::{SubmitBrushMessage, SubmitBrushTaskRequest};
use crate::models::submit_stats::RequestFrequencyPoint;
use crate::services::submit::SubmitService;

/// High-throughput endpoint: 100+ req/s
/// Flow: validate → filter text → record stats → push to Redis Stream → return
pub async fn submit_brush_task(
    pool: web::Data<DbPool>,
    redis: web::Data<redis::Client>,
    user: UserId,
    body: web::Json<SubmitBrushTaskRequest>,
) -> AppResult<HttpResponse> {
    let submit_service = SubmitService::new(pool.get_ref().clone(), redis.get_ref().clone());
    let result = submit_service.submit(user.0, body.into_inner()).await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": result,
        "data": result,
    })))
}

pub async fn get_request_frequency(
    pool: web::Data<DbPool>,
    user: UserId,
    query: web::Query<FrequencyQuery>,
) -> AppResult<HttpResponse> {
    let interval = query.interval.as_deref().unwrap_or("10min");

    let points = match interval {
        "1min" => get_frequency_by_interval(&pool, user.0, "1 minute", 60).await?,
        "10min" => get_frequency_by_interval(&pool, user.0, "10 minutes", 60).await?,
        "20min" => get_frequency_by_interval(&pool, user.0, "20 minutes", 72).await?,
        "4hour" => get_frequency_by_interval(&pool, user.0, "4 hours", 42).await?,
        _ => get_frequency_by_interval(&pool, user.0, "10 minutes", 60).await?,
    };

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": points,
    })))
}

#[derive(serde::Deserialize)]
pub struct FrequencyQuery {
    pub interval: Option<String>,
}

async fn get_frequency_by_interval(
    pool: &DbPool,
    account_id: i32,
    interval: &str,
    limit: i64,
) -> Result<Vec<RequestFrequencyPoint>, crate::errors::AppError> {
    let query = format!(
        r#"SELECT
             to_char(date_trunc('minute', submit_time), 'YYYY-MM-DD HH24:MI') as time_bucket,
             COUNT(*) as count
           FROM submit_brush_request
           WHERE account_id = $1
             AND submit_time >= NOW() - INTERVAL '{}'  * $2
           GROUP BY time_bucket
           ORDER BY time_bucket DESC
           LIMIT $2"#,
        interval
    );

    let rows = sqlx::query_as::<_, RequestFrequencyPoint>(&query)
        .bind(account_id)
        .bind(limit)
        .fetch_all(pool)
        .await?;

    Ok(rows)
}
