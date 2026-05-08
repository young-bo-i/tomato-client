//! Client for 七猫达人 platform endpoints.
//!
//! Auth model: every API call needs `x-qm-devops-token`. Tokens are
//! obtained via `signin` (account → token), stored on the browser
//! profile row by `jobs::qimao_token_refresh`, and hot-rotated every
//! 12 hours. No browser cookies are involved — qimao's APIs accept
//! the token alone (the legacy C# stack `KolScheduled` works the same
//! way).
//!
//! Response envelope (all endpoints):
//!   { code: 200, message: "", data: {...} }
//! `code: 200` (NOT 0) is the success marker.

use std::time::Duration;

use md5::{Digest, Md5};
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest_middleware::ClientWithMiddleware as Client;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use crate::services::api_log::ResponseSnapshot;
use crate::services::upstream_error::{CallOutcome, UpstreamError};

pub const SERVICE_NAME: &str = "qimao_promotion";
pub const ENDPOINT_BOOK_INDEX: &str = "data/book/index";
pub const ENDPOINT_SIGNIN: &str = "user/signin";
pub const ENDPOINT_KEYWORD_PRECHECK: &str = "promotion/keyword_precheck";
pub const ENDPOINT_ADD_KEYWORDS: &str = "promotion/add_keywords";
pub const ENDPOINT_KEYWORD_PAGE: &str = "promotion/keyword_page";
pub const ENDPOINT_ADD_KEYWORD_LINKS: &str = "promotion/add_keyword_links";

const SIGNIN_URL: &str = "https://kol.wtzw.com/api/v1/user/signin";
const KEYWORD_PRECHECK_URL: &str = "https://kol.wtzw.com/api/v1/promotion/keyword_precheck";
const ADD_KEYWORDS_URL: &str = "https://kol.wtzw.com/api/v1/promotion/add_keywords";
const KEYWORD_PAGE_BASE_URL: &str = concat!(
    "https://kol.wtzw.com/api/v1/promotion/keyword_page",
    "?page=1&page_size=50&product_id=1&book_name=&book_id=&status=&book_type="
);
const ADD_KEYWORD_LINKS_URL: &str = "https://kol.wtzw.com/api/v1/promotion/add_keyword_links";

/// qimao only has a single ad product in scope (`QiMaoXiaoShuo=1` in
/// the C# enum). Hardcoded throughout for clarity; if the platform
/// ever exposes more product_ids we'll thread it through here first.
const PRODUCT_ID: i32 = 1;

/// Per-keyword status codes from `keyword_page.list[*].status_text_code`.
/// Confirmed from live platform UI (filter dropdown, top-to-bottom = 1..6).
pub const QIMAO_STATUS_REVIEWING: &str = "1"; // 审核中  — wait, recheck next round
pub const QIMAO_STATUS_APPROVED: &str  = "2"; // 已通过  — alias_id available, can backfill
pub const QIMAO_STATUS_REJECTED: &str  = "3"; // 已驳回  — terminal; reject_reason has detail
pub const QIMAO_STATUS_PUBLISHED: &str = "4"; // 已发布  — alias_id available, can backfill
pub const QIMAO_STATUS_CANCELLED: &str = "5"; // 已取消  — terminal
pub const QIMAO_STATUS_EXPIRED: &str   = "6"; // 已失效  — terminal

/// True when the status means the keyword is live and has an alias_id.
pub fn is_active_status(code: &str) -> bool {
    code == QIMAO_STATUS_APPROVED || code == QIMAO_STATUS_PUBLISHED
}

/// Static prefix for the book-index endpoint. The variable bits
/// (page, page_size) are appended per-request.
const BOOK_INDEX_BASE_URL: &str = concat!(
    "https://kol.wtzw.com/api/v1/data/book/index",
    "?words_num=&category_type=&is_over=&tag=&recommend_reason=",
    "&product_id=1&book_name="
);

const REFERER: &str = "https://dmp.wtzw.com/";
const ORIGIN: &str = "https://dmp.wtzw.com";
const QIMAO_UA: &str = concat!(
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 ",
    "(KHTML, like Gecko) Chrome/147.0.0.0 Safari/537.36"
);
const QIMAO_SEC_CH_UA: &str =
    "\"Google Chrome\";v=\"147\", \"Not.A/Brand\";v=\"8\", \"Chromium\";v=\"147\"";
const QIMAO_SEC_CH_UA_PLATFORM: &str = "\"macOS\"";

