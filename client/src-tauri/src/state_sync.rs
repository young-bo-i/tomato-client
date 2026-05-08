//! Browser profile runtime-state sync (cookies, localStorage, Chromium
//! cookie-encryption key).
//!
//! Flow:
//! - **Before launch**: GET `/api/profiles/:id/state`. Restore the
//!   Chromium `os_crypt_key` file first (so cookie encryption roundtrips
//!   cleanly), then inject the cookie list, then extract the
//!   localStorage tarball.
//! - **After close**: extract cookies (plaintext, decrypted via the key),
//!   read the key file, tar.gz the localStorage directory, PUT the
//!   bundle back to the server.
//!
//! Failures are logged as warnings and do NOT block browser launch or
//! cleanup — a network hiccup should not prevent the user from using
//! their browser.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use once_cell::sync::Lazy;
use tar::{Archive, Builder};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::cookie_manager::{CookieManager, UnifiedCookie};
use crate::kol_client::{PutStateRequest, KOL_CLIENT};
use crate::profile::manager::ProfileManager;

const PERIODIC_INTERVAL: Duration = Duration::from_secs(30);

/// Active periodic-push tasks keyed by profile id. We hold the
/// JoinHandle so we can `abort()` it when the browser is stopped.
static PERIODIC_TASKS: Lazy<Mutex<HashMap<Uuid, JoinHandle<()>>>> =
  Lazy::new(|| Mutex::new(HashMap::new()));

const OS_CRYPT_KEY_FILENAME: &str = "os_crypt_key";
const B64: base64::engine::general_purpose::GeneralPurpose =
  base64::engine::general_purpose::STANDARD;

/// Resolved paths + browser-specific metadata for a profile.
struct ProfileCtx {
  browser: String,
  profile_data_path: PathBuf,
}

fn resolve_profile_ctx(profile_id: Uuid) -> Option<ProfileCtx> {
  let pm = ProfileManager::instance();
  let profiles = pm.list_profiles().ok()?;
  let profile = profiles.into_iter().find(|p| p.id == profile_id)?;
  let profiles_dir = pm.get_profiles_dir();
  Some(ProfileCtx {
    browser: profile.browser.clone(),
    profile_data_path: profile.get_profile_data_path(&profiles_dir),
  })
}

fn local_storage_rel_paths(browser: &str) -> &'static [&'static str] {
  match browser {
    // Chromium-based: leveldb holds all localStorage for Chromium.
    "wayfern" => &["Default/Local Storage"],
    // Firefox-based: localStorage modern path (per-origin ls/) lives
    // under storage/default. webappsstore.sqlite is the legacy fallback.
    "camoufox" => &["storage/default", "webappsstore.sqlite"],
    _ => &[],
  }
}

/// Bundle one or more relative paths (files or directories) under `base`
/// into a single tar.gz byte stream. Entries that don't exist are simply
/// skipped. Returns None if no entries produced any bytes.
fn tar_gzip_relative(base: &Path, rels: &[&str]) -> Option<Vec<u8>> {
  let buffer: Vec<u8> = Vec::new();
  let encoder = GzEncoder::new(buffer, Compression::default());
  let mut builder = Builder::new(encoder);
  builder.follow_symlinks(false);

  let mut any = false;
  for rel in rels {
    let full = base.join(rel);
    if !full.exists() {
      continue;
    }
    let res = if full.is_dir() {
      builder.append_dir_all(rel, &full)
    } else {
      let mut file = match std::fs::File::open(&full) {
        Ok(f) => f,
        Err(e) => {
          log::warn!("state_sync: tar skip {rel}: {e}");
          continue;
        }
      };
      builder.append_file(rel, &mut file)
    };
    if let Err(e) = res {
      log::warn!("state_sync: tar append {rel}: {e}");
      continue;
    }
    any = true;
  }

  if !any {
    return None;
  }
  let encoder = builder.into_inner().ok()?;
  encoder.finish().ok()
}

/// Unpack a tar.gz byte stream under `base`. Before extracting, any of
/// the top-level `rels` that currently exist under `base` are removed so
/// the restore is a full replace rather than a merge.
fn untar_gzip_replacing(base: &Path, rels: &[&str], bytes: &[u8]) -> Result<(), String> {
  for rel in rels {
    let full = base.join(rel);
    if full.is_dir() {
      std::fs::remove_dir_all(&full).map_err(|e| format!("remove {rel}: {e}"))?;
    } else if full.exists() {
      std::fs::remove_file(&full).map_err(|e| format!("remove {rel}: {e}"))?;
    }
  }
  std::fs::create_dir_all(base).map_err(|e| format!("mkdir base: {e}"))?;
  let decoder = GzDecoder::new(bytes);
  let mut archive = Archive::new(decoder);
  archive
    .unpack(base)
    .map_err(|e| format!("untar: {e}"))?;
  Ok(())
}

