//! KOL automation module.
//!
//! Hosts the Tauri commands the React layer calls into:
//! - `kol_login_*`, `kol_refresh_*` — thin event-emitter shims that ask
//!   the frontend to drive an existing browser-launch flow. Unchanged
//!   from the prior implementation; preserved for the React panels that
//!   still call them.
//! - `kol_start_gather` / `kol_stop_gather` / `kol_gather_status` /
//!   `kol_is_gather_running` — the new, real gather pipeline. The heavy
//!   lifting lives in `gather` (Orchestrator + Worker + Batcher) and
//!   `cdp` (persistent CDP client). See those modules for design notes.

pub mod batch;
pub mod cdp;
pub mod dedup;
pub mod dump;
pub mod extension;
pub mod gather;
pub mod ingest;
pub mod text_filter;

use serde_json::json;
use tauri::Emitter;

pub use gather::{GatherStatus, ProfileStatus, StartGatherRequest};

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
// Gather pipeline (real)
// ============================================================

#[tauri::command]
pub async fn kol_start_gather(
  app: tauri::AppHandle,
  request: StartGatherRequest,
) -> Result<GatherStatus, String> {
  gather::start(app, request).await
}

#[tauri::command]
pub fn kol_stop_gather() -> Result<GatherStatus, String> {
  gather::stop()
}

#[tauri::command]
pub fn kol_is_gather_running() -> bool {
  gather::is_running()
}

#[tauri::command]
pub fn kol_gather_status() -> GatherStatus {
  gather::current_status()
}

#[tauri::command]
pub fn kol_gather_local_stats() -> ingest::StatsSnapshot {
  ingest::snapshot_stats()
}
