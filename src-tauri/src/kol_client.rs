//! HTTP client to the tomato-server KOL backend.
//!
//! Stores the logged-in user's credentials (server URL + JWT) in a singleton
//! so any Tauri command can reach them. The React layer calls
//! `set_kol_credentials` after a successful login and `clear_kol_credentials`
//! on logout.
//!
//! **Strict online mode**: if credentials are unset or the server is
//! unreachable, profile operations return an error — there is no offline
//! fallback that serves stale data when the server is down. (A short-lived
//! 5 s profile-list cache, `PROFILE_LIST_CACHE_TTL` below, exists purely to
//! coalesce bursts of reads within a single window; it is not a fallback.)

use chrono::{DateTime, Local};
use once_cell::sync::Lazy;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use uuid::Uuid;

use crate::profile::types::BrowserProfile;

/// TTL for the in-memory profile-list cache. Hot path: the lib.rs
/// status broadcaster runs every 5 s and used to pay a full HTTP
/// round-trip per tick. With this cache, multiple callers within the
/// same window share a single fetch.
const PROFILE_LIST_CACHE_TTL: Duration = Duration::from_secs(5);

/// Dedicated tokio runtime for blocking HTTP calls. Having our own runtime
/// lets sync callers invoke async `reqwest` work without interfering with
/// Tauri's main runtime: `HTTP_RT.block_on(fut)` parks the calling thread
/// while HTTP completes on these separate worker threads. Crucially this
/// works from any thread — main-thread startup, Tauri command handlers,
/// scheduler tasks — without runtime-context panics.
static HTTP_RT: Lazy<Runtime> = Lazy::new(|| {
  tokio::runtime::Builder::new_multi_thread()
    .worker_threads(2)
    .enable_all()
    .thread_name("kol-http")
    .build()
    .expect("kol_client http runtime init")
});

#[derive(Clone, Debug)]
struct Credentials {
  server_url: String,
  token: String,
}

pub struct KolClient {
  creds: Mutex<Option<Credentials>>,
  http: reqwest::Client,
  /// (fetched_at, profiles) — None until first fetch. Invalidated on
  /// save_profile / delete_profile so write-paths see fresh data.
  profile_list_cache: Mutex<Option<(Instant, Vec<BrowserProfile>)>>,
}

impl KolClient {
  fn new() -> Self {
    Self {
      creds: Mutex::new(None),
      http: reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("reqwest client init"),
      profile_list_cache: Mutex::new(None),
    }
  }

  /// Drop the cached profile list so the next `list_profiles` does a fresh
  /// HTTP fetch. Called internally after any write, and by the batch
  /// orchestrator on lifecycle events that need an authoritative read.
  pub fn invalidate_profile_cache(&self) {
    if let Ok(mut g) = self.profile_list_cache.lock() {
      *g = None;
    }
  }

  pub fn set(&self, server_url: String, token: String) {
    let trimmed = server_url.trim_end_matches('/').to_string();
    *self.creds.lock().unwrap() = Some(Credentials {
      server_url: trimmed,
      token,
    });
  }

  pub fn clear(&self) {
    *self.creds.lock().unwrap() = None;
  }

  pub fn is_authenticated(&self) -> bool {
    self.creds.lock().unwrap().is_some()
  }

  fn creds(&self) -> Result<Credentials, String> {
    self
      .creds
      .lock()
      .unwrap()
      .clone()
      .ok_or_else(|| "not logged in to KOL server".to_string())
  }

  /// List the logged-in user's profiles. Returns an empty vec (not an
  /// error) if unauthenticated — donutbrowser's startup tasks (auto
  /// updater, sync scheduler, periodic cleanup) call this before the user
  /// has logged in, and we don't want a flood of error logs for what is
  /// just "no data yet".
  ///
  /// Cached for `PROFILE_LIST_CACHE_TTL` (5 s) so the lib.rs status
  /// broadcaster + KOL gather + profile-data-table polling don't each
  /// pay their own HTTP round-trip. Writes (save_profile / delete_profile)
  /// invalidate the cache.
  pub async fn list_profiles(&self) -> Result<Vec<BrowserProfile>, String> {
    // Fast path: cache hit within TTL.
    if let Ok(g) = self.profile_list_cache.lock() {
      if let Some((t, v)) = g.as_ref() {
        if t.elapsed() < PROFILE_LIST_CACHE_TTL {
          return Ok(v.clone());
        }
      }
    }
    let Some(c) = self.creds.lock().unwrap().clone() else {
      return Ok(vec![]);
    };
    let res = self
      .http
      .get(format!("{}/api/profiles", c.server_url))
      .bearer_auth(&c.token)
      .send()
      .await
      .map_err(|e| format!("list_profiles: {e}"))?;
    if !res.status().is_success() {
      return Err(http_err("list_profiles", res).await);
    }
    // Grab the body as text first so on parse failure we can include the
    // exact serde_json error (reqwest's own decode error swallows it).
    let body = res
      .text()
      .await
      .map_err(|e| format!("list_profiles read body: {e}"))?;
    let parsed = serde_json::from_str::<Vec<BrowserProfile>>(&body).map_err(|e| {
      let preview = body.chars().take(500).collect::<String>();
      format!("list_profiles parse: {e} | body[:500]={preview}")
    })?;
    if let Ok(mut g) = self.profile_list_cache.lock() {
      *g = Some((Instant::now(), parsed.clone()));
    }
    Ok(parsed)
  }

