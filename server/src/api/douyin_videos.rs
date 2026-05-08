//! `/api/douyin/videos` — ingest + read endpoints for video items
//! scraped by the desktop client.
//!
//! Designed for the 50-browser-per-client scrape scenario: clients buffer
//! and POST in batches of up to MAX_BULK_ITEMS rows; server dedupes via the
//! UNIQUE (profile_id, aweme_id) index and reports counts so the client
//! can tell "new vs already-seen" without a second round trip.
//!
//! ## Rate limit
//!
//! `bulk_create` is the highest-frequency authenticated endpoint
//! (5–10 req/s per active user). A buggy client could DoS the DB pool
//! by looping. We enforce a per-user cap of `MAX_BULK_PER_SECOND` (env
//! `KOL_BULK_CREATE_RPS`, default 100). Over-limit returns 429.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use actix_web::{web, HttpResponse};
use chrono::{DateTime, Local};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::db::DbPool;
use crate::errors::{AppError, AppResult};

/// Per-user rate limiter state for bulk_create.
/// `(window_start, count)` reset every second.
type RateState = Mutex<(Instant, u32)>;

static RATE_LIMITER: Lazy<RwLock<HashMap<i32, Arc<RateState>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

fn rate_limit_max() -> u32 {
    static MAX: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *MAX.get_or_init(|| {
        std::env::var("KOL_BULK_CREATE_RPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n: &u32| *n >= 1)
            .unwrap_or(100)
    })
}

/// Returns `Ok(())` when the call fits within the per-user budget; an
/// AppError::TooManyRequests otherwise. 1-second sliding window, capped
/// at MAX requests. Lazy-creates per-user state on first hit.
fn check_rate_limit(user_id: i32) -> AppResult<()> {
    let max = rate_limit_max();
    // Fast path: state already exists.
    let entry = {
        let r = RATE_LIMITER.read().map_err(|_| {
            AppError::Internal("rate limiter read poisoned".into())
        })?;
        r.get(&user_id).cloned()
    };
    let entry = match entry {
        Some(e) => e,
        None => {
            // Slow path: create on first call. Re-check inside the
            // write-lock to avoid double-insertion races.
            let mut w = RATE_LIMITER.write().map_err(|_| {
                AppError::Internal("rate limiter write poisoned".into())
            })?;
            w.entry(user_id)
                .or_insert_with(|| Arc::new(Mutex::new((Instant::now(), 0))))
                .clone()
        }
    };
    let mut g = entry
        .lock()
        .map_err(|_| AppError::Internal("rate state poisoned".into()))?;
    if g.0.elapsed() >= Duration::from_secs(1) {
        *g = (Instant::now(), 0);
    }
    g.1 += 1;
    if g.1 > max {
        return Err(AppError::TooManyRequests(format!(
            "rate limit: max {max} bulk_create per second per user"
        )));
    }
    Ok(())
}

/// Cap per request. Sized so a 5 KiB row × 200 ≈ 1 MB JSON, and Postgres
/// bind params (200 × 7 = 1400) stay well under the 65535 protocol limit.
const MAX_BULK_ITEMS: usize = 200;

/// Default and ceiling for the list endpoint.
const LIST_DEFAULT_LIMIT: i64 = 100;
const LIST_MAX_LIMIT: i64 = 500;

#[derive(Debug, Serialize, FromRow)]
pub struct DouyinVideo {
    pub id: i64,
    pub profile_id: Uuid,
    pub aweme_id: String,
    pub title: Option<String>,
    /// Chain-filtered version of `title` (book-name extraction).
    /// NULL means the client's filter rejected every candidate.
    pub title_filtered: Option<String>,
    pub suggest_word: Option<String>,
    /// Chain-filtered version of `suggest_word`.
    pub suggest_word_filtered: Option<String>,
    pub share_url: Option<String>,
    pub first_frame_url: Option<String>,
    pub captured_at: DateTime<Local>,
    pub inserted_at: DateTime<Local>,
}

#[derive(Debug, Deserialize)]
pub struct VideoSubmission {
    pub profile_id: Uuid,
    pub aweme_id: String,
    pub title: Option<String>,
    #[serde(default)]
    pub title_filtered: Option<String>,
    pub suggest_word: Option<String>,
    #[serde(default)]
    pub suggest_word_filtered: Option<String>,
    pub share_url: Option<String>,
    pub first_frame_url: Option<String>,
    /// Wall-clock at DOM extraction. If absent, server stamps NOW().
    pub captured_at: Option<DateTime<Local>>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub profile_id: Option<Uuid>,
    pub limit: Option<i64>,
}

/// `POST /api/douyin/videos/bulk` — batched ingest.
///
/// Validates that every distinct profile_id in the payload belongs to the
/// caller (one COUNT query, regardless of batch size), then issues a
/// single multi-VALUES INSERT with ON CONFLICT DO NOTHING. Returns
/// `{ inserted, duplicates }` so the client can compute new-vs-seen.
pub async fn bulk_create(
    pool: web::Data<DbPool>,
    user: AuthUser,
    body: web::Json<Vec<VideoSubmission>>,
) -> AppResult<HttpResponse> {
    // Per-user rate limit. Default 100 req/s comfortably above the
    // expected 5–10 req/s peak; a hard ceiling that protects the DB
    // pool from runaway clients.
    check_rate_limit(user.0.sub)?;

    let items = body.into_inner();

    if items.is_empty() {
        return Ok(HttpResponse::Ok().json(json!({ "inserted": 0, "duplicates": 0 })));
    }
    if items.len() > MAX_BULK_ITEMS {
        return Err(AppError::BadRequest(format!(
            "max {MAX_BULK_ITEMS} items per request, got {}",
            items.len()
        )));
    }
    for it in &items {
        if it.aweme_id.is_empty() {
            return Err(AppError::BadRequest("aweme_id must be non-empty".into()));
        }
    }

    // Authorize once for all profile_ids in the batch.
    let distinct: Vec<Uuid> = items
        .iter()
        .map(|i| i.profile_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let owned: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM browser_profiles WHERE user_id = $1 AND id = ANY($2)",
    )
    .bind(user.0.sub)
    .bind(&distinct)
    .fetch_one(pool.get_ref())
    .await?;
    if (owned as usize) != distinct.len() {
        return Err(AppError::Forbidden);
    }

    let total = items.len() as i64;
    let now = Local::now();

    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "INSERT INTO douyin_videos \
         (profile_id, aweme_id, title, title_filtered, suggest_word, suggest_word_filtered, \
          share_url, first_frame_url, captured_at) ",
    );
    qb.push_values(items.iter(), |mut b, item| {
        b.push_bind(item.profile_id)
            .push_bind(&item.aweme_id)
            .push_bind(&item.title)
            .push_bind(&item.title_filtered)
            .push_bind(&item.suggest_word)
            .push_bind(&item.suggest_word_filtered)
            .push_bind(&item.share_url)
            .push_bind(&item.first_frame_url)
            .push_bind(item.captured_at.unwrap_or(now));
    });
    qb.push(" ON CONFLICT (profile_id, aweme_id) DO NOTHING");

    let result = qb.build().execute(pool.get_ref()).await?;
    let inserted = result.rows_affected() as i64;
    let duplicates = total - inserted;

    // Enqueue distinct filtered words for both 番茄达人 AND 七猫达人
    // alias submitters. Rows are scoped to the submitting user so each
    // user's aliases are fully isolated from other users'.
    //
    // Pass the caller's role through so the router can short-circuit
    // the admin-contribution redirect for admin callers (their pool
    // already IS the admin pool).
    let words: Vec<String> = collect_distinct_filtered_words(&items);
    let uid = user.0.sub;
    let role = user.0.role.clone();
    if !words.is_empty() {
        let pool_tomato = pool.clone();
        let words_tomato = words.clone();
        let role_tomato = role.clone();
        actix_web::rt::spawn(async move {
            if let Err(e) = enqueue_aliases(&pool_tomato, words_tomato, uid, &role_tomato).await {
                tracing::warn!("enqueue_aliases (tomato): {e}");
            }
        });
        let pool_qimao = pool.clone();
        let words_qimao = words;
        actix_web::rt::spawn(async move {
            if let Err(e) = enqueue_qimao_aliases(&pool_qimao, words_qimao, uid, &role).await {
                tracing::warn!("enqueue_aliases (qimao): {e}");
            }
        });
    }

    Ok(HttpResponse::Ok().json(json!({
        "inserted": inserted,
        "duplicates": duplicates,
    })))
}

