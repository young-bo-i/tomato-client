use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::FromRow;
use uuid::Uuid;

/// Mirrors donutbrowser's `BrowserProfile` but stored per-user on the server.
/// The shape is kept identical on the wire so the Tauri client can just
/// serialize/deserialize without an adapter layer.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct BrowserProfile {
    pub id: Uuid,
    #[serde(skip_deserializing)]
    pub user_id: i32,
    pub name: String,
    pub browser: String,
    pub version: String,
    pub release_type: String,
    pub proxy_id: Option<String>,
    pub vpn_id: Option<String>,
    pub group_id: Option<String>,
    pub extension_group_id: Option<String>,
    pub tags: JsonValue,
    pub note: Option<String>,
    pub camoufox_config: Option<JsonValue>,
    pub wayfern_config: Option<JsonValue>,
    pub sync_mode: String,
    pub encryption_salt: Option<String>,
    pub last_sync: Option<i64>,
    pub last_launch: Option<i64>,
    pub host_os: Option<String>,
    pub ephemeral: bool,
    pub proxy_bypass_rules: JsonValue,
    pub created_by_id: Option<String>,
    pub created_by_email: Option<String>,
    pub dns_blocklist: Option<String>,
    /// Business-level classification: "tomato" / "qimao" / "douyin".
    /// Nullable for legacy profiles created before this column existed.
    pub kol_platform: Option<String>,

    // ── 七猫达人 credentials + server-managed token state ──
    // User-input (required by the create-profile dialog when
    // kol_platform='qimao'):
    pub qimao_identifier: Option<String>,
    pub qimao_credential: Option<String>,
    // Server-managed by `jobs::qimao_token_refresh`. Read-only to the
    // client; never accept these in create/update requests.
    pub qimao_token: Option<String>,
    pub qimao_token_refreshed_at: Option<DateTime<Local>>,
    pub qimao_token_last_error: Option<String>,

    pub created_at: DateTime<Local>,
    pub updated_at: DateTime<Local>,
}

/// Full-replace payload used on create. The client is the authority for
/// `id` (it's generated when the profile is created in the Tauri layer).
#[derive(Debug, Deserialize)]
pub struct CreateProfileRequest {
    pub id: Uuid,
    pub name: String,
    pub browser: String,
    pub version: String,
    #[serde(default = "default_release_type")]
    pub release_type: String,
    #[serde(default)]
    pub proxy_id: Option<String>,
    #[serde(default)]
    pub vpn_id: Option<String>,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub extension_group_id: Option<String>,
    #[serde(default = "empty_json_array")]
    pub tags: JsonValue,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub camoufox_config: Option<JsonValue>,
    #[serde(default)]
    pub wayfern_config: Option<JsonValue>,
    #[serde(default = "default_sync_mode")]
    pub sync_mode: String,
    #[serde(default)]
    pub encryption_salt: Option<String>,
    #[serde(default)]
    pub last_sync: Option<i64>,
    #[serde(default)]
    pub last_launch: Option<i64>,
    #[serde(default)]
    pub host_os: Option<String>,
    #[serde(default)]
    pub ephemeral: bool,
    #[serde(default = "empty_json_array")]
    pub proxy_bypass_rules: JsonValue,
    #[serde(default)]
    pub created_by_id: Option<String>,
    #[serde(default)]
    pub created_by_email: Option<String>,
    #[serde(default)]
    pub dns_blocklist: Option<String>,
    #[serde(default)]
    pub kol_platform: Option<String>,
    /// User-input qimao账号 (手机号/邮箱). Required by the dialog when
    /// kol_platform='qimao' but the server doesn't enforce that — the
    /// token-refresh worker just skips profiles without credentials.
    #[serde(default)]
    pub qimao_identifier: Option<String>,
    /// User-input qimao密码, plaintext at rest (matches legacy C# stack
    /// — see migration 019 for rationale).
    #[serde(default)]
    pub qimao_credential: Option<String>,
}

fn default_release_type() -> String {
    "stable".to_string()
}
fn default_sync_mode() -> String {
    "Disabled".to_string()
}
fn empty_json_array() -> JsonValue {
    JsonValue::Array(Vec::new())
}

/// Partial update — every field is optional. Only fields explicitly present
/// in the JSON body are modified. Last-write-wins, no optimistic locking.
#[derive(Debug, Deserialize, Default)]
pub struct UpdateProfileRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_id: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vpn_id: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_group_id: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camoufox_config: Option<Option<JsonValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wayfern_config: Option<Option<JsonValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_salt: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync: Option<Option<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_launch: Option<Option<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_os: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_bypass_rules: Option<JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dns_blocklist: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kol_platform: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qimao_identifier: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qimao_credential: Option<Option<String>>,
}
