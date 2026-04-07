use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Browser profile for fingerprint browser sync
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct BrowserProfile {
    pub id: Uuid,
    pub account_id: i32,
    pub name: String,
    pub browser_type: String,           // "chromium" or "firefox"
    pub fingerprint_config: serde_json::Value,
    pub proxy_config: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub last_sync_at: Option<NaiveDateTime>,
    pub is_deleted: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Profile data archive reference (stored in object storage)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ProfileArchive {
    pub id: i64,
    pub profile_id: Uuid,
    pub file_hash: String,
    pub file_size: i64,
    pub storage_path: String,           // path in object storage
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct CreateProfileRequest {
    pub name: String,
    pub browser_type: Option<String>,
    pub fingerprint_config: Option<serde_json::Value>,
    pub proxy_config: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    pub name: Option<String>,
    pub fingerprint_config: Option<serde_json::Value>,
    pub proxy_config: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct ProfileSyncStatus {
    pub profile_id: Uuid,
    pub last_sync_at: Option<NaiveDateTime>,
    pub file_hash: Option<String>,
    pub file_size: Option<i64>,
}
