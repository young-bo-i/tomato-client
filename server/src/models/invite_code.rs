use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct KolInviteCode {
    pub id: i64,
    pub account_id: i32,
    pub kol_id: i32,
    pub invite_code: String,
    pub share_token: Option<String>,
    pub x_kol_token: Option<String>,
    pub last_refresh_time: Option<NaiveDateTime>,
    pub is_deleted: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}