/// Pull every non-empty `title_filtered` / `suggest_word_filtered` from
/// the request batch into a deduped list. Both fields contribute — a
/// row with both populated yields 2 entries; a row with only one
/// yields 1; a row with neither contributes nothing.
fn collect_distinct_filtered_words(items: &[VideoSubmission]) -> Vec<String> {
    let mut seen = HashSet::new();
    for it in items {
        if let Some(t) = it.title_filtered.as_ref() {
            let trimmed = t.trim();
            if !trimmed.is_empty() {
                seen.insert(trimmed.to_string());
            }
        }
        if let Some(s) = it.suggest_word_filtered.as_ref() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                seen.insert(trimmed.to_string());
            }
        }
    }
    seen.into_iter().collect()
}

/// Pick a random element from a non-empty slice. Used to spread book
/// assignments across all available books from `*_books`.
fn pick_random_str(pool: &[String]) -> String {
    use rand::seq::SliceRandom;
    debug_assert!(!pool.is_empty());
    pool.choose(&mut rand::thread_rng())
        .cloned()
        .unwrap_or_default()
}
fn pick_random_pair(pool: &[(i64, String)]) -> (i64, String) {
    use rand::seq::SliceRandom;
    debug_assert!(!pool.is_empty());
    pool.choose(&mut rand::thread_rng())
        .cloned()
        .unwrap_or((0, String::new()))
}

