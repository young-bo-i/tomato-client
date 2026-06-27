//! Bridge between the in-browser KOL helper extension and the
//! tomato-server. Receives `POST /kol-ext/gather/bulk` from the
//! extension's service worker, forwards rows to the remote server via
//! `KOL_CLIENT.bulk_submit_douyin_videos`, and tallies running totals
//! for the panel UI.
//!
//! Counters are process-global (Tauri client = one user). They survive
//! browser-window close but reset when Donut itself restarts; that's
//! fine because the durable state lives on the tomato-server.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::Json;
use chrono::{DateTime, Local};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api_server;
use crate::kol_client::{VideoSubmission, KOL_CLIENT};

use super::batch;
use super::dedup;
use super::text_filter;

/// Match server's BULK cap. Extension batches at 50 by default, well
/// under this — but defensively reject anything larger to avoid
/// surprising the remote server.
const MAX_BATCH: usize = 200;

#[derive(Debug, Default)]
pub struct LocalStats {
  pub batches_received: AtomicU64,
  pub rows_received: AtomicU64,
  pub uploaded: AtomicU64,
  pub duplicates: AtomicU64,
  pub upload_errors: AtomicU64,
  /// Rows skipped by the local 24h dedup cache before forwarding.
  /// Tracks how many remote requests we saved.
  pub dedup_skipped: AtomicU64,
}

pub static STATS: Lazy<LocalStats> = Lazy::new(LocalStats::default);

/// Per-profile Douyin login state, populated by content.js pings.
///
/// Three values mirror the JS detector:
/// - `authenticated`: feed visible, content script may collect.
/// - `unauthenticated`: page loaded but no feed (logged out, or session
///   was invalidated by another device — Douyin renders both states
///   identically structurally).
/// - `unknown`: SPA still loading, can't decide yet.
///
/// The map is keyed by profile_id (the same UUID baked into each
/// extension's profile.json). Entries are overwritten on each ping;
/// missing entries mean the extension hasn't reported yet (just-launched
/// profile or content.js not yet attached).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileLoginState {
  pub state: String,
  pub updated_at: DateTime<Local>,
  pub url: Option<String>,
}

pub static PROFILE_STATES: Lazy<Mutex<HashMap<Uuid, ProfileLoginState>>> =
  Lazy::new(|| Mutex::new(HashMap::new()));

/// Snapshot of a single profile's login state. None if no ping yet.
pub fn get_login_state(profile_id: Uuid) -> Option<ProfileLoginState> {
  PROFILE_STATES.lock().ok()?.get(&profile_id).cloned()
}

/// JSON shape of one row from the extension. Matches `VideoSubmission`
/// field names (snake_case).
#[derive(Debug, Deserialize)]
pub struct ExtensionRow {
  pub profile_id: Uuid,
  pub aweme_id: String,
  #[serde(default)]
  pub title: Option<String>,
  #[serde(default)]
  pub suggest_word: Option<String>,
  #[serde(default)]
  pub share_url: Option<String>,
  #[serde(default)]
  pub first_frame_url: Option<String>,
  #[serde(default)]
  pub captured_at: Option<DateTime<Local>>,
}

#[derive(Debug, Serialize)]
pub struct BulkResponse {
  pub inserted: i64,
  pub duplicates: i64,
  pub forwarded: usize,
}