#[derive(Debug, Deserialize)]
struct BookIndexEnvelope {
    code: i32,
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: Option<BookIndexData>,
}

#[derive(Debug, Deserialize)]
struct BookIndexData {
    #[serde(default)]
    list: Vec<JsonValue>,
}

/// One page of books. Caller paginates by incrementing `page`. Returns
/// raw per-book objects (the worker decides which fields to promote
/// into the qimao_books table).
pub async fn fetch_book_page(
    http: &Client,
    token: &str,
    page: i32,
    page_size: i32,
) -> CallOutcome<Vec<JsonValue>> {
    let mut snap = ResponseSnapshot::default();
    let result = fetch_book_page_inner(http, token, page, page_size, &mut snap).await;
    CallOutcome::wrap(result, snap)
}

async fn fetch_book_page_inner(
    http: &Client,
    token: &str,
    page: i32,
    page_size: i32,
    snap: &mut ResponseSnapshot,
) -> Result<Vec<JsonValue>, UpstreamError> {
    let url = format!("{BOOK_INDEX_BASE_URL}&page={page}&page_size={page_size}");
    let body_text = qimao_get(http, token, &url, snap).await?;
    let envelope: BookIndexEnvelope =
        serde_json::from_str(&body_text).map_err(|e| UpstreamError::Parse(e.to_string()))?;
    match envelope.code {
        200 => Ok(envelope.data.map(|d| d.list).unwrap_or_default()),
        other => Err(UpstreamError::ApiCode {
            code: other,
            message: format!("[UNKNOWN CODE {other}] {}", envelope.message),
        }),
    }
}

/// Process-wide reqwest::Client for qimao — same shared-pool pattern
/// as `services::fanqie_promotion::HTTP`. See that module's HTTP
/// declaration for rationale; in short, `Client::clone()` is Arc-
/// cheap so the per-round `build_http_client()?` pattern in callers
/// effectively becomes a static lookup.
static HTTP: once_cell::sync::Lazy<Client> = once_cell::sync::Lazy::new(|| {
    let inner = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("qimao reqwest::Client init");
    crate::services::http_retry::with_connect_retries(inner)
});

/// Build the qimao HTTP client. 20-second timeout matches fanqie —
/// qimao is hosted on Aliyun and usually responds in under 2s, but the
/// WAF can occasionally inject challenge delays. Wrapped with
/// `with_connect_retries` so brief TCP/DNS hiccups retry automatically
/// (POSTs are non-idempotent so we deliberately do NOT retry on 5xx).
pub fn build_http_client() -> Result<Client, String> {
    Ok(HTTP.clone())
}

#[derive(Debug, Deserialize)]
struct SigninEnvelope {
    code: i32,
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: Option<SigninData>,
}

#[derive(Debug, Deserialize)]
struct SigninData {
    token: String,
}

/// Login with `(identifier, credential)` and return the session token
/// (the value of `x-qm-devops-token` for subsequent requests).
///
/// Mirrors `KolScheduled/QiMaoProxy/QiMaoInvokeProxy.Signin` from the
/// legacy C# stack — credential is hashed with MD5 (lowercase hex)
/// before being POSTed. The user's database row stores the credential
/// in plaintext for now; encryption-at-rest can be added later without
/// changing this contract.
pub async fn signin(
    http: &Client,
    identifier: &str,
    credential: &str,
) -> CallOutcome<String> {
    let mut snap = ResponseSnapshot::default();
    let result = signin_inner(http, identifier, credential, &mut snap).await;
    CallOutcome::wrap(result, snap)
}

async fn signin_inner(
    http: &Client,
    identifier: &str,
    credential: &str,
    snap: &mut ResponseSnapshot,
) -> Result<String, UpstreamError> {
    let body = json!({
        "identifier": identifier,
        "credential": md5_hex(credential),
        "kind": "password",
    })
    .to_string();
    // signin is the only endpoint that doesn't carry a token —
    // explicitly pass "" so base_headers omits the x-qm-devops-token
    // header.
    let body_text = qimao_post_json(http, "", SIGNIN_URL, body, snap).await?;
    let envelope: SigninEnvelope =
        serde_json::from_str(&body_text).map_err(|e| UpstreamError::Parse(e.to_string()))?;
    match envelope.code {
        200 => {}
        other => return Err(UpstreamError::ApiCode {
            code: other,
            message: format!("[UNKNOWN CODE {other}] {}", envelope.message),
        }),
    }
    envelope
        .data
        .map(|d| d.token)
        .filter(|t| !t.is_empty())
        .ok_or(UpstreamError::MissingField("token"))
}