/// For each (word, alias_type) pair: route to a profile via the
/// per-(platform, alias_type) router (caller's user pool first, admin
/// pool as fallback). When **both tiers are at capacity for that
/// specific alias_type**, the pair is discarded — but other alias_types
/// for the same word are evaluated independently. So if 悟空浏览器 (6)
/// is full but 畅听 (2) still has room, the word's type=2 row is kept
/// and only type=6 is dropped.
///
/// Each word gets its own random book (picked in-memory from the
/// pre-loaded book list). A 50-word batch was 50 SELECT + 150 INSERT
/// before; now it's 1 SELECT + 1 multi-VALUES INSERT.
///
/// UNIQUE(user_id, alias_name, alias_type) makes the insert idempotent —
/// re-runs with overlapping words are no-ops.
///
/// Admin contribution: `next_word_admin_first` is consulted ONCE per
/// word so all 3 alias_types of one word share the same tier
/// preference (a contribution-flagged word goes to admin pool
/// uniformly across types, not split across types).
async fn enqueue_aliases(
    pool: &DbPool,
    words: Vec<String>,
    user_id: i32,
    user_role: &str,
) -> Result<(), String> {
    if words.is_empty() {
        return Ok(());
    }
    let books = crate::services::cache::get_tomato_books(pool).await?;
    if books.is_empty() {
        return Ok(());
    }

    let alias_types = crate::services::fanqie_promotion::ALIAS_TYPES;
    let mut router = crate::services::submission_router::Router::load(
        pool, user_id, user_role, "tomato", alias_types,
    ).await?;

    // (user_id, book_id, alias_name, alias_type, target_profile_id)
    let mut rows: Vec<(i32, String, String, i32, uuid::Uuid)> =
        Vec::with_capacity(words.len() * alias_types.len());

    for word in &words {
        let book_id = pick_random_str(&books);
        // Decide tier ONCE per word — all three alias_types share the
        // same Self/Parent/Admin decision so they don't get split
        // across tiers. The router's per-(user, platform) accumulator
        // bumps once per call here.
        let decision = router.decide_tier();
        for &alias_type in alias_types {
            // Per-type routing within the chosen tier. None here means
            // EVERY tier (preferred, admin, parent, self) is at
            // capacity for this specific alias_type — discard this
            // (word, type) pair and move on. Other alias_types for
            // the same word may still succeed via their own pick.
            let Some(target_pid) =
                router.pick_for_tier("tomato", alias_type, decision)
            else {
                continue;
            };
            rows.push((user_id, book_id.clone(), word.clone(), alias_type, target_pid));
        }
    }

    if rows.is_empty() {
        return Ok(());
    }

    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "INSERT INTO tomato_aliases (user_id, book_id, alias_name, alias_type, status, target_profile_id) ",
    );
    qb.push_values(rows.iter(), |mut b, (uid, book_id, word, alias_type, target)| {
        b.push_bind(uid)
            .push_bind(book_id)
            .push_bind(word)
            .push_bind(alias_type)
            .push_bind("pending")
            .push_bind(target);
    });
    qb.push(" ON CONFLICT (user_id, alias_name, alias_type) DO NOTHING");
    qb.build()
        .execute(pool)
        .await
        .map_err(|e| format!("bulk insert tomato_aliases: {e}"))?;
    Ok(())
}

