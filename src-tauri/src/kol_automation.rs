//! KOL automation module.
//!
//! Hosts the Tauri commands the React layer calls into:
//! - `kol_login_*`, `kol_refresh_*` — thin event-emitter shims that ask
//!   the frontend to drive an existing browser-launch flow.
//! - `kol_gather_local_stats` — snapshot of the local ingest counters.
//!
//! The active gather pipeline is the extension-based one: the in-browser
//! MV3 extension (`extension/content.js`) posts rows to Donut's local axum
//! (`ingest`), which the `batch` rolling worker pool drives. CDP plumbing
//! lives in `cdp` (used by the one-shot DOM dump in `dump`).

pub mod batch;
pub mod cdp;
pub mod dedup;
pub mod dump;
pub mod extension;
pub mod ingest;
pub mod text_filter;

use serde_json::json;
use tauri::Emitter;

// ============================================================
// Login / refresh shims (frontend-driven)
// ============================================================

/// Ask the frontend to launch a browser pointed at the KOL platform.
#[tauri::command]
pub async fn kol_login_kol_platform(app: tauri::AppHandle) -> Result<String, String> {
  app
    .emit(
      "kol-open-url",
      json!({ "url": "https://kol.fanqieopen.com/", "purpose": "kol_login" }),
    )
    .map_err(|e| e.to_string())?;
  Ok("KOL login browser launched".into())
}

#[tauri::command]
pub async fn kol_login_douyin(app: tauri::AppHandle) -> Result<String, String> {
  app
    .emit(
      "kol-open-url",
      json!({ "url": "https://www.douyin.com/follow", "purpose": "douyin_login" }),
    )
    .map_err(|e| e.to_string())?;
  Ok("DouYin login browser launched".into())
}

#[tauri::command]
pub async fn kol_refresh_kol(app: tauri::AppHandle, kol_id: i32) -> Result<String, String> {
  app
    .emit("kol-refresh", json!({ "type": "kol", "id": kol_id }))
    .map_err(|e| e.to_string())?;
  Ok("Refresh initiated".into())
}

#[tauri::command]
pub async fn kol_refresh_douyin(
  app: tauri::AppHandle,
  douyin_id: i32,
) -> Result<String, String> {
  app
    .emit(
      "kol-refresh",
      json!({ "type": "douyin", "id": douyin_id }),
    )
    .map_err(|e| e.to_string())?;
  Ok("Refresh initiated".into())
}

// ============================================================
// Local ingest stats
// ============================================================

#[tauri::command]
pub fn kol_gather_local_stats() -> ingest::StatsSnapshot {
  ingest::snapshot_stats()
}
