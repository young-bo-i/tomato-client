//! Daily worker that scrapes the top ~100 books from 七猫达人's listing
//! endpoint and stores them in `qimao_books`.
//!
//! Sequence (mirrors `tomato_rank`):
//!   1. Pick a random online qimao admin cookie + extra_headers (the
//!      qimao session needs `x-qm-devops-token`, stored in
//!      `platform_kol_cookies.extra_headers` for that profile).
//!   2. For each page (page = 1..=10, page_size = 10):
//!      - GET /api/v1/data/book/index?...
//!      - Parse `data.list[]`
//!      - Stop early if response is shorter than page_size.
//!   3. In a single transaction: TRUNCATE `qimao_books` and INSERT all
//!      collected rows. Either replaces fully or no-ops on failure.
//!
//! On any per-page failure we audit-log + stop the run; partial data
//! is preferred over a half-populated table swap.

use std::sync::Arc;

use serde_json::{json, Value as JsonValue};

use crate::db::DbPool;
use crate::services::qimao_account;
use crate::services::qimao_promotion::{
    build_http_client, fetch_book_page, ENDPOINT_BOOK_INDEX, SERVICE_NAME,
};

const PAGE_SIZE: i32 = 10;
const TARGET_COUNT: usize = 100;
const MAX_PAGES: i32 = 20; // safety cap; should hit TARGET_COUNT well before this

/// Row about to be inserted into `qimao_books`. `raw` holds the full
/// per-book JSON object so a downstream feature can still pick up
/// fields we don't promote to columns.
struct BookRow {
    position: i32,
    book_id: i64,
    book_name: String,
    author: Option<String>,
    first_category: Option<String>,
    second_category: Option<String>,
    words_num_text: Option<String>,
    words: Option<i64>,
    cover: Option<String>,
    intro: Option<String>,
    income_text: Option<String>,
    is_forbid: bool,
    is_rights: bool,
    ad_status: Option<i32>,
    tags: Option<JsonValue>,
    raw: JsonValue,
}

pub async fn run(pool: &DbPool) -> Result<(), String> {
    tracing::info!("qimao_rank: starting daily fetch");

    let selected = match qimao_account::pick_random_active(pool).await? {
        Some(s) => s,
        None => {
            tracing::warn!(
                "qimao_rank: no usable qimao token (need a profile with credentials whose token has been refreshed); skipping"
            );
            return Ok(());
        }
    };

    let http = build_http_client()?;
    let mut books: Vec<BookRow> = Vec::with_capacity(TARGET_COUNT);
    let mut page = 1;

    while books.len() < TARGET_COUNT && page <= MAX_PAGES {
        let outcome = fetch_book_page(&http, &selected.token, page, PAGE_SIZE).await;
        let request_summary = json!({
            "page": page,
            "page_size": PAGE_SIZE,
            "profile_id": selected.profile_id,
        });
        let page_books = match outcome
            .audit(pool, SERVICE_NAME, ENDPOINT_BOOK_INDEX, request_summary)
            .await
        {
            Ok(list) => list,
            Err(err) if err.is_auth_failure() => {
                tracing::warn!("qimao_rank: page {page} auth failure: {err}; stopping");
                qimao_account::invalidate_token(
                    pool,
                    selected.profile_id,
                    &format!("qimao_rank: {err}"),
                )
                .await
                .ok();
                break;
            }
            Err(err) => {
                tracing::warn!("qimao_rank: page {page} failed: {err}; stopping");
                break;
            }
        };
        let page_count = page_books.len();
        tracing::info!("qimao_rank: page {page} returned {page_count} books");

        for raw in page_books {
            if books.len() >= TARGET_COUNT {
                break;
            }
            if let Some(row) = build_row(books.len() as i32 + 1, raw) {
                books.push(row);
            }
        }

        if page_count < PAGE_SIZE as usize {
            break; // last page
        }
        page += 1;
    }

    let total = books.len();
    if total == 0 {
        tracing::warn!("qimao_rank: collected 0 books — leaving table untouched");
        return Ok(());
    }
    tracing::info!("qimao_rank: collected {total} books, replacing table");

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("begin tx: {e}"))?;

    sqlx::query("TRUNCATE qimao_books")
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("truncate: {e}"))?;

    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "INSERT INTO qimao_books (\
            position, book_id, book_name, author, first_category, second_category, \
            words_num_text, words, cover, intro, income_text, \
            is_forbid, is_rights, ad_status, tags, raw\
         ) ",
    );
    qb.push_values(books.iter(), |mut s, b| {
        s.push_bind(b.position)
            .push_bind(b.book_id)
            .push_bind(&b.book_name)
            .push_bind(&b.author)
            .push_bind(&b.first_category)
            .push_bind(&b.second_category)
            .push_bind(&b.words_num_text)
            .push_bind(b.words)
            .push_bind(&b.cover)
            .push_bind(&b.intro)
            .push_bind(&b.income_text)
            .push_bind(b.is_forbid)
            .push_bind(b.is_rights)
            .push_bind(b.ad_status)
            .push_bind(&b.tags)
            .push_bind(&b.raw);
    });
    qb.build()
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("bulk insert qimao_books: {e}"))?;

    tx.commit().await.map_err(|e| format!("commit: {e}"))?;
    crate::services::cache::invalidate_qimao_books();
    tracing::info!("qimao_rank: replaced qimao_books with {total} rows");
    Ok(())
}

fn build_row(position: i32, raw: JsonValue) -> Option<BookRow> {
    let book_id = raw.get("book_id").and_then(JsonValue::as_i64)?;
    let book_name = raw
        .get("book_name")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .to_string();
    let author = raw
        .get("author")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let first_category = raw
        .get("first_category")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let second_category = raw
        .get("second_category")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let words_num_text = raw
        .get("words_num")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let words = raw.get("words").and_then(JsonValue::as_i64);
    let cover = raw
        .get("cover")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let intro = raw
        .get("intro")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let income_text = raw
        .get("income_text")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let is_forbid = raw
        .get("is_forbid")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let is_rights = raw
        .get("is_rights")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let ad_status = raw
        .get("ad_status")
        .and_then(JsonValue::as_i64)
        .map(|n| n as i32);
    let tags = raw.get("tags").cloned();

    Some(BookRow {
        position,
        book_id,
        book_name,
        author,
        first_category,
        second_category,
        words_num_text,
        words,
        cover,
        intro,
        income_text,
        is_forbid,
        is_rights,
        ad_status,
        tags,
        raw,
    })
}

/// Async-friendly wrapper for the cron scheduler — mirrors the shape
/// `tomato_rank::run` is invoked with so jobs/mod.rs can wire them
/// uniformly.
pub async fn run_with_pool(pool: Arc<DbPool>) {
    if let Err(e) = run(&pool).await {
        tracing::error!("qimao_rank job failed: {e}");
    }
}