// ---- Public entry points ----

/// Pull the server's latest state snapshot and apply it to the local
/// profile. Called right before `launch_browser_profile` starts the
/// actual browser process.
pub async fn pull_before_launch(profile_id: Uuid) {
  let state = match KOL_CLIENT.get_profile_state(profile_id).await {
    Ok(Some(s)) => s,
    Ok(None) => {
      log::debug!("state_sync: no server state for profile {profile_id}");
      return;
    }
    Err(e) => {
      log::warn!("state_sync: pull failed for {profile_id}: {e}");
      return;
    }
  };

  let ctx = match resolve_profile_ctx(profile_id) {
    Some(c) => c,
    None => {
      log::warn!("state_sync: profile {profile_id} not found locally");
      return;
    }
  };

  // Do all disk work on the blocking pool — file I/O + SQLite + tar/gzip
  // each independently could stall the tokio scheduler.
  let result = tokio::task::spawn_blocking(move || {
    // 1. Restore os_crypt_key BEFORE injecting cookies so both sides use
    //    the same encryption key.
    if let Some(key_contents) = state.os_crypt_key.as_ref() {
      let path = ctx.profile_data_path.join(OS_CRYPT_KEY_FILENAME);
      if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
          log::warn!("state_sync: mkdir for os_crypt_key: {e}");
        }
      }
      match std::fs::write(&path, key_contents.as_bytes()) {
        Ok(()) => log::info!("state_sync: restored os_crypt_key for {profile_id}"),
        Err(e) => log::warn!("state_sync: os_crypt_key write failed: {e}"),
      }
    }

    // 2. Inject cookies (re-encrypted with the key we just restored).
    if let Some(cookies_json) = state.cookies {
      match serde_json::from_value::<Vec<UnifiedCookie>>(cookies_json) {
        Ok(cookies) => {
          let count = cookies.len();
          match CookieManager::write_cookies_to_profile(&profile_id.to_string(), &cookies) {
            Ok((added, replaced)) => log::info!(
              "state_sync: injected {count} cookies into {profile_id} (added {added}, replaced {replaced})"
            ),
            Err(e) => log::warn!("state_sync: cookie write failed: {e}"),
          }
        }
        Err(e) => log::warn!("state_sync: cookies JSON malformed: {e}"),
      }
    }

    // 3. Extract localStorage tarball (replaces any existing paths).
    if let Some(ls_b64) = state.local_storage_b64.as_ref() {
      match B64.decode(ls_b64) {
        Ok(bytes) => {
          let rels = local_storage_rel_paths(&ctx.browser);
          if !rels.is_empty() {
            match untar_gzip_replacing(&ctx.profile_data_path, rels, &bytes) {
              Ok(()) => log::info!(
                "state_sync: restored localStorage for {profile_id} ({} bytes gz)",
                bytes.len()
              ),
              Err(e) => log::warn!("state_sync: localStorage untar failed: {e}"),
            }
          }
        }
        Err(e) => log::warn!("state_sync: localStorage base64 decode: {e}"),
      }
    }
  })
  .await;
  if let Err(e) = result {
    log::warn!("state_sync: pull task panicked: {e}");
  }
}

/// After the browser has exited, read the final cookie state + cookie
/// encryption key + localStorage dir and push everything to the server.
pub async fn push_after_close(profile_id: Uuid) {
  let ctx = match resolve_profile_ctx(profile_id) {
    Some(c) => c,
    None => {
      log::warn!("state_sync: profile {profile_id} not found locally for push");
      return;
    }
  };

  // All of this is disk/SQLite work — run on the blocking pool.
  let ctx_clone = ProfileCtx {
    browser: ctx.browser.clone(),
    profile_data_path: ctx.profile_data_path.clone(),
  };
  let collect_res = tokio::task::spawn_blocking(move || collect_state(profile_id, &ctx_clone)).await;

  let (cookies, os_crypt_key, local_storage_b64) = match collect_res {
    Ok(tup) => tup,
    Err(e) => {
      log::warn!("state_sync: collect task panicked: {e}");
      return;
    }
  };

  // Skip the PUT entirely if nothing to send.
  if cookies.is_none() && os_crypt_key.is_none() && local_storage_b64.is_none() {
    log::debug!("state_sync: nothing to push for {profile_id}");
    return;
  }

  let cookie_count = cookies.as_ref().map(Vec::len).unwrap_or(0);
  let ls_bytes = local_storage_b64.as_ref().map(String::len).unwrap_or(0);

  let body = PutStateRequest {
    cookies: cookies.map(|c| serde_json::json!(c)),
    local_storage_b64,
    os_crypt_key,
  };

  match KOL_CLIENT.put_profile_state(profile_id, &body).await {
    Ok(()) => log::info!(
      "state_sync: pushed profile {profile_id}: {cookie_count} cookies, localStorage b64={ls_bytes}"
    ),
    Err(e) => log::warn!("state_sync: push failed for {profile_id}: {e}"),
  }
}

