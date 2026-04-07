use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct KolBook {
    pub id: i64,
    pub book_id: String,
    pub book_name: String,
    pub platform: i16,
    pub is_deleted: bool,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct QiMaoBook {
    pub id: i64,
    pub book_id: String,
    pub book_name: String,
    pub is_forbidden: bool,
    pub is_deleted: bool,
    pub created_at: NaiveDateTime,
}
