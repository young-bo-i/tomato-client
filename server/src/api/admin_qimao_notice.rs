//! Admin-only read endpoint for the 七猫 monthly income notice
//! history. Backs the "七猫收益通知" panel.
//!
//! `GET /api/admin/qimao_notices` — every row in `qimao_income_notice`
//! joined to the profile + owner, sorted newest-emailed-first. The
//! UI renders `content_html` in a sandboxed surface so the inline
//! styles from the upstream don't leak into the page.

use actix_web::{web, HttpResponse};
use chrono::{DateTime, Local, NaiveDate};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

use crate::auth::AdminUser;
use crate::db::DbPool;
use crate::errors::AppResult;

/// One historical notice. `emailed_at` is NULL when the email send
/// failed (in which case `send_error` carries the SMTP error string);
/// the operator can re-send by deleting the row and waiting for the
/// next cron fire.
#[derive(Debug, Serialize, FromRow)]
pub struct NoticeRow {
    pub profile_id: Uuid,
    pub profile_name: String,
    pub owner_user_id: i32,
    pub owner_username: String,

    pub message_id: i64,
    pub title: String,
    pub content_html: String,
    pub notice_date: Option<NaiveDate>,

    pub recipient_email: Option<String>,
    pub emailed_at: Option<DateTime<Local>>,
    pub send_error: Option<String>,

    pub created_at: DateTime<Local>,
}

pub async fn list(pool: web::Data<DbPool>, _: AdminUser) -> AppResult<HttpResponse> {
    let rows = sqlx::query_as::<_, NoticeRow>(
        r#"SELECT
              n.profile_id,
              bp.name             AS profile_name,
              bp.user_id          AS owner_user_id,
              u.username          AS owner_username,
              n.message_id, n.title, n.content_html, n.notice_date,
              n.recipient_email, n.emailed_at, n.send_error,
              n.created_at
           FROM qimao_income_notice n
           JOIN browser_profiles bp ON bp.id = n.profile_id
           JOIN users u             ON u.id = bp.user_id
           ORDER BY n.created_at DESC
           LIMIT 500"#,
    )
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(rows))
}