/// Lowercase hex MD5, matching what the upstream's signin endpoint
/// expects (the legacy C# implementation does the same).
fn md5_hex(s: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(s.as_bytes());
    hex::encode(hasher.finalize())
}

/// Common headers for every qimao call. `token` is empty for the
/// signin call (the only one that doesn't need auth) and the actual
/// token string elsewhere.
fn base_headers(token: &str) -> Result<HeaderMap, String> {
    let mut h = HeaderMap::new();
    h.insert(
        "accept",
        HeaderValue::from_static("application/json, text/plain, */*"),
    );
    h.insert(
        "accept-encoding",
        HeaderValue::from_static("gzip, deflate, br, zstd"),
    );
    h.insert("accept-language", HeaderValue::from_static("zh-CN,zh;q=0.9"));
    h.insert("cache-control", HeaderValue::from_static("no-cache"));
    h.insert("pragma", HeaderValue::from_static("no-cache"));
    h.insert("referer", HeaderValue::from_static(REFERER));
    h.insert("origin", HeaderValue::from_static(ORIGIN));
    h.insert("sec-ch-ua", HeaderValue::from_static(QIMAO_SEC_CH_UA));
    h.insert("sec-ch-ua-mobile", HeaderValue::from_static("?0"));
    h.insert(
        "sec-ch-ua-platform",
        HeaderValue::from_static(QIMAO_SEC_CH_UA_PLATFORM),
    );
    if !token.is_empty() {
        let v = HeaderValue::from_str(token).map_err(|e| format!("invalid token header: {e}"))?;
        h.insert("x-qm-devops-token", v);
    }
    Ok(h)
}

// ───────────────────────── promotion endpoints ───────────────────────

#[derive(Debug, Deserialize)]
struct PrecheckEnvelope {
    code: i32,
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: Option<PrecheckData>,
}

#[derive(Debug, Deserialize, Default)]
struct PrecheckData {
    /// Empty string when the keyword is acceptable; non-empty when the
    /// platform pre-rejects (e.g. blacklisted, similar to existing
    /// alias). The C# stack treated any non-empty string as terminal.
    #[serde(default)]
    reject_reason: String,
}

/// `POST /promotion/keyword_precheck` — validate a keyword before
/// committing to add_keywords. Returns the platform's reject_reason
/// (empty `""` means "OK to submit").
pub async fn keyword_precheck(
    http: &Client,
    token: &str,
    keyword: &str,
) -> CallOutcome<String> {
    let mut snap = ResponseSnapshot::default();
    let result = keyword_precheck_inner(http, token, keyword, &mut snap).await;
    CallOutcome::wrap(result, snap)
}

async fn keyword_precheck_inner(
    http: &Client,
    token: &str,
    keyword: &str,
    snap: &mut ResponseSnapshot,
) -> Result<String, UpstreamError> {
    let body = json!({ "keyword": keyword, "product_id": PRODUCT_ID }).to_string();
    let body_text = qimao_post_json(http, token, KEYWORD_PRECHECK_URL, body, snap).await?;
    let env: PrecheckEnvelope =
        serde_json::from_str(&body_text).map_err(|e| UpstreamError::Parse(e.to_string()))?;
    match env.code {
        // 200: success — reject_reason="" means OK, non-empty means rejected keyword
        200 => Ok(env.data.unwrap_or_default().reject_reason),
        other => Err(UpstreamError::ApiCode {
            code: other,
            message: format!("[UNKNOWN CODE {other}] {}", env.message),
        }),
    }
}

#[derive(Debug, Deserialize)]
struct AddKeywordsEnvelope {
    code: i32,
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: Option<JsonValue>,
}

/// `POST /promotion/add_keywords` — submit one keyword for one book.
/// Body shape mirrors the legacy C# stack EXACTLY: the `keywords`
/// field is a JSON-encoded STRING (not an inline array). The platform
/// can return code=200 with a non-empty `failed_list[]` even on
/// "success" — we surface that as `ApiCode` so the worker doesn't
/// mistakenly mark the row submitted.
pub async fn add_keywords(
    http: &Client,
    token: &str,
    book_id: i64,
    book_name: &str,
    keyword: &str,
) -> CallOutcome<()> {
    let mut snap = ResponseSnapshot::default();
    let result = add_keywords_inner(http, token, book_id, book_name, keyword, &mut snap).await;
    CallOutcome::wrap(result, snap)
}