/// qimao counterpart. Same one-SELECT + one-INSERT pattern; only one
/// row per word (no alias_type fan-out).
///
/// The admin-contribution counter is per-(user_id, platform), so
/// tomato and qimao maintain INDEPENDENT 1/N cycles. This is the
/// intended behavior: a 20% setting means "20% of tomato words AND
/// 20% of qimao words go to admin", each cycle counted on its own.
async fn enqueue_qimao_aliases(
    pool: &DbPool,
    words: Vec<String>,
    user_id: i32,
    user_role: &str,
) -> Result<(), String> {
    if words.is_empty() {
        return Ok(());
    }
    let books = crate::services::cache::get_qimao_books(pool).await?;
    if books.is_empty() {
        return Ok(());
    }

    let mut router = crate::services::submission_router::Router::load(
        pool, user_id, user_role, "qimao", &[1],
    ).await?;

    // (book_id, book_name, alias_name, target_profile_id)
    let mut rows: Vec<(i64, String, String, uuid::Uuid)> =
        Vec::with_capacity(words.len());

    for word in words {
        // Single alias_type — one decide_tier per word still maps
        // cleanly. Counter is per-(user, platform) so tomato/qimao
        // each maintain independent cascades (the operator's setting
        // applies to BOTH platforms but they don't share state).
        let decision = router.decide_tier();
        let Some(target_pid) = router.pick_for_tier("qimao", 1, decision) else {
            continue;
        };
        let (book_id, book_name) = pick_random_pair(&books);
        rows.push((book_id, book_name, word, target_pid));
    }

    if rows.is_empty() {
        return Ok(());
    }

    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "INSERT INTO qimao_aliases (user_id, book_id, book_name, alias_name, status, target_profile_id) ",
    );
    qb.push_values(rows.iter(), |mut b, (book_id, book_name, word, target)| {
        b.push_bind(user_id)
            .push_bind(book_id)
            .push_bind(book_name)
            .push_bind(word)
            .push_bind("pending")
            .push_bind(target);
    });
    qb.push(" ON CONFLICT (user_id, alias_name) DO NOTHING");
    qb.build()
        .execute(pool)
        .await
        .map_err(|e| format!("bulk insert qimao_aliases: {e}"))?;
    Ok(())
}

/// `GET /api/douyin/videos?profile_id=&limit=` — newest-first list,
/// JOINed against browser_profiles so cross-user reads are impossible.
pub async fn list(
    pool: web::Data<DbPool>,
    user: AuthUser,
    query: web::Query<ListQuery>,
) -> AppResult<HttpResponse> {
    let limit = query
        .limit
        .unwrap_or(LIST_DEFAULT_LIMIT)
        .clamp(1, LIST_MAX_LIMIT);

    let rows = sqlx::query_as::<_, DouyinVideo>(
        r#"SELECT v.id, v.profile_id, v.aweme_id,
                  v.title, v.title_filtered,
                  v.suggest_word, v.suggest_word_filtered,
                  v.share_url, v.first_frame_url,
                  v.captured_at, v.inserted_at
           FROM douyin_videos v
           JOIN browser_profiles p ON p.id = v.profile_id
           WHERE p.user_id = $1
             AND ($2::UUID IS NULL OR v.profile_id = $2)
           ORDER BY v.inserted_at DESC
           LIMIT $3"#,
    )
    .bind(user.0.sub)
    .bind(query.profile_id)
    .bind(limit)
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(rows))
}
