//! In-process cache for the singleton `admin_settings` row.
//!
//! The submission router consults this on every word it routes (after
//! the cache hit it's a single Arc clone). 60-second TTL is a safety
//! net for direct DB edits; the API path explicitly invalidates on
//! every PUT so admin slider changes take effect immediately.

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use sqlx::Row;

use crate::db::DbPool;

/// Mirrors columns of the `admin_settings` table that the rest of
/// the server cares about. Add fields here as new global knobs are
/// added to the table.
#[derive(Debug, Clone, Default)]
pub struct AdminSettings {
    /// 0..=100. See `Router::next_word_admin_first` for how this is
    /// applied. 0 disables the redirection entirely.
    pub admin_contribution_pct: i32,
}

const TTL: Duration = Duration::from_secs(60);

static CACHE: Lazy<RwLock<Option<(Instant, Arc<AdminSettings>)>>> =
    Lazy::new(|| RwLock::new(None));

/// Returns an `Arc` to the current settings, hitting the cache when
/// fresh and falling through to a single SELECT otherwise.
pub async fn get(pool: &DbPool) -> Result<Arc<AdminSettings>, String> {
    if let Ok(g) = CACHE.read() {
        if let Some((t, ref s)) = *g {
            if t.elapsed() < TTL {
                return Ok(Arc::clone(s));
            }
        }
    }

    // Slow path: write-lock + double-check (avoids thundering herd).
    let mut g = CACHE
        .write()
        .map_err(|_| "admin_settings cache write poisoned".to_string())?;
    if let Some((t, ref s)) = *g {
        if t.elapsed() < TTL {
            return Ok(Arc::clone(s));
        }
    }

    let row = sqlx::query("SELECT admin_contribution_pct FROM admin_settings WHERE id = 1")
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("load admin_settings: {e}"))?;

    let pct: i32 = row
        .as_ref()
        .and_then(|r| r.try_get::<i32, _>("admin_contribution_pct").ok())
        .unwrap_or(0);

    let settings = Arc::new(AdminSettings {
        admin_contribution_pct: pct.clamp(0, 100),
    });
    *g = Some((Instant::now(), Arc::clone(&settings)));
    Ok(settings)
}

/// Drop the cached value so the next `get()` re-reads from DB. Called
/// from the PUT handler whenever the row is updated.
pub fn invalidate() {
    if let Ok(mut g) = CACHE.write() {
        *g = None;
    }
}