#[derive(Debug, Deserialize)]
pub struct StatePing {
  pub profile_id: Uuid,
  pub state: String,
  #[serde(default)]
  pub url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StatePingResponse {
  pub ok: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StatsSnapshot {
  pub batches_received: u64,
  pub rows_received: u64,
  pub uploaded: u64,
  pub duplicates: u64,
  pub upload_errors: u64,
  pub dedup_skipped: u64,
}

/// Idempotently ensure Donut's local axum server is running so the
/// in-browser KOL extension can POST to `/kol-ext/gather/bulk`.
///
/// Donut by default leaves `settings.api_enabled = false` and does NOT
/// start the server at boot. The KOL helper extension has nothing to
/// talk to in that case, so the gather pipeline silently fails (the
/// browser slides happily but the database stays empty). We call this
/// from the douyin-profile launch path: if the server is already up,
/// return its port; otherwise start it on 10108 (with the api_server's
/// own random fallback if 10108 is taken — extension probes 10108..12).
///
/// This intentionally bypasses `settings.api_enabled` because the
/// extension treats the local server as an internal IPC channel, not a
/// publicly exposed API. The /v1 routes still require Bearer auth and
/// /kol-ext is gated by Origin: chrome-extension://, so behaviour for
/// other callers is unchanged.
pub async fn ensure_api_server_started(
  app_handle: &tauri::AppHandle,
) -> Result<u16, String> {
  if let Ok(Some(p)) = api_server::get_api_server_status().await {
    return Ok(p);
  }
  log::info!("kol-ext: starting local axum server on 10108 for ingest");
  api_server::start_api_server_internal(10108, app_handle).await
}

pub fn snapshot_stats() -> StatsSnapshot {
  StatsSnapshot {
    batches_received: STATS.batches_received.load(Ordering::Relaxed),
    rows_received: STATS.rows_received.load(Ordering::Relaxed),
    uploaded: STATS.uploaded.load(Ordering::Relaxed),
    duplicates: STATS.duplicates.load(Ordering::Relaxed),
    upload_errors: STATS.upload_errors.load(Ordering::Relaxed),
    dedup_skipped: STATS.dedup_skipped.load(Ordering::Relaxed),
  }
}

/// `POST /kol-ext/gather/bulk` handler. Accepts the raw body as bytes
/// (the extension may post with `Content-Type: text/plain` to avoid
/// triggering CORS preflight) and parses it as JSON manually. Converts
/// rows to `VideoSubmission`, calls `KOL_CLIENT.bulk_submit_douyin_videos`,
/// returns `{inserted, duplicates, forwarded}`.
pub async fn handle_bulk(
  raw: Bytes,
) -> Result<Json<BulkResponse>, (StatusCode, String)> {
  let body: Vec<ExtensionRow> = serde_json::from_slice(&raw)
    .map_err(|e| (StatusCode::BAD_REQUEST, format!("parse json: {e}")))?;
  if body.is_empty() {
    return Ok(Json(BulkResponse {
      inserted: 0,
      duplicates: 0,
      forwarded: 0,
    }));
  }
  if body.len() > MAX_BATCH {
    return Err((
      StatusCode::PAYLOAD_TOO_LARGE,
      format!("max {MAX_BATCH} rows per batch, got {}", body.len()),
    ));
  }

  STATS.batches_received.fetch_add(1, Ordering::Relaxed);
  STATS
    .rows_received
    .fetch_add(body.len() as u64, Ordering::Relaxed);

  let now = Local::now();
  let mut subs: Vec<VideoSubmission> = Vec::with_capacity(body.len());
  let mut skipped_local: u64 = 0;
  for r in body {
    // Apply the C#-compat filter chain to both title and suggest
    // word. Empty results are stored as None (NULL on the server).
    let title_filtered = r.title.as_deref().and_then(text_filter::filter);
    let suggest_word_filtered = r
      .suggest_word
      .as_deref()
      .and_then(text_filter::filter);

    // Short-circuit when neither field passed the 4-8 char filter —
    // there's no usable alias_name for this row, so the server has
    // nothing to do with it. Don't forward + don't store; the user
    // explicitly opted out of keeping these for now.
    if title_filtered.is_none() && suggest_word_filtered.is_none() {
      skipped_local += 1;
      continue;
    }

    // Local 24h dedup gate, keyed on (profile_id, aweme_id): skip a row
    // the same profile already saw within TTL. Saves a remote round trip
    // and a server INSERT attempt. Cross-profile sightings are kept (each
    // profile owns its own douyin_videos row).
    if dedup::check_and_record(r.profile_id, &r.aweme_id) {
      skipped_local += 1;
      continue;
    }

    subs.push(VideoSubmission {
      profile_id: r.profile_id,
      aweme_id: r.aweme_id,
      title: r.title,
      title_filtered,
      suggest_word: r.suggest_word,
      suggest_word_filtered,
      share_url: r.share_url,
      first_frame_url: r.first_frame_url,
      captured_at: Some(r.captured_at.unwrap_or(now)),
    });
  }
  if skipped_local > 0 {
    STATS
      .dedup_skipped
      .fetch_add(skipped_local, Ordering::Relaxed);
  }
  let forwarded = subs.len();
  if forwarded == 0 {
    return Ok(Json(BulkResponse {
      inserted: 0,
      duplicates: 0,
      forwarded: 0,
    }));
  }

  match KOL_CLIENT.bulk_submit_douyin_videos(&subs).await {
    Ok(resp) => {
      STATS
        .uploaded
        .fetch_add(resp.inserted as u64, Ordering::Relaxed);
      STATS
        .duplicates
        .fetch_add(resp.duplicates as u64, Ordering::Relaxed);
      Ok(Json(BulkResponse {
        inserted: resp.inserted,
        duplicates: resp.duplicates,
        forwarded,
      }))
    }
    Err(e) => {
      STATS.upload_errors.fetch_add(1, Ordering::Relaxed);
      log::error!("kol-ext bulk forward failed: {e}");
      Err((StatusCode::BAD_GATEWAY, format!("forward: {e}")))
    }
  }
}

/// `POST /kol-ext/state` — content.js reports per-profile Douyin login
/// state when it changes (and also on first attach). Stored in
/// PROFILE_STATES for the panel UI to surface and consumed by the
/// batch unauth-auto-close watchdog. Body is plain JSON bytes (no
/// Content-Type pinned, same convention as `/gather/bulk`).
pub async fn handle_state(
  axum::extract::State(state): axum::extract::State<crate::api_server::ApiServerState>,
  raw: Bytes,
) -> Result<Json<StatePingResponse>, (StatusCode, String)> {
  let app_handle = state.app_handle.clone();
  let body: StatePing = serde_json::from_slice(&raw)
    .map_err(|e| (StatusCode::BAD_REQUEST, format!("parse json: {e}")))?;

  // Validate state value — keep the set tight so typos don't pollute.
  match body.state.as_str() {
    "authenticated" | "unauthenticated" | "unknown" => {}
    other => {
      return Err((
        StatusCode::BAD_REQUEST,
        format!("unknown state: {other}"),
      ))
    }
  }

  let entry = ProfileLoginState {
    state: body.state.clone(),
    updated_at: Local::now(),
    url: body.url.clone(),
  };

  let mut state_changed = false;
  if let Ok(mut map) = PROFILE_STATES.lock() {
    let prev = map.get(&body.profile_id).map(|p| p.state.clone());
    if prev.as_deref() != Some(body.state.as_str()) {
      state_changed = true;
      log::info!(
        "kol-ext profile {} login state: {:?} -> {}",
        body.profile_id,
        prev,
        body.state
      );
    }
    map.insert(body.profile_id, entry);
  }

  // On state transitions, forward to the server so the
  // notification_dispatcher (server-side) can email admins about
  // douyin offlines. Fire-and-forget: a transient server hiccup
  // shouldn't block the watchdog logic below, and the next state
  // change will retry naturally.
  if state_changed {
    let pid = body.profile_id;
    let st = body.state.clone();
    let url = body.url.clone();
    tokio::spawn(async move {
      if let Err(e) = crate::kol_client::KOL_CLIENT
        .push_douyin_state(pid, &st, url.as_deref())
        .await
      {
        log::warn!("kol-ext push_douyin_state {pid}: {e}");
      }
    });
  }

  // Batch unauth watchdog: in batch sessions, a profile that stays
  // logged-out for too long is closed to free up resources.
  match body.state.as_str() {
    "unauthenticated" => {
      if batch::note_unauth_state(body.profile_id) {
        let app = app_handle.clone();
        let pid = body.profile_id;
        tokio::spawn(async move { batch::kill_unauth_profile(app, pid).await });
      }
    }
    _ => batch::clear_unauth_marker(body.profile_id),
  }

  Ok(Json(StatePingResponse { ok: true }))
}

/// `GET /kol-ext/gather/should?profile_id=<uuid>` — content.js polls
/// this every few seconds to discover whether it should be gathering.
/// Mirrors the value flipped by `batch::kol_batch_{start,stop}`.
#[derive(Debug, Deserialize)]
pub struct ShouldQuery {
  pub profile_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct ShouldResponse {
  pub should_gather: bool,
}

pub async fn handle_should(
  axum::extract::Query(q): axum::extract::Query<ShouldQuery>,
) -> Json<ShouldResponse> {
  Json(ShouldResponse {
    should_gather: batch::read_should_gather(q.profile_id),
  })
}
