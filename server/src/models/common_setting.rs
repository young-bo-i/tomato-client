use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CommonSetting {
    pub id: i32,
    pub account_id: i32,
    pub kol_id: i32,
    pub scene: String,          // "OpenBrushPlatform", "BrushLimit", etc.
    pub setting_value: String,  // JSON value
    pub is_deleted: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomConfig {
    pub selectors: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct SavePlatformRequest {
    pub kol_id: i32,
    pub open_types: Vec<i16>,
}

#[derive(Debug, Deserialize)]
pub struct SaveLimitRequest {
    pub kol_id: i32,
    pub platform: i16,
    pub limit: i32,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDomRequest {
    pub dom_type: String,       // "douyin" or "kol"
    pub selectors: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct IncomeNoticeSettingRequest {
    pub emails: Vec<String>,
    pub has_child: bool,
}
