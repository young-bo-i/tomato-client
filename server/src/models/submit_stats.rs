use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SubmitBrushRequest {
    pub id: i64,
    pub account_id: i32,
    pub douyin_id: i32,
    pub submit_time: NaiveDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SubmitWordStatistics {
    pub id: i64,
    pub account_id: i32,
    pub douyin_id: i32,
    pub original_word: String,
    pub filter_word: String,
    pub submit_time: NaiveDateTime,
}

#[derive(Debug, Serialize, FromRow)]
pub struct RequestFrequencyPoint {
    pub time_bucket: String,
    pub count: i64,
}
