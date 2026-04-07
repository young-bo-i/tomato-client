use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct KolAccount {
    pub id: i32,
    pub account_id: i32,
    pub cookies: Option<String>,         // JSON string of cookies
    pub uid: Option<String>,
    pub identity_name: Option<String>,
    pub identity_number: Option<String>,
    pub payment_account: Option<String>,
    pub mobile: Option<String>,
    pub remark: Option<String>,
    pub status: i16,
    pub is_deleted: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct SubmitCookiesRequest {
    pub cookies: String,
    pub uid: Option<String>,
    pub identity_name: Option<String>,
    pub remark: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCookiesRequest {
    pub id: i32,
    pub cookies: String,
}

#[derive(Debug, Serialize, FromRow)]
pub struct KolAccountInfo {
    pub id: i32,
    pub account_id: i32,
    pub uid: Option<String>,
    pub identity_name: Option<String>,
    pub remark: Option<String>,
    pub status: i16,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Serialize, FromRow)]
pub struct KolAccountFull {
    pub id: i32,
    pub account_id: i32,
    pub cookies: Option<String>,
    pub uid: Option<String>,
    pub identity_name: Option<String>,
    pub identity_number: Option<String>,
    pub payment_account: Option<String>,
    pub mobile: Option<String>,
    pub remark: Option<String>,
    pub status: i16,
    pub created_at: NaiveDateTime,
}
