//! Local 24h dedup cache.
//!
//! Sits between `ingest::handle_bulk` and the remote tomato-server.
//! Skips forwarding a row if the per-profile aweme key was seen within
//! the last 24 hours:
//!
//! `aweme:{profile_id}:{aweme_id}` — same profile re-sees the same
//! video within a day. Server's `UNIQUE(profile_id, aweme_id)` would
//! catch it too, but skipping locally saves a network round trip
//! and a tomato-server INSERT attempt per row.
//!
//! Note: we intentionally do NOT dedup on filtered title/suggest words
//! globally. The douyin_videos table records per-account ownership
//! (same video seen by different profiles = different rows), and the
//! server's alias insertion is already idempotent via
//! `ON CONFLICT (alias_name, alias_type) DO NOTHING`.
//!
//! Crash/restart-safe: the live `HashMap` is mirrored to a JSON file
//! at `<app-data>/kol-dedup-cache.json` every 60s and on shutdown via
//! `flush_now()`. On boot we load it back + drop expired entries.
//!
//! Performance: per-row cost is 1 hashmap lookup + at most 1 insert.
//! At 200 rows/sec aggregate (50 profiles × ~4 rows/sec each), this
//! runs in single-digit microseconds against a ~70k-entry map. The
//! file flush runs on a spawned blocking task so it never blocks the
//! tokio worker threads.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Local};
use once_cell::sync::Lazy;
use uuid::Uuid;

const TTL_HOURS: i64 = 24;
const FLUSH_INTERVAL: Duration = Duration::from_secs(60);

static CACHE: Lazy<Mutex<HashMap<String, DateTime<Local>>>> =
  Lazy::new(|| Mutex::new(HashMap::new()));

fn cache_path() -> PathBuf {
  crate::app_dirs::data_dir().join("kol-dedup-cache.json")
}

/// Synchronous load from disk. Call once at app boot, before any
/// `handle_bulk` is allowed to run, so the first batch already sees a
/// warm cache. Drops entries whose `expires_at` is already past.
pub fn load_from_disk() {
  let path = cache_path();
  let raw = match std::fs::read_to_string(&path) {
    Ok(s) => s,
    Err(_) => return, // first boot, no file
  };
  let parsed: HashMap<String, DateTime<Local>> = match serde_json::from_str(&raw) {
    Ok(m) => m,
    Err(e) => {
      log::warn!("kol-dedup load: parse failed: {e}; starting empty");
      return;
    }
  };
  let now = Local::now();
  let kept: HashMap<_, _> = parsed.into_iter().filter(|(_, exp)| *exp > now).collect();
  log::info!(
    "kol-dedup loaded {} live entries from {}",
    kept.len(),
    path.display()
  );
  if let Ok(mut g) = CACHE.lock() {
    *g = kept;
  }
}

/// Atomic flush. Serializes the cache, then offloads the blocking file
/// write + rename to a `spawn_blocking` task so the tokio worker thread
/// is never parked on disk IO (~5 MB at saturation).
fn flush_to_disk() {
  let snapshot: HashMap<String, DateTime<Local>> = match CACHE.lock() {
    Ok(g) => g.clone(),
    Err(_) => return,
  };
  let path = cache_path();
  let json = match serde_json::to_string(&snapshot) {
    Ok(s) => s,
    Err(e) => {
      log::warn!("kol-dedup flush: serialize failed: {e}");
      return;
    }
  };
  tokio::task::spawn_blocking(move || {
    let tmp = path.with_extension("json.tmp");
    if let Some(parent) = path.parent() {
      let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&tmp, &json) {
      log::warn!("kol-dedup flush: write tmp failed: {e}");
      return;
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
      log::warn!("kol-dedup flush: rename failed: {e}");
    }
  });
}

/// Drop expired entries in place. O(n) walk but n stays bounded by
/// (rows_per_day × TTL_HOURS / 24) which is the daily volume.
fn evict_expired() {
  let now = Local::now();
  if let Ok(mut g) = CACHE.lock() {
    g.retain(|_, exp| *exp > now);
  }
}