  /// Create-or-update. Tries PATCH first; if 404 (profile doesn't exist
  /// yet on the server) falls back to POST. Mirrors the semantics of the
  /// old `fs::write(.../metadata.json, ...)` which silently created or
  /// overwrote.
  pub async fn save_profile(&self, profile: &BrowserProfile) -> Result<(), String> {
    let c = self.creds()?;

    let patch_res = self
      .http
      .patch(format!("{}/api/profiles/{}", c.server_url, profile.id))
      .bearer_auth(&c.token)
      .json(profile)
      .send()
      .await
      .map_err(|e| format!("save_profile patch: {e}"))?;

    let result = match patch_res.status() {
      s if s.is_success() => Ok(()),
      StatusCode::NOT_FOUND => {
        let post_res = self
          .http
          .post(format!("{}/api/profiles", c.server_url))
          .bearer_auth(&c.token)
          .json(profile)
          .send()
          .await
          .map_err(|e| format!("save_profile post: {e}"))?;
        if !post_res.status().is_success() {
          return Err(http_err("save_profile create", post_res).await);
        }
        Ok(())
      }
      _ => Err(http_err("save_profile", patch_res).await),
    };
    // Any successful write makes the cache stale.
    if result.is_ok() {
      self.invalidate_profile_cache();
    }
    result
  }

  pub async fn delete_profile(&self, id: Uuid) -> Result<(), String> {
    let c = self.creds()?;
    let res = self
      .http
      .delete(format!("{}/api/profiles/{}", c.server_url, id))
      .bearer_auth(&c.token)
      .send()
      .await
      .map_err(|e| format!("delete_profile: {e}"))?;
    // Treat 404 as success — the intent was "not there anymore".
    if res.status().is_success() || res.status() == StatusCode::NOT_FOUND {
      self.invalidate_profile_cache();
      Ok(())
    } else {
      Err(http_err("delete_profile", res).await)
    }
  }

  /// Fetch the state snapshot (cookies + localStorage blob) for a profile.
  /// Returns `None` if the user isn't logged in yet (e.g. launch called
  /// during an intermediate state).
  pub async fn get_profile_state(
    &self,
    id: Uuid,
  ) -> Result<Option<GetStateResponse>, String> {
    let Some(c) = self.creds.lock().unwrap().clone() else {
      return Ok(None);
    };
    let res = self
      .http
      .get(format!("{}/api/profiles/{}/state", c.server_url, id))
      .bearer_auth(&c.token)
      .send()
      .await
      .map_err(|e| format!("get_profile_state: {e}"))?;
    match res.status() {
      s if s.is_success() => res
        .json::<GetStateResponse>()
        .await
        .map(Some)
        .map_err(|e| format!("get_profile_state parse: {e}")),
      StatusCode::NOT_FOUND => Ok(None),
      _ => Err(http_err("get_profile_state", res).await),
    }
  }

  pub async fn put_profile_state(
    &self,
    id: Uuid,
    body: &PutStateRequest,
  ) -> Result<(), String> {
    let c = self.creds()?;
    let res = self
      .http
      .put(format!("{}/api/profiles/{}/state", c.server_url, id))
      .bearer_auth(&c.token)
      .json(body)
      .send()
      .await
      .map_err(|e| format!("put_profile_state: {e}"))?;
    if res.status().is_success() {
      Ok(())
    } else {
      Err(http_err("put_profile_state", res).await)
    }
  }

  /// Bulk-upload videos scraped by the douyin gather pipeline. Server caps
  /// each request at 200 rows and dedupes on (profile_id, aweme_id) — the
  /// caller (`kol_automation::ingest::handle_bulk`) already chunks accordingly.
  pub async fn bulk_submit_douyin_videos(
    &self,
    items: &[VideoSubmission],
  ) -> Result<BulkSubmitResponse, String> {
    let c = self.creds()?;
    let res = self
      .http
      .post(format!("{}/api/douyin/videos/bulk", c.server_url))
      .bearer_auth(&c.token)
      .json(items)
      .send()
      .await
      .map_err(|e| format!("bulk_submit_douyin_videos: {e}"))?;
    if !res.status().is_success() {
      return Err(http_err("bulk_submit_douyin_videos", res).await);
    }
    res
      .json::<BulkSubmitResponse>()
      .await
      .map_err(|e| format!("bulk_submit_douyin_videos parse: {e}"))
  }

