//! Admin-only CRUD for the SMTP / notification email settings, plus
//! a "send test email" hatch so the operator can verify the config
//! without waiting for a real notification.
//!
//! Single-row design: the table is seeded with id=1 by migration 021.
//! `GET` always returns something (defaults if never configured),
//! `PUT` upserts in place. The password is never returned in `GET`
//! responses — the API surfaces an `is_password_set` boolean instead
//! so the form can show "**** (saved)" without exposing it.

use actix_web::{web, HttpResponse};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use sqlx::Row;

use crate::auth::AdminUser;
use crate::db::DbPool;
use crate::errors::AppResult;
use crate::services::email_sender;

/// Public-facing email settings shape (password redacted).
#[derive(Debug, Serialize)]
pub struct EmailSettingsView {
    pub smtp_host: String,
    pub smtp_port: i32,
    pub smtp_username: String,
    /// True when a non-empty password is stored on the server. The
    /// actual value is never returned.
    pub is_password_set: bool,
    pub from_address: String,
    pub from_name: String,
    pub use_tls: bool,
    pub recipients: Vec<String>,
    pub updated_at: DateTime<Local>,
}

/// `GET /api/admin/email_settings` — fetch the current configuration.
/// Always returns 200 with defaults if never configured (the row is
/// seeded by migration 021).
pub async fn get(pool: web::Data<DbPool>, _: AdminUser) -> AppResult<HttpResponse> {
    let row = sqlx::query(
        r#"SELECT smtp_host, smtp_port, smtp_username, smtp_password,
                  from_address, from_name, use_tls, recipients, updated_at
           FROM email_settings WHERE id = 1"#,
    )
    .fetch_one(pool.get_ref())
    .await?;

    let recipients: JsonValue = row.try_get("recipients").unwrap_or(JsonValue::Array(vec![]));
    let recipients_vec: Vec<String> = recipients
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let smtp_password: String = row.try_get("smtp_password").unwrap_or_default();

    Ok(HttpResponse::Ok().json(EmailSettingsView {
        smtp_host: row.try_get("smtp_host").unwrap_or_default(),
        smtp_port: row.try_get("smtp_port").unwrap_or(587),
        smtp_username: row.try_get("smtp_username").unwrap_or_default(),
        is_password_set: !smtp_password.is_empty(),
        from_address: row.try_get("from_address").unwrap_or_default(),
        from_name: row.try_get("from_name").unwrap_or_default(),
        use_tls: row.try_get("use_tls").unwrap_or(true),
        recipients: recipients_vec,
        updated_at: row.try_get("updated_at")?,
    }))
}

#[derive(Debug, Deserialize)]
pub struct UpdateBody {
    pub smtp_host: String,
    pub smtp_port: i32,
    pub smtp_username: String,
    /// Optional. If omitted (`null` / missing), the existing password
    /// stays unchanged — lets the UI submit the form without re-typing
    /// the password every time. Empty string explicitly clears it.
    #[serde(default)]
    pub smtp_password: Option<String>,
    pub from_address: String,
    #[serde(default)]
    pub from_name: String,
    pub use_tls: bool,
    #[serde(default)]
    pub recipients: Vec<String>,
}

/// `PUT /api/admin/email_settings` — replace the (single) settings
/// row. Password handling: omit / null = preserve existing, "" =
/// explicitly clear, anything else = set new.
pub async fn put(
    pool: web::Data<DbPool>,
    body: web::Json<UpdateBody>,
    _: AdminUser,
) -> AppResult<HttpResponse> {
    let b = body.into_inner();
    let recipients_json = serde_json::to_value(&b.recipients).unwrap_or(JsonValue::Array(vec![]));

    // Two SQL paths: with password or without. Branched explicitly so
    // "absent (omit field) preserves existing" stays distinct from
    // "explicit empty string clears" — COALESCE can't express that.
    let updated = if let Some(pw) = b.smtp_password {
        sqlx::query(
            r#"UPDATE email_settings
               SET smtp_host=$1, smtp_port=$2, smtp_username=$3,
                   smtp_password=$4,
                   from_address=$5, from_name=$6, use_tls=$7,
                   recipients=$8, updated_at=NOW()
               WHERE id=1"#,
        )
        .bind(&b.smtp_host)
        .bind(b.smtp_port)
        .bind(&b.smtp_username)
        .bind(&pw)
        .bind(&b.from_address)
        .bind(&b.from_name)
        .bind(b.use_tls)
        .bind(&recipients_json)
        .execute(pool.get_ref())
        .await?
        .rows_affected()
    } else {
        sqlx::query(
            r#"UPDATE email_settings
               SET smtp_host=$1, smtp_port=$2, smtp_username=$3,
                   from_address=$4, from_name=$5, use_tls=$6,
                   recipients=$7, updated_at=NOW()
               WHERE id=1"#,
        )
        .bind(&b.smtp_host)
        .bind(b.smtp_port)
        .bind(&b.smtp_username)
        .bind(&b.from_address)
        .bind(&b.from_name)
        .bind(b.use_tls)
        .bind(&recipients_json)
        .execute(pool.get_ref())
        .await?
        .rows_affected()
    };

    if updated == 0 {
        // Should be unreachable: migration 021 seeds the row.
        return Ok(HttpResponse::InternalServerError().json(json!({
            "ok": false, "error": "settings row missing"
        })));
    }
    Ok(HttpResponse::Ok().json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct TestBody {
    /// Optional explicit recipient. When omitted, send to the first
    /// configured recipient (`recipients[0]`).
    #[serde(default)]
    pub to: Option<String>,
}

/// `POST /api/admin/email_settings/test` — synchronously send a one-off
/// test email so the operator gets immediate feedback on whether the
/// SMTP config actually works.
///
/// Returns `502` with the SMTP error verbatim when the send fails.
/// `400` when there's no usable recipient.
pub async fn send_test(
    pool: web::Data<DbPool>,
    body: web::Json<TestBody>,
    _: AdminUser,
) -> AppResult<HttpResponse> {
    let settings = email_sender::load(pool.get_ref())
        .await
        .map_err(|e| crate::errors::AppError::BadRequest(format!("load settings: {e}")))?;

    let to_addr = body
        .to
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| settings.recipients.first().cloned());
    let Some(to_addr) = to_addr else {
        return Ok(HttpResponse::BadRequest().json(json!({
            "ok": false,
            "error": "no recipient: pass {\"to\": \"...\"} or configure default recipients"
        })));
    };

    let subject = "Tomato KOL · 测试邮件";
    let body_text = format!(
        "这是一封测试邮件,用于验证 SMTP 配置是否正常。\n\n\
         发件主机: {}:{}\n\
         发件地址: {}\n\
         收件人: {}\n",
        settings.smtp_host, settings.smtp_port, settings.from_address, to_addr
    );

    match email_sender::send(&settings, &[to_addr.clone()], subject, &body_text).await {
        Ok(()) => Ok(HttpResponse::Ok().json(json!({ "ok": true, "to": to_addr }))),
        Err(e) => Ok(HttpResponse::BadGateway().json(json!({
            "ok": false, "error": e
        }))),
    }
}
