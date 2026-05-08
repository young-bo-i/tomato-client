//! Daily worker that scrapes the top ~100 books from 番茄达人 ranking
//! endpoint and stores them in `tomato_books`.
//!
//! Sequence:
//!   1. Pick the most-recently-updated tomato profile that has cookies
//!      for `kol.fanqieopen.com` from `platform_kol_cookies`.
//!   2. For each page (page_index = 1..=10, page_size = 10):
//!      - Build URL with msToken + a_bogus (via abogus container)
//!      - GET with Cookie header built from the stored cookie list
//!      - Parse `data.rank_books[]`
//!      - Stop early if response is empty / shorter than page_size.
//!   3. In a single transaction: TRUNCATE `tomato_books` and INSERT all
//!      collected rows. Either replaces fully or no-ops on failure.
//!
//! The job logs warnings instead of failing hard for transient issues
//! (network blip, missing cookies). It returns `Err` only for setup
//! problems (DB unreachable) where retry on the next tick is safe.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::Value as JsonValue;

use serde_json::json;

use crate::db::DbPool;
use crate::services::abogus::{
    sign_url, TOMATO_SEC_CH_UA, TOMATO_SEC_CH_UA_MOBILE, TOMATO_SEC_CH_UA_PLATFORM, TOMATO_UA,
};
use crate::services::api_log::{self, ResponseSnapshot};
use crate::services::tomato_cookie;

const SERVICE_NAME: &str = "fanqie_promotion";
const ENDPOINT_RANK_LIST: &str = "platform/ranking/rank_list";

const REFERER: &str = "https://kol.fanqieopen.com/page/content?tab_type=2&top_tab_genre=-1";

const PAGE_SIZE: i32 = 10;
const TARGET_COUNT: usize = 100;
const BASE_URL: &str = concat!(
    "https://kol.fanqieopen.com/api/platform/ranking/rank_list/by_conf/v1",
    "?rank_id=6&sort_key=9&content_tab=2",
    "&app_id=457699&aid=457699&origin_app_id=457699&host_app_id=457699"
);

#[derive(Debug, Deserialize)]
struct ApiEnvelope {
    code: i32,
    #[serde(default)]
    message: String,
    data: Option<RankData>,
}

#[derive(Debug, Deserialize)]
struct RankData {
    // API returns null (not []) on the last page — use Option so serde
    // accepts both null and a real array without a parse error.
    #[serde(default)]
    rank_books: Option<Vec<JsonValue>>,
}

/// Row about to be inserted into `tomato_books`. `raw` holds the full
/// per-book JSON object so a downstream feature can still pick up
/// fields we don't promote to columns.
struct BookRow {
    position: i32,
    book_id: String,
    book_name: String,
    author: Option<String>,
    word_num: Option<i64>,
    score: Option<f64>,
    chapter_num: Option<i32>,
    recent_income: Option<i64>,
    thumb_url: Option<String>,
    book_abstract: Option<String>,
    categories: Option<JsonValue>,
    promotion_types: Option<JsonValue>,
    raw: JsonValue,
}