  /// Forward the douyin login state ping to the server. Server stamps
  /// the row + clears the offline_notified_at flag on transition back
  /// to authenticated. Best-effort: callers fire-and-forget; if creds
  /// aren't set yet (login in progress) we silently no-op.
  pub async fn push_douyin_state(
    &self,
    profile_id: Uuid,
    state: &str,
    url: Option<&str>,
  ) -> Result<(), String> {
    let Some(c) = self.creds.lock().unwrap().clone() else {
      return Ok(());
    };
    let body = serde_json::json!({ "state": state, "url": url });
    let res = self
      .http
      .post(format!(
        "{}/api/profiles/{}/douyin_state",
        c.server_url, profile_id
      ))
      .bearer_auth(&c.token)
      .json(&body)
      .send()
      .await
      .map_err(|e| format!("push_douyin_state: {e}"))?;
    if res.status().is_success() {
      Ok(())
    } else {
      Err(http_err("push_douyin_state", res).await)
    }
  }

  pub fn get_profile_state_blocking(
    &self,
    id: Uuid,
  ) -> Result<Option<GetStateResponse>, String> {
    run_blocking(self.get_profile_state(id))
  }

  pub fn put_profile_state_blocking(
    &self,
    id: Uuid,
    body: &PutStateRequest,
  ) -> Result<(), String> {
    run_blocking(self.put_profile_state(id, body))
  }
}

// Wire type for POST /api/douyin/videos/bulk — mirrors server's
// VideoSubmission. snake_case matches server defaults.
//
// `title_filtered` / `suggest_word_filtered` carry the result of the
// `kol_automation::text_filter` chain. `None` means the filter rejected
// the input (no rule produced a candidate that passed cleanup +
// blacklist + length checks). Server stores these as NULL columns —
// downstream `WHERE title_filtered IS NOT NULL` cleanly selects the
// "usable book name" subset.
#[derive(Debug, Serialize, Clone)]
pub struct VideoSubmission {
  pub profile_id: Uuid,
  pub aweme_id: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub title: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub title_filtered: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub suggest_word: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub suggest_word_filtered: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub share_url: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub first_frame_url: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub captured_at: Option<DateTime<Local>>,
}

#[derive(Debug, Deserialize)]
pub struct BulkSubmitResponse {
  pub inserted: i64,
  pub duplicates: i64,
}

// Wire types for /api/profiles/:id/state — mirrors the server's
// ProfileStateResponse / PutProfileStateRequest shape.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GetStateResponse {
  pub cookies: Option<serde_json::Value>,
  pub local_storage_b64: Option<String>,
  pub os_crypt_key: Option<String>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PutStateRequest {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub cookies: Option<serde_json::Value>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub local_storage_b64: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub os_crypt_key: Option<String>,
}

// ---- Sync-friendly wrappers ----
//
// Many existing Tauri commands are synchronous. `run_blocking` picks the
// right bridge depending on runtime context (see its docstring below).
impl KolClient {
  pub fn list_profiles_blocking(&self) -> Result<Vec<BrowserProfile>, String> {
    run_blocking(self.list_profiles())
  }

  pub fn save_profile_blocking(&self, profile: &BrowserProfile) -> Result<(), String> {
    run_blocking(self.save_profile(profile))
  }

  pub fn delete_profile_blocking(&self, id: Uuid) -> Result<(), String> {
    run_blocking(self.delete_profile(id))
  }
}

/// Bridge sync callers to our async HTTP client. Context-sensitive:
///
/// - Called from a tokio runtime's worker (Tauri command handlers,
///   scheduler tasks): we can't `block_on` — it would nest runtimes and
///   panic. `block_in_place` yields the worker back to the scheduler
///   while this thread blocks.
/// - Called from a plain thread (e.g. startup code on the main thread):
///   no runtime to coordinate with; run on our dedicated `HTTP_RT`.
fn run_blocking<F: std::future::Future>(fut: F) -> F::Output {
  match tokio::runtime::Handle::try_current() {
    Ok(handle) => tokio::task::block_in_place(|| handle.block_on(fut)),
    Err(_) => HTTP_RT.block_on(fut),
  }
}

async fn http_err(ctx: &str, res: reqwest::Response) -> String {
  let status = res.status();
  let body = res.text().await.unwrap_or_default();
  let snippet = if body.len() > 200 { &body[..200] } else { &body };
  format!("{ctx}: {status} {snippet}")
}

pub static KOL_CLIENT: Lazy<KolClient> = Lazy::new(KolClient::new);

// ---- Tauri commands exposed to the React side ----

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetKolCredsArgs {
  pub server_url: String,
  pub token: String,
}

#[derive(Debug, Serialize)]
pub struct KolStatus {
  pub authenticated: bool,
}

#[tauri::command]
pub fn set_kol_credentials(args: SetKolCredsArgs) -> Result<KolStatus, String> {
  if args.server_url.trim().is_empty() || args.token.trim().is_empty() {
    return Err("server_url and token required".into());
  }
  KOL_CLIENT.set(args.server_url, args.token);
  log::info!("KOL credentials set");
  Ok(KolStatus { authenticated: true })
}

#[tauri::command]
pub fn clear_kol_credentials() -> KolStatus {
  KOL_CLIENT.clear();
  log::info!("KOL credentials cleared");
  KolStatus { authenticated: false }
}

#[tauri::command]
pub fn kol_auth_status() -> KolStatus {
  KolStatus {
    authenticated: KOL_CLIENT.is_authenticated(),
  }
}