async fn add_keywords_inner(
    http: &Client,
    token: &str,
    book_id: i64,
    book_name: &str,
    keyword: &str,
    snap: &mut ResponseSnapshot,
) -> Result<(), UpstreamError> {
    let inner = json!([{
        "book_id": book_id,
        "book_name": book_name,
        "keyword": keyword,
    }])
    .to_string();
    let body = json!({
        "keywords": inner,
        "product_id": PRODUCT_ID,
    })
    .to_string();
    let body_text = qimao_post_json(http, token, ADD_KEYWORDS_URL, body, snap).await?;
    let env: AddKeywordsEnvelope =
        serde_json::from_str(&body_text).map_err(|e| UpstreamError::Parse(e.to_string()))?;
    match env.code {
        200 => {}
        other => return Err(UpstreamError::ApiCode {
            code: other,
            message: format!("[UNKNOWN CODE {other}] {}", env.message),
        }),
    }
    // code=200 but failed_list non-empty = partial/full rejection
    if let Some(data) = env.data {
        if let Some(failed) = data.get("failed_list") {
            let is_empty = failed.is_null() || failed.as_array().is_some_and(|a| a.is_empty());
            if !is_empty {
                return Err(UpstreamError::ApiCode {
                    code: 200,
                    message: format!("failed_list non-empty: {failed}"),
                });
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
pub struct QimaoKeywordItem {
    /// Platform-side keyword id; what we store as `qimao_aliases.alias_id`.
    pub id: i64,
    /// The keyword text we originally submitted.
    pub search_keyword: String,
    /// Status code: "1"=审核中, "2"/"4"=通过, others=invalid. Strings
    /// (not ints) per the upstream — be careful matching.
    pub status_text_code: String,
    /// Human-readable status (e.g. "审核中"). Useful for dashboards.
    #[serde(default)]
    pub status_text: String,
    /// Why the platform rejected the keyword, when applicable.
    #[serde(default)]
    pub reject_reason: String,
}

#[derive(Debug, Deserialize)]
struct KeywordPageEnvelope {
    code: i32,
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: Option<KeywordPageData>,
}

#[derive(Debug, Deserialize)]
struct KeywordPageData {
    #[serde(default)]
    list: Vec<QimaoKeywordItem>,
}

/// `GET /promotion/keyword_page?...&keyword=X` — search the platform
/// for keywords this account submitted. Used by the backfill worker
/// to recover the alias_id (which `add_keywords` doesn't return) and
/// to check the platform-side review status.
///
/// `start_date` / `end_date` widen the search window to capture
/// recently-submitted keywords; the C# stack uses [-1 month, +1 day].
pub async fn keyword_page(
    http: &Client,
    token: &str,
    keyword: &str,
    start_date: &str,
    end_date: &str,
) -> CallOutcome<Vec<QimaoKeywordItem>> {
    let mut snap = ResponseSnapshot::default();
    let result = keyword_page_inner(http, token, keyword, start_date, end_date, &mut snap).await;
    CallOutcome::wrap(result, snap)
}

async fn keyword_page_inner(
    http: &Client,
    token: &str,
    keyword: &str,
    start_date: &str,
    end_date: &str,
    snap: &mut ResponseSnapshot,
) -> Result<Vec<QimaoKeywordItem>, UpstreamError> {
    let url = format!(
        "{KEYWORD_PAGE_BASE_URL}&start_date={}&end_date={}&keyword={}",
        urlencoding::encode(start_date),
        urlencoding::encode(end_date),
        urlencoding::encode(keyword),
    );
    let body_text = qimao_get(http, token, &url, snap).await?;
    let env: KeywordPageEnvelope =
        serde_json::from_str(&body_text).map_err(|e| UpstreamError::Parse(e.to_string()))?;
    match env.code {
        200 => Ok(env.data.map(|d| d.list).unwrap_or_default()),
        other => Err(UpstreamError::ApiCode {
            code: other,
            message: format!("[UNKNOWN CODE {other}] {}", env.message),
        }),
    }
}

#[derive(Debug, Deserialize)]
struct AddKeywordLinksEnvelope {
    code: i32,
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: Option<JsonValue>,
}

/// `POST /promotion/add_keyword_links` — backfill a Douyin link for
/// an active alias. Body shape matches the C# stack: `keywords` is a
/// JSON-encoded STRING (a one-element array). promotion_type is
/// hardcoded to 1 (book).
pub async fn add_keyword_links(
    http: &Client,
    token: &str,
    alias_id: i64,
    keyword: &str,
    url: &str,
) -> CallOutcome<()> {
    let mut snap = ResponseSnapshot::default();
    let result = add_keyword_links_inner(http, token, alias_id, keyword, url, &mut snap).await;
    CallOutcome::wrap(result, snap)
}

async fn add_keyword_links_inner(
    http: &Client,
    token: &str,
    alias_id: i64,
    keyword: &str,
    url: &str,
    snap: &mut ResponseSnapshot,
) -> Result<(), UpstreamError> {
    let inner = json!([{
        "url": url,
        "keyword": keyword,
        "id": alias_id,
    }])
    .to_string();
    let body = json!({
        "keywords": inner,
        "promotion_type": 1,
    })
    .to_string();
    let body_text = qimao_post_json(http, token, ADD_KEYWORD_LINKS_URL, body, snap).await?;
    let env: AddKeywordLinksEnvelope =
        serde_json::from_str(&body_text).map_err(|e| UpstreamError::Parse(e.to_string()))?;
    match env.code {
        200 => {}
        other => return Err(UpstreamError::ApiCode {
            code: other,
            message: format!("[UNKNOWN CODE {other}] {}", env.message),
        }),
    }
    if let Some(data) = env.data {
        if let Some(failed) = data.get("failed_list") {
            let is_empty = failed.is_null() || failed.as_array().is_some_and(|a| a.is_empty());
            if !is_empty {
                return Err(UpstreamError::ApiCode {
                    code: 200,
                    message: format!("failed_list non-empty: {failed}"),
                });
            }
        }
    }
    Ok(())
}

/// Shared POST helper. Sets the standard `Content-Type: application/json`
/// + token-bearing headers, fills in the snapshot, returns the body
/// text on 2xx or a typed `UpstreamError`.
async fn qimao_post_json(
    http: &Client,
    token: &str,
    url: &str,
    body: String,
    snap: &mut ResponseSnapshot,
) -> Result<String, UpstreamError> {
    let mut headers = base_headers(token).map_err(UpstreamError::Other)?;
    headers.insert("sec-fetch-dest", HeaderValue::from_static("empty"));
    headers.insert("sec-fetch-mode", HeaderValue::from_static("cors"));
    headers.insert("sec-fetch-site", HeaderValue::from_static("same-site"));

    let res = http
        .post(url)
        .headers(headers)
        .header(reqwest::header::USER_AGENT, QIMAO_UA)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| UpstreamError::Transport(e.to_string()))?;
    snap.http_status = Some(res.status().as_u16());
    let status = res.status();

    let body_text = res
        .text()
        .await
        .map_err(|e| UpstreamError::Transport(format!("read body: {e}")))?;
    snap.body_text = Some(body_text.clone());
    snap.body_json = serde_json::from_str(&body_text).ok();

    if !status.is_success() {
        let preview = body_text.chars().take(200).collect::<String>();
        return Err(UpstreamError::from_http(status.as_u16(), preview));
    }
    Ok(body_text)
}

/// GET counterpart. Used by `fetch_book_page` and `keyword_page`.
async fn qimao_get(
    http: &Client,
    token: &str,
    url: &str,
    snap: &mut ResponseSnapshot,
) -> Result<String, UpstreamError> {
    let mut headers = base_headers(token).map_err(UpstreamError::Other)?;
    headers.insert("sec-fetch-dest", HeaderValue::from_static("empty"));
    headers.insert("sec-fetch-mode", HeaderValue::from_static("cors"));
    headers.insert("sec-fetch-site", HeaderValue::from_static("same-site"));

    let res = http
        .get(url)
        .headers(headers)
        .header(reqwest::header::USER_AGENT, QIMAO_UA)
        .send()
        .await
        .map_err(|e| UpstreamError::Transport(e.to_string()))?;
    snap.http_status = Some(res.status().as_u16());
    let status = res.status();

    let body_text = res
        .text()
        .await
        .map_err(|e| UpstreamError::Transport(format!("read body: {e}")))?;
    snap.body_text = Some(body_text.clone());
    snap.body_json = serde_json::from_str(&body_text).ok();

    if !status.is_success() {
        let preview = body_text.chars().take(200).collect::<String>();
        return Err(UpstreamError::from_http(status.as_u16(), preview));
    }
    Ok(body_text)
}
