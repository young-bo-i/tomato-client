//! In-process write-invalidated cache for frequently-read, rarely-written data.
//!
//! Design:
//! - Each cache entry is `Arc<RwLock<Option<(Instant, Arc<T>)>>>`.
//! - Reads: take a read-lock, return an Arc clone of the cached value if fresh.
//! - Misses: drop read-lock, take write-lock, re-check (double-checked locking
//!   to prevent thundering herd), fetch from DB, store.
//! - Invalidation: on any write to the underlying table, call `invalidate()`.
//!   This simply sets the inner value to `None`; the next read rebuilds.
//!
//! TTL acts as a safety net for missed invalidations (e.g. direct DB edits).
//! Normal operation relies on explicit invalidation.
//!
//! Three caches are provided:
//!   1. `SUBMISSION_CONFIG`  — kol_submission_config rows, per-user
//!   2. `TOMATO_BOOKS`       — tomato_books book_ids
//!   3. `QIMAO_BOOKS`        — qimao_books (book_id, book_name) pairs

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use uuid::Uuid;

use crate::db::DbPool;

const TTL: Duration = Duration::from_secs(300); // 5-minute safety TTL

// ── Submission config ─────────────────────────────────────────────────────

/// (platform, alias_type) → (enabled, daily_limit)
pub type ConfigMap = HashMap<(String, i32), (bool, i32)>;
/// profile_id → ConfigMap
pub type ProfileConfigMap = HashMap<Uuid, ConfigMap>;

struct ConfigCache {
    inner: RwLock<Option<(Instant, Arc<ProfileConfigMap>)>>,
}

static SUBMISSION_CONFIG: Lazy<Arc<ConfigCache>> = Lazy::new(|| {
    Arc::new(ConfigCache {
        inner: RwLock::new(None),
    })
});

/// Returns an Arc to the submission config map, loading from DB on miss.
/// Arc clone is O(1) — callers share the same allocation until the next
/// invalidation or TTL expiry.
pub async fn get_submission_config(pool: &DbPool) -> Result<Arc<ProfileConfigMap>, String> {
    // Fast path: read-lock, return Arc clone if fresh.
    if let Ok(guard) = SUBMISSION_CONFIG.inner.read() {
        if let Some((ts, ref map)) = *guard {
            if ts.elapsed() < TTL {
                return Ok(Arc::clone(map));
            }
        }
    }

    // Slow path: write-lock, double-check, then fetch.
    let mut guard = SUBMISSION_CONFIG
        .inner
        .write()
        .map_err(|_| "config cache write lock poisoned".to_string())?;

    if let Some((ts, ref map)) = *guard {
        if ts.elapsed() < TTL {
            return Ok(Arc::clone(map)); // another task already refreshed
        }
    }

    #[derive(sqlx::FromRow)]
    struct ConfigRow {
        profile_id: Uuid,
        platform: String,
        alias_type: i32,
        enabled: bool,
        daily_limit: i32,
    }
    let rows: Vec<ConfigRow> = sqlx::query_as(
        "SELECT profile_id, platform, alias_type, enabled, daily_limit
         FROM kol_submission_config",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("load submission_config: {e}"))?;

    let mut map: ProfileConfigMap = HashMap::new();
    for r in rows {
        map.entry(r.profile_id)
            .or_default()
            .insert((r.platform, r.alias_type), (r.enabled, r.daily_limit));
    }

    let shared = Arc::new(map);
    *guard = Some((Instant::now(), Arc::clone(&shared)));
    Ok(shared)
}

pub fn invalidate_submission_config() {
    if let Ok(mut g) = SUBMISSION_CONFIG.inner.write() {
        *g = None;
    }
}

// ── Tomato books ──────────────────────────────────────────────────────────

struct TomatoBooksCache {
    inner: RwLock<Option<(Instant, Vec<String>)>>,
}

static TOMATO_BOOKS: Lazy<Arc<TomatoBooksCache>> = Lazy::new(|| {
    Arc::new(TomatoBooksCache {
        inner: RwLock::new(None),
    })
});

pub async fn get_tomato_books(pool: &DbPool) -> Result<Vec<String>, String> {
    if let Ok(guard) = TOMATO_BOOKS.inner.read() {
        if let Some((ts, ref v)) = *guard {
            if ts.elapsed() < TTL {
                return Ok(v.clone());
            }
        }
    }

    let mut guard = TOMATO_BOOKS
        .inner
        .write()
        .map_err(|_| "tomato_books cache write lock poisoned".to_string())?;

    if let Some((ts, ref v)) = *guard {
        if ts.elapsed() < TTL {
            return Ok(v.clone());
        }
    }

    let books: Vec<String> = sqlx::query_scalar("SELECT book_id FROM tomato_books")
        .fetch_all(pool)
        .await
        .map_err(|e| format!("load tomato_books: {e}"))?;

    *guard = Some((Instant::now(), books.clone()));
    Ok(books)
}

pub fn invalidate_tomato_books() {
    if let Ok(mut g) = TOMATO_BOOKS.inner.write() {
        *g = None;
    }
}

// ── Qimao books ───────────────────────────────────────────────────────────

struct QimaoBooksCache {
    inner: RwLock<Option<(Instant, Vec<(i64, String)>)>>,
}

static QIMAO_BOOKS: Lazy<Arc<QimaoBooksCache>> = Lazy::new(|| {
    Arc::new(QimaoBooksCache {
        inner: RwLock::new(None),
    })
});

pub async fn get_qimao_books(pool: &DbPool) -> Result<Vec<(i64, String)>, String> {
    if let Ok(guard) = QIMAO_BOOKS.inner.read() {
        if let Some((ts, ref v)) = *guard {
            if ts.elapsed() < TTL {
                return Ok(v.clone());
            }
        }
    }

    let mut guard = QIMAO_BOOKS
        .inner
        .write()
        .map_err(|_| "qimao_books cache write lock poisoned".to_string())?;

    if let Some((ts, ref v)) = *guard {
        if ts.elapsed() < TTL {
            return Ok(v.clone());
        }
    }

    let books: Vec<(i64, String)> =
        sqlx::query_as("SELECT book_id, book_name FROM qimao_books")
            .fetch_all(pool)
            .await
            .map_err(|e| format!("load qimao_books: {e}"))?;

    *guard = Some((Instant::now(), books.clone()));
    Ok(books)
}

pub fn invalidate_qimao_books() {
    if let Ok(mut g) = QIMAO_BOOKS.inner.write() {
        *g = None;
    }
}
