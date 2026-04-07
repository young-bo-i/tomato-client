use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Account {
    pub id: i32,
    pub account_name: String,
    pub password_hash: String,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub status: i16,           // 1=enabled, 0=disabled
    pub parent_id: Option<i32>,
    pub is_deleted: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub account: String,       // phone, email, or account_name
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub account_id: i32,
    pub account_name: String,
}

#[derive(Debug, Serialize, FromRow)]
pub struct AccountInfo {
    pub id: i32,
    pub account_name: String,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub status: i16,
    pub parent_id: Option<i32>,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct CreateAccountRequest {
    pub account_name: String,
    pub password: String,
    pub phone: Option<String>,
    pub email: Option<String>,
}