/// Periodic GC + flush task. Spawn once at app startup.
pub async fn start_flush_loop() {
  let mut tick = tokio::time::interval(FLUSH_INTERVAL);
  tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
  // First tick fires immediately — skip it, no point flushing an
  // empty cache milliseconds after boot.
  tick.tick().await;
  loop {
    tick.tick().await;
    evict_expired();
    flush_to_disk();
  }
}

/// Force-flush for shutdown. Writes synchronously (no spawn) because
/// the tokio runtime may already be tearing down at call time.
pub fn flush_now() {
  evict_expired();
  let snapshot: HashMap<String, DateTime<Local>> = match CACHE.lock() {
    Ok(g) => g.clone(),
    Err(_) => return,
  };
  let path = cache_path();
  let json = match serde_json::to_string(&snapshot) {
    Ok(s) => s,
    Err(e) => { log::warn!("kol-dedup flush_now: serialize: {e}"); return; }
  };
  let tmp = path.with_extension("json.tmp");
  if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
  if let Err(e) = std::fs::write(&tmp, &json) {
    log::warn!("kol-dedup flush_now: write: {e}"); return;
  }
  if let Err(e) = std::fs::rename(&tmp, &path) {
    log::warn!("kol-dedup flush_now: rename: {e}");
  }
}

/// Test if a row should be SKIPPED (same profile re-seeing the same
/// aweme within 24h). On miss, records the key and returns `false`.
///
/// `title_filtered` and `suggest_filtered` are accepted for signature
/// compatibility but are no longer used for dedup — cross-profile
/// ownership is preserved in `douyin_videos` and alias dedup is
/// handled server-side via `ON CONFLICT DO NOTHING`.
pub fn check_and_record(
  profile_id: Uuid,
  aweme_id: &str,
  _title_filtered: Option<&str>,
  _suggest_filtered: Option<&str>,
) -> bool {
  let now = Local::now();
  let expires_at = now + chrono::Duration::hours(TTL_HOURS);
  let aweme_key = format!("aweme:{profile_id}:{aweme_id}");

  let mut cache = match CACHE.lock() {
    Ok(g) => g,
    Err(_) => return false, // poisoned: fail open so ingest continues
  };

  if cache.get(&aweme_key).map(|exp| *exp > now).unwrap_or(false) {
    return true;
  }

  cache.insert(aweme_key, expires_at);
  false
}

/// Test-only inspect. Returns approximate size; not snapshot-stable.
#[allow(dead_code)]
pub fn approximate_size() -> usize {
  CACHE.lock().map(|g| g.len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn miss_then_hit_aweme() {
    let pid = Uuid::new_v4();
    assert!(!check_and_record(pid, "v1", None, None));
    assert!(check_and_record(pid, "v1", None, None));
  }

  #[test]
  fn different_profile_same_aweme_not_deduped_by_aweme() {
    let p1 = Uuid::new_v4();
    let p2 = Uuid::new_v4();
    assert!(!check_and_record(p1, "v2", None, None));
    // p2 sees same aweme — not deduped on aweme key.
    assert!(!check_and_record(p2, "v2", None, None));
  }

  #[test]
  fn same_filtered_title_different_profiles_not_deduped() {
    let p1 = Uuid::new_v4();
    let p2 = Uuid::new_v4();
    // Per-profile ownership: both profiles' rows should be forwarded.
    assert!(!check_and_record(p1, "a1", Some("新月池爷"), None));
    assert!(!check_and_record(p2, "a2", Some("新月池爷"), None));
  }

  #[test]
  fn same_filtered_suggest_different_profiles_not_deduped() {
    let p1 = Uuid::new_v4();
    let p2 = Uuid::new_v4();
    assert!(!check_and_record(p1, "a3", None, Some("乡野田间的视频")));
    assert!(!check_and_record(p2, "a4", None, Some("乡野田间的视频")));
  }
}
