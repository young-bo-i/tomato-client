use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Tomato platform brush task
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct KolBrushTask {
    pub id: i64,
    pub account_id: i32,
    pub kol_id: i32,
    pub alias_name: String,
    pub alias_id: Option<String>,
    pub share_url: Option<String>,
    pub first_picture_url: Option<String>,
    pub platform: i16,                    // 1=XiaoShuo, 2=TouTiao, 3=ChangTing, 4=WuKong
    pub task_status: i16,
    pub write_back_status: i16,           // 0=pending, 1=done, 2=expired, 3=change_url
    pub write_back_time: Option<NaiveDateTime>,
    pub is_deleted: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// QiMao platform brush task
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct QiMaoBrushTask {
    pub id: i64,
    pub account_id: i32,
    pub qimao_account_id: i32,
    pub alias_name: String,
    pub alias_id: Option<String>,
    pub share_url: Option<String>,
    pub platform: i16,
    pub task_status: i16,                 // 0=under_review, 1=taking_effect, 2=invalid
    pub write_back_status: i16,
    pub write_back_time: Option<NaiveDateTime>,
    pub is_deleted: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Non-task brush record (failed/fallback)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct KolBrushNonTask {
    pub id: i64,
    pub account_id: i32,
    pub kol_id: i32,
    pub alias_name: String,
    pub share_url: Option<String>,
    pub platform: i16,
    pub is_deleted: bool,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct SubmitBrushTaskRequest {
    pub douyin_id: i32,
    pub alias_name: String,
    pub share_url: Option<String>,
    pub first_picture_url: Option<String>,
}

/// Kafka/Redis message for async processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitBrushMessage {
    pub account_id: i32,
    pub douyin_id: i32,
    pub alias_name: String,
    pub share_url: Option<String>,
    pub first_picture_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TaskQueryRequest {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub date_range: Option<String>,      // "day", "week", "month"
    pub platform: Option<i16>,
}

#[derive(Debug, Serialize)]
pub struct TaskDataGrid {
    pub items: Vec<KolBrushTask>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Debug, Serialize)]
pub struct TaskSummary {
    pub total_count: i64,
    pub today_count: i64,
    pub no_callback_count: i64,
}

/// Platform types for Tomato
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i16)]
pub enum AliasType {
    XiaoShuo = 1,
    TouTiao = 2,
    ChangTing = 3,
    WuKong = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i16)]
pub enum WriteBackStatus {
    Pending = 0,
    Done = 1,
    Expired = 2,
    ChangeUrl = 3,
}
