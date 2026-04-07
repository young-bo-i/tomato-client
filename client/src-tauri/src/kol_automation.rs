//! KOL Automation Module
//!
//! Handles browser automation for:
//! - KOL platform login (fanqieopen.com)
//! - DouYin login and cookie extraction
//! - DouYin video gathering (DOM automation)
//! - Scheduled task execution
//!
//! Uses the existing Donut Browser profile infrastructure
//! to launch fingerprint-protected browser instances.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

/// DOM selectors for DouYin automation (fetched from server)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomConfig {
    #[serde(rename = "IsOpenSite")]
    pub is_open_site: Option<String>,
    #[serde(rename = "IsLogin")]
    pub is_login: Option<String>,
    #[serde(rename = "VideoContainerSelector")]
    pub video_container_selector: Option<String>,
    #[serde(rename = "LiveSelector")]
    pub live_selector: Option<String>,
    #[serde(rename = "VideoIdAttr")]
    pub video_id_attr: Option<String>,
    #[serde(rename = "SuggestWork")]
    pub suggest_work: Option<String>,
    #[serde(rename = "BottomInfo")]
    pub bottom_info: Option<String>,
    #[serde(rename = "VideoTitle")]
    pub video_title: Option<String>,
    #[serde(rename = "FirstFrame")]
    pub first_frame: Option<String>,
    #[serde(rename = "NextButton")]
    pub next_button: Option<String>,
}

/// Auto-gather configuration from the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoGatherConfig {
    pub enabled_douyin_ids: Vec<i32>,
    pub start_time: String,
    pub end_time: String,
    pub interval_ms: u64,
    pub videos_per_session: u32,
}

/// Gathered video data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatheredVideo {
    pub douyin_id: i32,
    pub alias_name: String,
    pub share_url: String,
    pub first_picture_url: Option<String>,
}

/// Global gather state
static GATHER_RUNNING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

// ============================================================
// Tauri Commands
// ============================================================

/// Launch a browser profile to login to KOL platform (fanqieopen.com)
/// After user logs in manually, extract cookies and send to server
#[tauri::command]
pub async fn kol_login_kol_platform(
    app: tauri::AppHandle,
) -> Result<String, String> {
    // Create a temporary browser profile for KOL login
    // Using Donut Browser's existing profile infrastructure
    let url = "https://kol.fanqieopen.com/";

    // Emit event to frontend to handle via existing profile launch
    app.emit("kol-open-url", serde_json::json!({
        "url": url,
        "purpose": "kol_login",
    }))
    .map_err(|e| e.to_string())?;

    Ok("KOL login browser launched".into())
}

/// Launch a browser to login to DouYin
#[tauri::command]
pub async fn kol_login_douyin(
    app: tauri::AppHandle,
) -> Result<String, String> {
    let url = "https://www.douyin.com/follow";

    app.emit("kol-open-url", serde_json::json!({
        "url": url,
        "purpose": "douyin_login",
    }))
    .map_err(|e| e.to_string())?;

    Ok("DouYin login browser launched".into())
}

/// Refresh KOL account cookies
#[tauri::command]
pub async fn kol_refresh_kol(
    app: tauri::AppHandle,
    kol_id: i32,
) -> Result<String, String> {
    app.emit("kol-refresh", serde_json::json!({
        "type": "kol",
        "id": kol_id,
    }))
    .map_err(|e| e.to_string())?;

    Ok("Refresh initiated".into())
}

/// Refresh DouYin account
#[tauri::command]
pub async fn kol_refresh_douyin(
    app: tauri::AppHandle,
    douyin_id: i32,
) -> Result<String, String> {
    app.emit("kol-refresh", serde_json::json!({
        "type": "douyin",
        "id": douyin_id,
    }))
    .map_err(|e| e.to_string())?;

    Ok("Refresh initiated".into())
}

/// Start the auto-gather process
/// This runs in the background, using CDP to control browser instances
#[tauri::command]
pub async fn kol_start_gather(
    app: tauri::AppHandle,
    config: AutoGatherConfig,
    dom_config: DomConfig,
) -> Result<String, String> {
    if GATHER_RUNNING.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("Gather already running".into());
    }

    GATHER_RUNNING.store(true, std::sync::atomic::Ordering::SeqCst);

    let app_handle = app.clone();

    // Spawn background task
    tokio::spawn(async move {
        let result = run_gather_loop(&app_handle, &config, &dom_config).await;

        GATHER_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);

        if let Err(e) = result {
            let _ = app_handle.emit("kol-gather-log", serde_json::json!({
                "douyin_id": 0,
                "nickname": "系统",
                "level": "error",
                "message": format!("采集异常终止: {}", e),
            }));
        }

        let _ = app_handle.emit("kol-gather-stopped", serde_json::json!({}));
    });

    Ok("Gather started".into())
}

/// Stop the auto-gather process
#[tauri::command]
pub async fn kol_stop_gather() -> Result<String, String> {
    GATHER_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
    Ok("Gather stop requested".into())
}

/// Check if gather is currently running
#[tauri::command]
pub fn kol_is_gather_running() -> bool {
    GATHER_RUNNING.load(std::sync::atomic::Ordering::SeqCst)
}

// ============================================================
// Internal Logic
// ============================================================

async fn run_gather_loop(
    app: &tauri::AppHandle,
    config: &AutoGatherConfig,
    dom_config: &DomConfig,
) -> Result<(), String> {
    emit_log(app, 0, "系统", "info", "采集循环启动");

    for &douyin_id in &config.enabled_douyin_ids {
        if !GATHER_RUNNING.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }

        emit_log(app, douyin_id, &format!("抖音#{}", douyin_id), "info",
                  "开始采集视频");

        // Use CDP to connect to the browser profile running for this DouYin account
        // The actual browser is launched via Donut Browser's profile system
        // We connect to its debug port for DOM automation
        //
        // In the current implementation, we emit events to the frontend
        // which handles the browser interaction via the existing profile infrastructure
        app.emit("kol-gather-account", serde_json::json!({
            "douyin_id": douyin_id,
            "dom_config": dom_config,
            "interval_ms": config.interval_ms,
            "videos_per_session": config.videos_per_session,
        }))
        .map_err(|e| e.to_string())?;

        // Wait for this account's gathering to complete
        // (the frontend will emit kol-gather-account-done when finished)
        let mut attempts = 0;
        while GATHER_RUNNING.load(std::sync::atomic::Ordering::SeqCst) && attempts < 300 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            attempts += 1;
        }
    }

    emit_log(app, 0, "系统", "info", "采集循环结束");
    Ok(())
}

fn emit_log(app: &tauri::AppHandle, douyin_id: i32, nickname: &str, level: &str, message: &str) {
    let _ = app.emit("kol-gather-log", serde_json::json!({
        "douyin_id": douyin_id,
        "nickname": nickname,
        "level": level,
        "message": message,
    }));
}
