//! Read + admin-trigger endpoints for the qimao_books table, plus the
//! per-profile setter for `x-qm-devops-token` (the qimao session uses
//! a non-cookie header that the browser keeps in localStorage; the
//! admin pastes it here after logging in).

use actix_web::{web, HttpResponse};
use chrono::{DateTime, Local};
use serde::Serialize;
use serde_json::{json, Value as JsonValue};
use sqlx::{FromRow, Row};
use uuid::Uuid;

use crate::auth::{AdminUser, AuthUser};
use crate::db::DbPool;
use crate::errors::AppResult;
use crate::jobs::qimao_rank;
use crate::services::qimao_promotion::{
    build_http_client, signin, ENDPOINT_SIGNIN, SERVICE_NAME,
};

#[derive(Debug, Serialize, FromRow)]
pub struct QimaoBook {
    pub position: i32,
    pub book_id: i64,
    pub book_name: String,
    pub author: Option<String>,
    pub first_category: Option<String>,
    pub second_category: Option<String>,
    pub words_num_text: Option<String>,
    pub words: Option<i64>,
    pub cover: Option<String>,
    pub intro: Option<String>,
    pub income_text: Option<String>,
    pub is_forbid: bool,
    pub is_rights: bool,
    pub ad_status: Option<i32>,
    pub tags: Option<JsonValue>,
    pub fetched_at: DateTime<Local>,
}

/// `GET /api/qimao/books` — current snapshot in rank order.
pub async fn list(pool: web::Data<DbPool>, _: AuthUser) -> AppResult<HttpResponse> {
    let rows = sqlx::query_as::<_, QimaoBook>(
        r#"SELECT position, book_id, book_name, author, first_category, second_category,
                  words_num_text, words, cover, intro, income_text,
                  is_forbid, is_rights, ad_status, tags, fetched_at
           FROM qimao_books
           ORDER BY position ASC"#,
    )
    .fetch_all(pool.get_ref())
    .await?;
    Ok(HttpResponse::Ok().json(rows))
}

/// `POST /api/qimao/books/refresh` — admin-only, kicks off the daily
/// scrape job synchronously (so the response carries the result count).
pub async fn refresh(pool: web::Data<DbPool>, _: AdminUser) -> AppResult<HttpResponse> {
    if let Err(e) = qimao_rank::run(pool.get_ref()).await {
        return Ok(HttpResponse::InternalServerError().json(json!({
            "ok": false, "error": e
        })));
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM qimao_books")
        .fetch_one(pool.get_ref())
        .await?;
    Ok(HttpResponse::Ok().json(json!({ "ok": true, "count": count })))
}

/// `POST /api/profiles/{id}/qimao_refresh_token` — admin-only.
/// Synchronously calls /api/v1/user/signin with the profile's stored
/// credentials and persists the resulting token. Useful for "I just
/// changed the password, refresh now" instead of waiting for the
/// background worker's next sweep (up to 30 min later).
///
/// Returns 404 if the profile doesn't exist or has no credentials.
/// Returns 502 with the upstream's error message if signin failed.
pub async fn refresh_token(
    pool: web::Data<DbPool>,
    profile_id: web::Path<Uuid>,
    _: AdminUser,
) -> AppResult<HttpResponse> {
    let profile_id = profile_id.into_inner();

    let row = sqlx::query(
        r#"SELECT qimao_identifier, qimao_credential
           FROM browser_profiles
           WHERE id = $1
             AND kol_platform = 'qimao'
             AND qimao_identifier IS NOT NULL AND qimao_identifier <> ''
             AND qimao_credential IS NOT NULL AND qimao_credential <> ''"#,
    )
    .bind(profile_id)
    .fetch_optional(pool.get_ref())
    .await?;
    let Some(row) = row else {
        return Ok(HttpResponse::NotFound().json(json!({
            "ok": false,
            "error": "profile not found, not qimao, or missing credentials"
        })));
    };
    let identifier: String = row
        .try_get("qimao_identifier")
        .map_err(|e| crate::errors::AppError::BadRequest(format!("identifier col: {e}")))?;
    let credential: String = row
        .try_get("qimao_credential")
        .map_err(|e| crate::errors::AppError::BadRequest(format!("credential col: {e}")))?;

    let http = build_http_client()
        .map_err(|e| crate::errors::AppError::BadRequest(format!("http client: {e}")))?;
    let outcome = signin(&http, &identifier, &credential).await;
    let request_summary = json!({ "profile_id": profile_id, "trigger": "manual" });

    match outcome
        .audit(pool.get_ref(), SERVICE_NAME, ENDPOINT_SIGNIN, request_summary)
        .await
    {
        Ok(token) => {
            sqlx::query(
                r#"UPDATE browser_profiles
                   SET qimao_token = $1,
                       qimao_token_refreshed_at = NOW(),
                       qimao_token_last_error = NULL
                   WHERE id = $2"#,
            )
            .bind(&token)
            .bind(profile_id)
            .execute(pool.get_ref())
            .await?;
            Ok(HttpResponse::Ok().json(json!({ "ok": true })))
        }
        Err(err) => {
            let reason = err.to_string();
            sqlx::query(
                r#"UPDATE browser_profiles
                   SET qimao_token_last_error = $1,
                       qimao_token_refreshed_at = NOW()
                   WHERE id = $2"#,
            )
            .bind(&reason)
            .bind(profile_id)
            .execute(pool.get_ref())
            .await?;
            Ok(HttpResponse::BadGateway().json(json!({
                "ok": false,
                "error": reason,
            })))
        }
    }
}
