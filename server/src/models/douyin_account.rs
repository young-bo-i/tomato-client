use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DouYinAccount {
    pub id: i32,
    pub account_id: i32,
    pub storage_state: Option<String>,   // JSON browser storage state
    pub nickname: Option<String>,
    pub remark: Option<String>,
    pub status: i16,
    pub is_deleted: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct SubmitDouYinRequest {
    pub storage_state: String,
    pub nickname: Option<String>,
    pub remark: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDouYinRequest {
    pub id: i32,
    pub storage_state: String,
}

#[derive(Debug, Serialize, FromRow)]
pub struct DouYinAccountInfo {
    pub id: i32,
    pub account_id: i32,
    pub nickname: Option<String>,
    pub remark: Option<String>,
    pub status: i16,
    pub created_at: NaiveDateTime,
}