pub async fn run(pool: &DbPool, abogus_url: &str) -> Result<(), String> {
    tracing::info!("tomato_rank: starting daily fetch");

    // 选 admin 的随机一个在线 cookie。书籍排行是平台全局数据,理应从
    // 管理员账号池里抽,而不是借用普通用户的 cookie 跑爬虫。
    let selected = match tomato_cookie::pick_random_online_admin(pool).await? {
        Some(s) => s,
        None => {
            tracing::warn!(
                "tomato_rank: no online admin tomato cookie available; \
                 skipping (need at least one admin with a logged-in tomato profile)"
            );
            return Ok(());
        }
    };
    let cookie_header = selected.cookie_header.clone();

    let inner = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let http = crate::services::http_retry::with_connect_retries(inner);

    // Headers captured from a real browser session — many of these
    // (sec-ch-ua, referer, x-kol-token=undefined) are part of the
    // signal the upstream uses to decide whether the request looks
    // organic. Cookies + UA are added per-request below.
    let mut base_headers = HeaderMap::new();
    base_headers.insert("accept", HeaderValue::from_static("application/json, text/plain, */*"));
    base_headers.insert(
        "accept-encoding",
        HeaderValue::from_static("gzip, deflate, br, zstd"),
    );
    base_headers.insert("accept-language", HeaderValue::from_static("zh-CN,zh;q=0.9"));
    base_headers.insert("cache-control", HeaderValue::from_static("no-cache"));
    base_headers.insert("pragma", HeaderValue::from_static("no-cache"));
    base_headers.insert("priority", HeaderValue::from_static("u=1, i"));
    base_headers.insert("referer", HeaderValue::from_static(REFERER));
    base_headers.insert("sec-ch-ua", HeaderValue::from_static(TOMATO_SEC_CH_UA));
    base_headers.insert(
        "sec-ch-ua-mobile",
        HeaderValue::from_static(TOMATO_SEC_CH_UA_MOBILE),
    );
    base_headers.insert(
        "sec-ch-ua-platform",
        HeaderValue::from_static(TOMATO_SEC_CH_UA_PLATFORM),
    );
    base_headers.insert("sec-fetch-dest", HeaderValue::from_static("empty"));
    base_headers.insert("sec-fetch-mode", HeaderValue::from_static("cors"));
    base_headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
    // Real browser actually sent the literal string "undefined" — keep it.
    base_headers.insert("x-kol-token", HeaderValue::from_static("undefined"));

    let mut books: Vec<BookRow> = Vec::with_capacity(TARGET_COUNT);
    let mut page_index = 1;

    while books.len() < TARGET_COUNT && page_index <= 20 {
        let url = format!("{BASE_URL}&page_index={page_index}&page_size={PAGE_SIZE}");
        let signed = match sign_url(&http, abogus_url, &url, "", TOMATO_UA).await {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("tomato_rank: abogus sign failed: {e}; aborting fetch");
                return Ok(());
            }
        };

        let mut snap = ResponseSnapshot::default();
        let request_summary = json!({
            "page_index": page_index,
            "profile_id": selected.profile_id,
        });

        let send_result = http
            .get(&signed)
            .headers(base_headers.clone())
            .header(reqwest::header::USER_AGENT, TOMATO_UA)
            .header(reqwest::header::COOKIE, &*cookie_header)
            .send()
            .await;

        let res = match send_result {
            Ok(r) => r,
            Err(e) => {
                api_log::log_call(pool, SERVICE_NAME, ENDPOINT_RANK_LIST,
                    request_summary, &snap, false, Some(&e.to_string())).await;
                tracing::warn!("tomato_rank: page {page_index} transport: {e}; stopping");
                break;
            }
        };

        snap.http_status = Some(res.status().as_u16());
        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            snap.body_text = Some(body.clone());
            snap.body_json = serde_json::from_str(&body).ok();
            api_log::log_call(pool, SERVICE_NAME, ENDPOINT_RANK_LIST,
                request_summary, &snap, false,
                Some(&format!("HTTP {status}"))).await;
            tracing::warn!("tomato_rank: page {page_index} HTTP {status}; stopping");
            if tomato_cookie::is_auth_failure_status(Some(status.as_u16())) {
                tomato_cookie::mark_offline(
                    pool,
                    selected.profile_id,
                    &format!("tomato_rank: HTTP {status}"),
                )
                .await
                .ok();
            }
            break;
        }

        let body_text = match res.text().await {
            Ok(t) => t,
            Err(e) => {
                api_log::log_call(pool, SERVICE_NAME, ENDPOINT_RANK_LIST,
                    request_summary, &snap, false, Some(&e.to_string())).await;
                tracing::warn!("tomato_rank: page {page_index} read body: {e}; stopping");
                break;
            }
        };
        snap.body_text = Some(body_text.clone());
        snap.body_json = serde_json::from_str(&body_text).ok();

        let envelope: ApiEnvelope = match serde_json::from_str(&body_text) {
            Ok(e) => e,
            Err(e) => {
                api_log::log_call(pool, SERVICE_NAME, ENDPOINT_RANK_LIST,
                    request_summary, &snap, false, Some(&e.to_string())).await;
                tracing::warn!("tomato_rank: page {page_index} parse: {e}; stopping");
                break;
            }
        };
        if envelope.code != 0 {
            api_log::log_call(pool, SERVICE_NAME, ENDPOINT_RANK_LIST,
                request_summary, &snap, false,
                Some(&format!("api code={} msg={}", envelope.code, envelope.message))).await;
            tracing::warn!(
                "tomato_rank: page {page_index} api code={} msg={}; stopping",
                envelope.code, envelope.message
            );
            break;
        }

        // Success — log for response shape tracking.
        api_log::log_call(pool, SERVICE_NAME, ENDPOINT_RANK_LIST,
            request_summary, &snap, true, None).await;

        // rank_books == null means the platform has no more books on this
        // page (it sends null instead of [] as the sentinel for last page).
        let page_books = match envelope.data.and_then(|d| d.rank_books) {
            Some(books) => books,
            None => {
                tracing::info!("tomato_rank: page {page_index} rank_books=null, last page");
                break;
            }
        };
        let page_count = page_books.len();
        tracing::info!("tomato_rank: page {page_index} returned {page_count} books");

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
        page_index += 1;
    }

    let total = books.len();
    tracing::info!("tomato_rank: collected {total} books, replacing table");

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("begin tx: {e}"))?;

    sqlx::query("TRUNCATE tomato_books")
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("truncate: {e}"))?;

    // Single multi-VALUES INSERT instead of 100 round-trips. Same
    // ordering preserved by iterating `books` in collection order.
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "INSERT INTO tomato_books (\
            position, book_id, book_name, author, word_num, score, \
            chapter_num, recent_income, thumb_url, book_abstract, \
            categories, promotion_types, raw\
         ) ",
    );
    qb.push_values(books.iter(), |mut s, b| {
        s.push_bind(b.position)
            .push_bind(&b.book_id)
            .push_bind(&b.book_name)
            .push_bind(&b.author)
            .push_bind(b.word_num)
            .push_bind(b.score)
            .push_bind(b.chapter_num)
            .push_bind(b.recent_income)
            .push_bind(&b.thumb_url)
            .push_bind(&b.book_abstract)
            .push_bind(&b.categories)
            .push_bind(&b.promotion_types)
            .push_bind(&b.raw);
    });
    qb.build()
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("bulk insert tomato_books: {e}"))?;

    tx.commit().await.map_err(|e| format!("commit: {e}"))?;
    crate::services::cache::invalidate_tomato_books();
    tracing::info!("tomato_rank: replaced tomato_books with {total} rows");
    Ok(())
}

fn build_row(position: i32, raw: JsonValue) -> Option<BookRow> {
    let book_id = raw.get("book_id")?.as_str()?.to_string();
    let book_name = raw
        .get("book_name")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .to_string();
    let author = raw
        .get("author")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let word_num = raw.get("word_num").and_then(JsonValue::as_i64);
    let score = raw.get("score").and_then(JsonValue::as_f64);
    let chapter_num = raw
        .get("chapter_num")
        .and_then(JsonValue::as_i64)
        .map(|n| n as i32);
    let recent_income = raw.get("recent_income").and_then(JsonValue::as_i64);
    let thumb_url = raw
        .get("thumb_url")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let book_abstract = raw
        .get("book_abstract")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let categories = raw.get("categories").cloned();
    let promotion_types = raw.get("promotion_types").cloned();

    Some(BookRow {
        position,
        book_id,
        book_name,
        author,
        word_num,
        score,
        chapter_num,
        recent_income,
        thumb_url,
        book_abstract,
        categories,
        promotion_types,
        raw,
    })
}
