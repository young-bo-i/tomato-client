//! Douyin video gather pipeline.
//!
//! Lifecycle:
//!   start() → spawn one Worker per qualifying profile + one Uploader.
//!   Workers: launch profile → connect CDP → inject collector.js → drain
//!     `Runtime.bindingCalled` events → push VideoSubmission rows into the
//!     mpsc channel.
//!   Uploader: reads each row as it arrives and POSTs it as its own
//!     single-element request to `/api/douyin/videos/bulk`. Concurrency
//!     is capped by an UPLOAD_CONCURRENCY semaphore so a burst of 50
//!     workers can't open 50 simultaneous HTTP requests at the server;
//!     each in-flight POST runs in its own spawned task so a slow
//!     response never blocks the channel reader.
//!   stop() → signals cancel; workers tear down CDP but leave browsers
//!     open (the user owns those windows, killing them mid-session is
//!     hostile and would lose unsaved cookies).
//!
//! 50-browser scaling notes:
//! - LAUNCH_CONCURRENCY caps simultaneous Chromium spawns. Spawns are
//!   CPU-spike-heavy (process fork + fingerprint negotiation); above ~8
//!   we observe Wayfern's CDP handshake racing on under-provisioned
//!   machines. 5 is conservative.
//! - VIDEO_BLOCK_PATTERNS cuts the bulk of bandwidth/memory: Chromium
//!   never decodes the actual mp4 streams, only thumbnails and DOM data.
//! - Workers share a single mpsc upload channel feeding one Batcher, so
//!   server fan-in is bounded regardless of profile count.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Mutex as StdMutex;
use tauri::Emitter;
use tokio::sync::{mpsc, Mutex, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::browser_runner::BrowserRunner;
use crate::kol_client::{VideoSubmission, KOL_CLIENT};
use crate::profile::types::BrowserProfile;
use crate::wayfern_manager::WayfernManager;

use super::cdp::Cdp;

// ---- tunables ----------------------------------------------------------

const DOUYIN_URL: &str = "https://www.douyin.com/follow";
const COLLECTOR_JS: &str = include_str!("collector.js");
const LAUNCH_CONCURRENCY: usize = 5;
const UPLOAD_CHANNEL_CAP: usize = 4096;
/// Max simultaneous in-flight POSTs the uploader will run. Workers push
/// rows into the mpsc channel one at a time; the uploader reads them and
/// dispatches each row as its own POST, capping concurrency here so the
/// server isn't slammed when 50 profiles all surface a fresh row at once.
const UPLOAD_CONCURRENCY: usize = 16;

/// Network request URL patterns blocked at the CDP layer to prevent video
/// stream decode + cache. These match Chromium's `Network.setBlockedURLs`
/// glob format. Patterns must cover the major Douyin CDN hosts; missing
/// one means the page will silently start streaming a video and burn RAM.
const VIDEO_BLOCK_PATTERNS: &[&str] = &[
  "*.douyinvod.com/*",
  "*.amemv.com/*video*",
  "*.bytedance.com/*video*",
  "*.bdstatic.com/video*",
  "*aweme*.snssdk.com/*video*",
  "*.douyincdn.com/*",
];

const BINDING_NAME: &str = "__kolPush";

// ---- public types ------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartGatherRequest {
  /// Explicit profile UUIDs to run against. If `None`, every local
  /// profile with `kol_platform == "douyin"` is included.
  pub profile_ids: Option<Vec<Uuid>>,
  /// Per-profile cap. Worker stops collecting (but stays connected) once
  /// the count is reached. None = unlimited (until session stop).
  pub max_videos_per_profile: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatherStatus {
  pub running: bool,
  pub started_at: Option<DateTime<Local>>,
  pub uploaded_total: u64,
  pub duplicates_total: u64,
  pub upload_errors: u64,
  pub profiles: Vec<ProfileStatus>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileStatus {
  pub profile_id: Uuid,
  pub name: String,
  pub state: String, // "pending" | "launching" | "collecting" | "stopped" | "error"
  pub message: Option<String>,
  pub captured: u64,
}

// ---- internal session --------------------------------------------------

struct Inner {
  started_at: DateTime<Local>,
  cancel: CancellationToken,
  uploaded: AtomicU64,
  duplicates: AtomicU64,
  upload_errors: AtomicU64,
  profiles: StdMutex<HashMap<Uuid, ProfileStatus>>,
  /// Held only so the Drop chain reaches the batcher. Worker handles are
  /// detached — we don't await them in stop() to keep that command fast.
  _join_handles: Mutex<Vec<JoinHandle<()>>>,
}

impl Inner {
  fn set_state(&self, profile_id: Uuid, state: &str, msg: Option<String>) {
    if let Ok(mut map) = self.profiles.lock() {
      if let Some(p) = map.get_mut(&profile_id) {
        p.state = state.to_string();
        p.message = msg;
      }
    }
  }

  fn bump_captured(&self, profile_id: Uuid, n: u64) {
    if let Ok(mut map) = self.profiles.lock() {
      if let Some(p) = map.get_mut(&profile_id) {
        p.captured = p.captured.saturating_add(n);
      }
    }
  }

  fn snapshot_status(&self, running: bool) -> GatherStatus {
    let profiles = self
      .profiles
      .lock()
      .map(|m| m.values().cloned().collect::<Vec<_>>())
      .unwrap_or_default();
    GatherStatus {
      running,
      started_at: Some(self.started_at),
      uploaded_total: self.uploaded.load(Ordering::Relaxed),
      duplicates_total: self.duplicates.load(Ordering::Relaxed),
      upload_errors: self.upload_errors.load(Ordering::Relaxed),
      profiles,
    }
  }
}

static SESSION: Lazy<StdMutex<Option<Arc<Inner>>>> = Lazy::new(|| StdMutex::new(None));

// ---- public API --------------------------------------------------------

pub fn is_running() -> bool {
  SESSION.lock().map(|s| s.is_some()).unwrap_or(false)
}

pub fn current_status() -> GatherStatus {
  match SESSION.lock() {
    Ok(g) => match g.as_ref() {
      Some(inner) => inner.snapshot_status(true),
      None => empty_status(),
    },
    Err(_) => empty_status(),
  }
}

fn empty_status() -> GatherStatus {
  GatherStatus {
    running: false,
    started_at: None,
    uploaded_total: 0,
    duplicates_total: 0,
    upload_errors: 0,
    profiles: vec![],
  }
}

/// Resolve the profile set, build the session, spawn workers + batcher.
/// Returns the initial status snapshot. Errors fast on bad inputs (no
/// matching profiles, already-running session, missing creds).
pub async fn start(
  app: tauri::AppHandle,
  req: StartGatherRequest,
) -> Result<GatherStatus, String> {
  if !KOL_CLIENT.is_authenticated() {
    return Err("KOL server not logged in — submit upstream will fail".into());
  }

  // Refuse to overlap sessions. Holding the std::sync::Mutex briefly is
  // safe; we drop it before spawning anything.
  {
    let g = SESSION
      .lock()
      .map_err(|_| "session lock poisoned".to_string())?;
    if g.is_some() {
      return Err("gather session already running".into());
    }
  }

  let profiles = resolve_profiles(req.profile_ids.as_deref())?;
  if profiles.is_empty() {
    return Err("no douyin profiles matched".into());
  }
  log::info!(
    "kol_gather start: {} profile(s), max_per_profile={:?}",
    profiles.len(),
    req.max_videos_per_profile
  );

  // Build session state.
  let cancel = CancellationToken::new();
  let mut profile_status_map = HashMap::new();
  for p in &profiles {
    profile_status_map.insert(
      p.id,
      ProfileStatus {
        profile_id: p.id,
        name: p.name.clone(),
        state: "pending".into(),
        message: None,
        captured: 0,
      },
    );
  }
  let inner = Arc::new(Inner {
    started_at: Local::now(),
    cancel: cancel.clone(),
    uploaded: AtomicU64::new(0),
    duplicates: AtomicU64::new(0),
    upload_errors: AtomicU64::new(0),
    profiles: StdMutex::new(profile_status_map),
    _join_handles: Mutex::new(Vec::new()),
  });

  // Channel: workers → batcher.
  let (upload_tx, upload_rx) = mpsc::channel::<VideoSubmission>(UPLOAD_CHANNEL_CAP);

  // Spawn the batcher first so workers never block on a missing consumer.
  let uploader_inner = inner.clone();
  let uploader_app = app.clone();
  let uploader_cancel = cancel.clone();
  let uploader_handle = tokio::spawn(async move {
    uploader_loop(uploader_app, uploader_inner, upload_rx, uploader_cancel).await;
  });

  // Limit concurrent Chromium spawns to avoid CPU spike on big batches.
  let launch_sem = Arc::new(Semaphore::new(LAUNCH_CONCURRENCY));
  let max_per = req.max_videos_per_profile;

  let mut handles = vec![uploader_handle];
  for profile in profiles {
    let sem = launch_sem.clone();
    let inner_w = inner.clone();
    let upload_tx_w = upload_tx.clone();
    let app_w = app.clone();
    let cancel_w = cancel.clone();
    handles.push(tokio::spawn(async move {
      worker_run(
        app_w,
        profile,
        inner_w,
        sem,
        upload_tx_w,
        cancel_w,
        max_per,
      )
      .await;
    }));
  }
  // Drop the original sender so when all workers exit the batcher's
  // recv() returns None and it shuts down naturally.
  drop(upload_tx);

  {
    let mut store = inner._join_handles.lock().await;
    *store = handles;
  }

  // Publish the session.
  *SESSION
    .lock()
    .map_err(|_| "session lock poisoned".to_string())? = Some(inner.clone());

  let status = inner.snapshot_status(true);
  emit_status(&app, &status);
  Ok(status)
}

/// Cancel and tear down the session. Returns the final status snapshot.
/// Does not await worker completion (browsers persist after stop, the
/// next start() will find any leftover Wayfern instances and reuse them).
pub fn stop() -> Result<GatherStatus, String> {
  let prev = {
    let mut g = SESSION
      .lock()
      .map_err(|_| "session lock poisoned".to_string())?;
    g.take()
  };
  match prev {
    Some(inner) => {
      inner.cancel.cancel();
      let snap = inner.snapshot_status(false);
      log::info!(
        "kol_gather stop: uploaded={} duplicates={} errors={}",
        snap.uploaded_total,
        snap.duplicates_total,
        snap.upload_errors
      );
      Ok(snap)
    }
    None => Ok(empty_status()),
  }
}

// ---- profile resolution ------------------------------------------------

fn resolve_profiles(filter_ids: Option<&[Uuid]>) -> Result<Vec<BrowserProfile>, String> {
  let all = BrowserRunner::instance()
    .profile_manager
    .list_profiles()
    .map_err(|e| format!("list_profiles: {e}"))?;
  let filter_set: Option<std::collections::HashSet<Uuid>> =
    filter_ids.map(|s| s.iter().copied().collect());

  let picked: Vec<BrowserProfile> = all
    .into_iter()
    .filter(|p| {
      // Only wayfern profiles support our CDP-driven scrape — non-wayfern
      // browsers either lack CDP or use Firefox-style protocol.
      p.browser == "wayfern"
        && p.kol_platform.as_deref() == Some("douyin")
        && filter_set.as_ref().is_none_or(|s| s.contains(&p.id))
    })
    .collect();
  Ok(picked)
}

// ---- per-profile worker ------------------------------------------------

async fn worker_run(
  app: tauri::AppHandle,
  profile: BrowserProfile,
  inner: Arc<Inner>,
  launch_sem: Arc<Semaphore>,
  upload_tx: mpsc::Sender<VideoSubmission>,
  cancel: CancellationToken,
  max_per_profile: Option<u32>,
) {
  let pid = profile.id;
  let name = profile.name.clone();

  // Phase 1: throttled launch.
  inner.set_state(pid, "launching", None);
  emit_status(&app, &inner.snapshot_status(true));

  let permit = match launch_sem.acquire_owned().await {
    Ok(p) => p,
    Err(_) => {
      inner.set_state(pid, "error", Some("launch semaphore closed".into()));
      return;
    }
  };

  if cancel.is_cancelled() {
    inner.set_state(pid, "stopped", Some("cancelled before launch".into()));
    drop(permit);
    return;
  }

  // launch_browser_profile is the canonical entrypoint — it handles
  // proxy/VPN/state-sync and returns once Wayfern's CDP is ready.
  if let Err(e) = crate::browser_runner::launch_browser_profile(
    app.clone(),
    profile.clone(),
    Some(DOUYIN_URL.to_string()),
  )
  .await
  {
    log::error!("worker[{name}]: launch failed: {e}");
    inner.set_state(pid, "error", Some(format!("launch: {e}")));
    drop(permit);
    return;
  }

  // After launch returns, look up the CDP port WayfernManager recorded
  // when it spawned the browser process.
  let profiles_dir = BrowserRunner::instance()
    .profile_manager
    .get_profiles_dir();
  let profile_path = profile.get_profile_data_path(&profiles_dir);
  let profile_path_str = profile_path.to_string_lossy().to_string();

  let cdp_port = match WayfernManager::instance()
    .get_cdp_port(&profile_path_str)
    .await
  {
    Some(p) => p,
    None => {
      inner.set_state(pid, "error", Some("no cdp port".into()));
      drop(permit);
      return;
    }
  };

  // Release the launch slot — page-target lookup + CDP ws happen in
  // parallel with other workers' Chromium spawns.
  drop(permit);

  let ws_url = match fetch_first_page_ws(cdp_port).await {
    Ok(u) => u,
    Err(e) => {
      log::error!("worker[{name}]: cdp targets: {e}");
      inner.set_state(pid, "error", Some(format!("cdp targets: {e}")));
      return;
    }
  };

  let (cdp, mut events) = match Cdp::connect(&ws_url).await {
    Ok(v) => v,
    Err(e) => {
      log::error!("worker[{name}]: cdp connect: {e}");
      inner.set_state(pid, "error", Some(format!("cdp connect: {e}")));
      return;
    }
  };

  // Phase 2: page-side setup. Each call is best-effort — log failures but
  // continue if non-critical (e.g. setBlockedURLs failing on an old
  // Chromium build still leaves us with a working scraper, just slower).
  if let Err(e) = setup_page(&cdp).await {
    log::error!("worker[{name}]: setup: {e}");
    inner.set_state(pid, "error", Some(format!("setup: {e}")));
    cdp.close().await;
    return;
  }

  inner.set_state(pid, "collecting", None);
  emit_status(&app, &inner.snapshot_status(true));

  // Phase 3: drain events.
  let mut collected: u64 = 0;
  loop {
    tokio::select! {
      _ = cancel.cancelled() => {
        log::info!("worker[{name}]: cancelled");
        break;
      }
      ev = events.recv() => {
        let ev = match ev {
          Some(e) => e,
          None => {
            log::info!("worker[{name}]: cdp disconnected");
            break;
          }
        };
        if ev.method != "Runtime.bindingCalled" {
          continue;
        }
        let Some(name_field) = ev.params.get("name").and_then(|v| v.as_str()) else { continue };
        if name_field != BINDING_NAME { continue }
        let Some(payload) = ev.params.get("payload").and_then(|v| v.as_str()) else { continue };

        match handle_binding_payload(payload, pid, &upload_tx, &inner, &app).await {
          Ok(n) => {
            collected = collected.saturating_add(n);
            if let Some(cap) = max_per_profile {
              if collected >= cap as u64 {
                log::info!("worker[{name}]: reached cap {cap}");
                break;
              }
            }
          }
          Err(e) => log::warn!("worker[{name}]: payload err: {e}"),
        }
      }
    }
  }

  cdp.close().await;
  inner.set_state(pid, "stopped", None);
  emit_status(&app, &inner.snapshot_status(true));
}

async fn setup_page(cdp: &Cdp) -> Result<(), String> {
  cdp.call("Page.enable", json!({})).await?;
  cdp.call("Runtime.enable", json!({})).await?;
  cdp.call("Network.enable", json!({})).await?;
  // Best-effort: older Chromiums silently ignore unknown methods.
  let _ = cdp
    .call(
      "Network.setBlockedURLs",
      json!({ "urls": VIDEO_BLOCK_PATTERNS }),
    )
    .await;
  cdp
    .call("Runtime.addBinding", json!({ "name": BINDING_NAME }))
    .await?;
  cdp
    .call(
      "Page.addScriptToEvaluateOnNewDocument",
      json!({ "source": COLLECTOR_JS }),
    )
    .await?;
  // Inject into the *current* document too — the script registered above
  // only runs on subsequent navigations.
  cdp
    .call(
      "Runtime.evaluate",
      json!({ "expression": COLLECTOR_JS, "awaitPromise": false }),
    )
    .await?;
  Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum CollectorMsg {
  #[serde(rename = "video")]
  Video {
    aweme_id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    suggest_word: Option<String>,
    #[serde(default)]
    share_url: Option<String>,
    #[serde(default)]
    first_frame_url: Option<String>,
  },
  #[serde(rename = "log")]
  Log {
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    extra: Option<Value>,
  },
  #[serde(rename = "dump")]
  Dump {
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    extra: Option<Value>,
  },
}

async fn handle_binding_payload(
  payload: &str,
  profile_id: Uuid,
  upload_tx: &mpsc::Sender<VideoSubmission>,
  inner: &Arc<Inner>,
  app: &tauri::AppHandle,
) -> Result<u64, String> {
  let msg: CollectorMsg =
    serde_json::from_str(payload).map_err(|e| format!("parse: {e}"))?;
  match msg {
    CollectorMsg::Video {
      aweme_id,
      title,
      suggest_word,
      share_url,
      first_frame_url,
    } => {
      if aweme_id.is_empty() {
        return Ok(0);
      }
      // Apply the C#-compat title/suggest filter at the row's source
      // too — gather.rs is the legacy CDP-based path and should produce
      // schema-equivalent rows to the extension ingest path.
      let title_filtered = title.as_deref().and_then(super::text_filter::filter);
      let suggest_word_filtered = suggest_word
        .as_deref()
        .and_then(super::text_filter::filter);
      // Short-circuit when neither field yields a usable alias_name —
      // matches the extension-ingest path's policy of not forwarding
      // these rows. Saves a channel slot + a server round-trip.
      if title_filtered.is_none() && suggest_word_filtered.is_none() {
        return Ok(0);
      }
      let sub = VideoSubmission {
        profile_id,
        aweme_id,
        title,
        title_filtered,
        suggest_word,
        suggest_word_filtered,
        share_url,
        first_frame_url,
        captured_at: Some(Local::now()),
      };
      // Non-blocking-ish: capacity is 4096, so workers should never wedge.
      // If we do hit the cap, prefer to drop new rows over blocking the
      // CDP event reader (which would back up bindingCalled events).
      if upload_tx.try_send(sub).is_err() {
        log::warn!("upload channel full, dropping row");
        inner.upload_errors.fetch_add(1, Ordering::Relaxed);
        return Ok(0);
      }
      inner.bump_captured(profile_id, 1);
      Ok(1)
    }
    CollectorMsg::Dump { msg, extra } => {
      log::info!(
        "collector[{profile_id}] dump: {} extra={:?}",
        msg.as_deref().unwrap_or_default(),
        extra
      );
      let _ = app.emit(
        "kol-gather-dump",
        json!({ "profile_id": profile_id, "msg": msg, "extra": extra }),
      );
      Ok(0)
    }
    CollectorMsg::Log { level, msg, extra } => {
      log::info!(
        "collector[{profile_id}] {}: {} extra={:?}",
        level.as_deref().unwrap_or("info"),
        msg.as_deref().unwrap_or(""),
        extra
      );
      Ok(0)
    }
  }
}

// ---- uploader ----------------------------------------------------------
//
// Per-row upload pipeline. Reads VideoSubmission off the mpsc channel and
// dispatches each row as its own POST, one row per HTTP request. The
// reader stays hot — it acquires a permit (bounded by UPLOAD_CONCURRENCY)
// and spawns the actual HTTP work, so a slow server response never
// stalls the read end of the channel and never delays a CDP worker
// trying to push the next row in.

const UPLOAD_MAX_ATTEMPTS: u32 = 3;
const UPLOAD_RETRY_BASE_MS: u64 = 500;

async fn uploader_loop(
  app: tauri::AppHandle,
  inner: Arc<Inner>,
  mut rx: mpsc::Receiver<VideoSubmission>,
  cancel: CancellationToken,
) {
  let sem = Arc::new(Semaphore::new(UPLOAD_CONCURRENCY));
  loop {
    tokio::select! {
      _ = cancel.cancelled() => break,
      msg = rx.recv() => {
        let Some(item) = msg else { break }; // all senders dropped
        // Acquiring the permit on the reader thread is the backpressure
        // signal: when N rows are already in flight, this await pauses
        // before pulling the next row off the channel. Workers' try_send
        // remains non-blocking (channel cap = 4096) so CDP events keep
        // draining; we only choke the uploader path itself.
        let permit = match sem.clone().acquire_owned().await {
          Ok(p) => p,
          Err(_) => break, // semaphore closed (only on shutdown)
        };
        let app = app.clone();
        let inner = inner.clone();
        tokio::spawn(async move {
          let _permit = permit; // released on task exit
          upload_one(&app, &inner, item).await;
        });
      }
    }
  }
  log::info!("kol_gather uploader: exit");
}

async fn upload_one(app: &tauri::AppHandle, inner: &Arc<Inner>, item: VideoSubmission) {
  // Tiny single-element slice into the existing bulk endpoint — server
  // doesn't need to grow a separate /single route.
  let batch = std::slice::from_ref(&item);
  for attempt in 1..=UPLOAD_MAX_ATTEMPTS {
    match KOL_CLIENT.bulk_submit_douyin_videos(batch).await {
      Ok(resp) => {
        inner.uploaded.fetch_add(resp.inserted as u64, Ordering::Relaxed);
        inner.duplicates.fetch_add(resp.duplicates as u64, Ordering::Relaxed);
        emit_status(app, &inner.snapshot_status(true));
        return;
      }
      Err(e) if attempt < UPLOAD_MAX_ATTEMPTS => {
        let wait = UPLOAD_RETRY_BASE_MS * (1 << (attempt - 1));
        log::warn!(
          "kol_gather upload attempt {attempt}/{UPLOAD_MAX_ATTEMPTS} failed: {e}; retry in {wait}ms"
        );
        tokio::time::sleep(Duration::from_millis(wait)).await;
      }
      Err(e) => {
        inner.upload_errors.fetch_add(1, Ordering::Relaxed);
        log::error!("kol_gather upload failed after {UPLOAD_MAX_ATTEMPTS} attempts: {e}");
      }
    }
  }
  emit_status(app, &inner.snapshot_status(true));
}

// ---- helpers -----------------------------------------------------------

#[derive(Debug, Deserialize)]
struct PageTargetEntry {
  #[serde(rename = "type")]
  target_type: String,
  #[serde(rename = "webSocketDebuggerUrl", default)]
  ws_url: Option<String>,
  #[serde(default)]
  url: Option<String>,
}

pub(super) async fn fetch_first_page_ws(port: u16) -> Result<String, String> {
  // Bounded by both connect-only and overall timeouts so a wedged CDP
  // port doesn't hang the gather worker indefinitely on launch.
  static HTTP: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
  let client = HTTP.get_or_init(|| {
    reqwest::Client::builder()
      .timeout(std::time::Duration::from_secs(3))
      .connect_timeout(std::time::Duration::from_secs(1))
      .build()
      .expect("kol_automation::gather reqwest client init")
  });
  let url = format!("http://127.0.0.1:{port}/json");
  let resp = client
    .get(&url)
    .send()
    .await
    .map_err(|e| format!("targets fetch: {e}"))?;
  let targets: Vec<PageTargetEntry> = resp
    .json()
    .await
    .map_err(|e| format!("targets parse: {e}"))?;

  // Prefer a target whose URL hints at douyin.com — covers the case where
  // an existing instance has multiple tabs open.
  if let Some(t) = targets
    .iter()
    .find(|t| {
      t.target_type == "page"
        && t.ws_url.is_some()
        && t.url.as_deref().map(|u| u.contains("douyin")).unwrap_or(false)
    })
    .or_else(|| {
      targets
        .iter()
        .find(|t| t.target_type == "page" && t.ws_url.is_some())
    })
  {
    return Ok(t.ws_url.clone().unwrap());
  }
  Err("no page target with ws url".into())
}

fn emit_status(app: &tauri::AppHandle, status: &GatherStatus) {
  let _ = app.emit("kol-gather-status", status);
}