/// Read all local state off disk and return it ready-to-PUT. Each piece
/// is independently optional — a fresh profile may have no cookies, no
/// key, or no localStorage.
fn collect_state(
  profile_id: Uuid,
  ctx: &ProfileCtx,
) -> (Option<Vec<UnifiedCookie>>, Option<String>, Option<String>) {
  let cookies = match CookieManager::read_cookies(&profile_id.to_string()) {
    Ok(result) => Some(
      result
        .domains
        .into_iter()
        .flat_map(|d| d.cookies)
        .collect::<Vec<_>>(),
    ),
    Err(e) => {
      log::warn!("state_sync: read cookies failed for {profile_id}: {e}");
      None
    }
  };

  let os_crypt_key = {
    let path = ctx.profile_data_path.join(OS_CRYPT_KEY_FILENAME);
    match std::fs::read_to_string(&path) {
      Ok(s) => Some(s),
      Err(_) => None,
    }
  };

  let rels = local_storage_rel_paths(&ctx.browser);
  let local_storage_b64 = if rels.is_empty() {
    None
  } else {
    tar_gzip_relative(&ctx.profile_data_path, rels).map(|bytes| B64.encode(&bytes))
  };

  (cookies, os_crypt_key, local_storage_b64)
}

// ---- Periodic state push while the browser is running ----
//
// Cookies and the os_crypt_key file can be safely read while Chromium
// is running (Cookies SQLite is in WAL mode; the key file is read-only
// after first launch). localStorage is LevelDB which the browser holds
// open exclusively, so we skip it in the periodic path — that's only
// captured at close.

/// Spawn a per-profile timer that pushes cookies + os_crypt_key every
/// `PERIODIC_INTERVAL`. Idempotent — calling twice for the same profile
/// replaces the existing timer.
pub fn start_periodic_push(profile_id: Uuid) {
  stop_periodic_push(profile_id);
  let handle = tokio::spawn(async move {
    let mut interval = tokio::time::interval(PERIODIC_INTERVAL);
    // Skip the immediate first tick — the launch hook just pulled state
    // milliseconds ago, no point pushing it right back.
    interval.tick().await;
    loop {
      interval.tick().await;
      push_cookies_only(profile_id).await;
    }
  });
  PERIODIC_TASKS
    .lock()
    .unwrap()
    .insert(profile_id, handle);
  log::info!("state_sync: started periodic push for {profile_id} (every 30s)");
}

/// Cancel a running periodic-push task. Safe to call when no task is
/// active. Should be called at the start of `kill_browser_profile`
/// before the final `push_after_close` so the periodic push doesn't
/// race against the browser shutdown sequence.
pub fn stop_periodic_push(profile_id: Uuid) {
  if let Some(handle) = PERIODIC_TASKS.lock().unwrap().remove(&profile_id) {
    handle.abort();
    log::info!("state_sync: stopped periodic push for {profile_id}");
  }
}

/// One iteration of the periodic loop: read cookies + key, push to
/// server. Skips the localStorage tarball for safety reasons (see
/// module-level comment).
async fn push_cookies_only(profile_id: Uuid) {
  let ctx = match resolve_profile_ctx(profile_id) {
    Some(c) => c,
    None => return,
  };

  let collect_res = tokio::task::spawn_blocking(move || {
    let cookies = match CookieManager::read_cookies(&profile_id.to_string()) {
      Ok(r) => Some(
        r.domains
          .into_iter()
          .flat_map(|d| d.cookies)
          .collect::<Vec<_>>(),
      ),
      Err(e) => {
        log::warn!("state_sync: periodic read cookies failed for {profile_id}: {e}");
        None
      }
    };
    let os_crypt_key =
      std::fs::read_to_string(ctx.profile_data_path.join(OS_CRYPT_KEY_FILENAME)).ok();
    (cookies, os_crypt_key)
  })
  .await;

  let (cookies, os_crypt_key) = match collect_res {
    Ok(t) => t,
    Err(e) => {
      log::warn!("state_sync: periodic collect panicked: {e}");
      return;
    }
  };

  if cookies.is_none() && os_crypt_key.is_none() {
    return;
  }

  let cookie_count = cookies.as_ref().map(Vec::len).unwrap_or(0);
  let body = PutStateRequest {
    cookies: cookies.map(|c| serde_json::json!(c)),
    local_storage_b64: None,
    os_crypt_key,
  };

  match KOL_CLIENT.put_profile_state(profile_id, &body).await {
    Ok(()) => log::debug!(
      "state_sync: periodic push for {profile_id} ({cookie_count} cookies)"
    ),
    Err(e) => log::warn!("state_sync: periodic push failed for {profile_id}: {e}"),
  }
}
