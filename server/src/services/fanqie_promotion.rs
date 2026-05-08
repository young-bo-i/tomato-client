//! Client for the 番茄达人 promotion-plan-create endpoint:
//!   POST https://kol.fanqieopen.com/api/platform/promotion/plan/create/v:version
//!
//! Submits a `(book_id, alias_name, alias_type)` triple as a "promotion
//! alias" — i.e. tells the platform "treat this word as another way to
//! find this book". The same word goes in three times, one per
//! alias_type, to cover all surfaces the platform serves:
//!   1 → 番茄小说
//!   2 → 番茄畅听
//!   6 → 悟空浏览器
//!
//! Mirrors the abogus signing dance in `tomato_rank.rs` (msToken +
//! a_bogus). The cookie + UA must match what the abogus algorithm was
//! tuned for; see `services::abogus`.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue};
use reqwest_middleware::ClientWithMiddleware as Client;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use crate::services::abogus::{
    sign_url, TOMATO_SEC_CH_UA, TOMATO_SEC_CH_UA_MOBILE, TOMATO_SEC_CH_UA_PLATFORM, TOMATO_UA,
};
use crate::services::api_log::ResponseSnapshot;
use crate::services::upstream_error::{CallOutcome, UpstreamError};

/// Logical service name for `external_api_responses.service`.
pub const SERVICE_NAME: &str = "fanqie_promotion";

/// Endpoint label for `external_api_responses.endpoint`.
pub const ENDPOINT_PROMOTION_PLAN_CREATE: &str = "promotion/plan/create";

/// Endpoint label for the backfill (post-create) call.
pub const ENDPOINT_PROMOTION_POST_CREATE: &str = "promotion/post/create";

/// Endpoint label for the alias status lookup (used by backfill_submitter
/// to gate on review state before attempting post/create).
pub const ENDPOINT_PROMOTION_PLAN_LIST: &str = "promotion/plan/list";

pub const ALIAS_TYPE_NOVEL: i32 = 1;
pub const ALIAS_TYPE_AUDIO: i32 = 2;
pub const ALIAS_TYPE_WUKONG: i32 = 6;
pub const ALIAS_TYPES: &[i32] = &[ALIAS_TYPE_NOVEL, ALIAS_TYPE_AUDIO, ALIAS_TYPE_WUKONG];

/// Per-alias review status reported by `promotion/plan/list`. Sourced
/// from the platform's filter dropdown; values 1–6.
pub const ALIAS_STATUS_ACTIVE: i32 = 1;          // 生效中
pub const ALIAS_STATUS_EXPIRED: i32 = 2;         // 已失效
pub const ALIAS_STATUS_PENDING_REVIEW: i32 = 3;  // 待审核
pub const ALIAS_STATUS_REJECTED: i32 = 4;        // 审核不通过
pub const ALIAS_STATUS_FORCE_INVALID: i32 = 5;   // 强制失效
pub const ALIAS_STATUS_EXPIRING: i32 = 6;        // 即将失效

/// Human-readable label for an alias_status value. Used in error
/// reasons + dashboard tooltips.
pub fn alias_status_label(status: i32) -> &'static str {
    match status {
        ALIAS_STATUS_ACTIVE => "生效中",
        ALIAS_STATUS_EXPIRED => "已失效",
        ALIAS_STATUS_PENDING_REVIEW => "待审核",
        ALIAS_STATUS_REJECTED => "审核不通过",
        ALIAS_STATUS_FORCE_INVALID => "强制失效",
        ALIAS_STATUS_EXPIRING => "即将失效",
        _ => "未知状态",
    }
}

const REFERER: &str = "https://kol.fanqieopen.com/page/content?tab_type=2&top_tab_genre=-1";
const BASE_URL: &str = concat!(
    "https://kol.fanqieopen.com/api/platform/promotion/plan/create/v:version",
    "?app_id=457699&aid=457699&origin_app_id=457699&host_app_id=457699"
);
const POST_CREATE_BASE_URL: &str = concat!(
    "https://kol.fanqieopen.com/api/platform/promotion/post/create/v:version",
    "?app_id=457699&aid=457699&origin_app_id=457699&host_app_id=457699"
);
/// Static prefix for the list endpoint. The variable bits (alias_name +
/// task_type + paging) are appended per-request because alias_name has
/// to be percent-encoded and task_type varies by alias_type.
const PLAN_LIST_BASE_URL: &str = concat!(
    "https://kol.fanqieopen.com/api/platform/promotion/plan/list/v:version",
    "?need_post_audit=true&page_index=0&page_size=10",
    "&app_id=457699&aid=457699&origin_app_id=457699&host_app_id=457699"
);

#[derive(Debug, Deserialize)]
struct ApiEnvelope {
    code: i32,
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: Option<AliasCreateData>,
}

#[derive(Debug, Deserialize)]
struct AliasCreateData {
    #[serde(default)]
    alias_id: Option<String>,
    #[serde(default)]
    reason: Option<JsonValue>,
}

/// Process-wide reqwest::Client for fanqie. Built once on first
/// access; `Client::clone()` is Arc-cheap so callers can keep their
/// `let http = build_http_client()?;` pattern with no per-call cost.
/// Connection pool / DNS cache are shared across every call.
static HTTP: once_cell::sync::Lazy<Client> = once_cell::sync::Lazy::new(|| {
    let inner = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("fanqie reqwest::Client init");
    crate::services::http_retry::with_connect_retries(inner)
});

/// Hand out a clone of the shared client. Single 20-second timeout
/// matches what the upstream typically responds in; idle keep-alive
/// stays ≥ 90s so consecutive worker rounds reuse connections. The
/// returned client is wrapped with `with_connect_retries` so transient
/// connect-establishment failures retry automatically (see
/// `services::http_retry`).
pub fn build_http_client() -> Result<Client, String> {
    Ok(HTTP.clone())
}

/// Submit one alias. Outer wrapper builds the snapshot envelope; the
/// inner function uses `?` to short-circuit on the first failure.
///
/// Workers consume via `submit_alias(...).await.audit(...).await?` —
/// see `services::upstream_error::CallOutcome` for the chaining
/// pattern.
pub async fn submit_alias(
    http: &Client,
    abogus_url: &str,
    cookie_header: &str,
    book_id: &str,
    alias_name: &str,
    alias_type: i32,
) -> CallOutcome<String> {
    let mut snap = ResponseSnapshot::default();
    let result = submit_alias_inner(
        http,
        abogus_url,
        cookie_header,
        book_id,
        alias_name,
        alias_type,
        &mut snap,
    )
    .await;
    CallOutcome::wrap(result, snap)
}

async fn submit_alias_inner(
    http: &Client,
    abogus_url: &str,
    cookie_header: &str,
    book_id: &str,
    alias_name: &str,
    alias_type: i32,
    snap: &mut ResponseSnapshot,
) -> Result<String, UpstreamError> {
    let body = json!({
        "book_id": book_id,
        "alias_type": alias_type,
        "alias_name": alias_name,
        "metrics_data": {
            "app_id": "457699",
            "create_entrance": "book_list",
            "create_page": "book_list",
            "app_name": "fanqie_novel",
            "genre": "0",
            "sub_page_name": "爆款榜",
            "book_id": book_id,
            "is_recommend": "0",
        },
        "alias_post_type": 3,
    });
    let body_str = body.to_string();

    let signed = sign_url(http, abogus_url, BASE_URL, &body_str, TOMATO_UA)
        .await
        .map_err(UpstreamError::Sign)?;

    let body_text = post_signed(http, &signed, cookie_header, body_str, snap).await?;
    let envelope: ApiEnvelope =
        serde_json::from_str(&body_text).map_err(|e| UpstreamError::Parse(e.to_string()))?;

    let reason = || {
        envelope
            .data
            .as_ref()
            .and_then(|d| d.reason.as_ref())
            .map(|r| r.to_string())
            .unwrap_or_default()
    };
    match envelope.code {
        // ── Known success ──────────────────────────────────────────────
        0 => {}
        // ── Known business errors ──────────────────────────────────────
        // 30001: 别名审核不通过 — permanent, do not retry
        30001 => return Err(UpstreamError::ApiCode {
            code: 30001,
            message: format!("别名审核不通过 reason={}", reason()),
        }),
        // 10004: 平台内部错误 — transient, caller retries
        10004 => return Err(UpstreamError::ApiCode {
            code: 10004,
            message: format!("内部错误 reason={}", reason()),
        }),
        // ── Unknown — log and surface for classification ────────────────
        other => return Err(UpstreamError::ApiCode {
            code: other,
            message: format!("[UNKNOWN CODE {other}] {} reason={}", envelope.message, reason()),
        }),
    }

    envelope
        .data
        .and_then(|d| d.alias_id)
        .filter(|s| !s.is_empty())
        .filter(|s| s != "0")
        .ok_or(UpstreamError::MissingField("alias_id"))
}

/// Shared "POST signed URL with cookie + standard fanqie headers" helper.
/// Reads the body, fills in `snap.http_status` / `body_text` /
/// `body_json`, returns the body text on 2xx or an `UpstreamError`
/// (auth-failed for 401/403, http-error for other 4xx/5xx).
async fn post_signed(
    http: &Client,
    signed_url: &str,
    cookie_header: &str,
    body: String,
    snap: &mut ResponseSnapshot,
) -> Result<String, UpstreamError> {
    let res = http
        .post(signed_url)
        .headers(fanqie_base_headers())
        .header(reqwest::header::USER_AGENT, TOMATO_UA)
        .header(reqwest::header::COOKIE, cookie_header)
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

/// GET-equivalent of `post_signed`. Used by `query_alias_status`.
async fn get_signed(
    http: &Client,
    signed_url: &str,
    cookie_header: &str,
    snap: &mut ResponseSnapshot,
) -> Result<String, UpstreamError> {
    let res = http
        .get(signed_url)
        .headers(fanqie_base_headers())
        .header(reqwest::header::USER_AGENT, TOMATO_UA)
        .header(reqwest::header::COOKIE, cookie_header)
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

/// Headers sent on every fanqie call. Static; allocated fresh because
/// `HeaderMap` doesn't `Copy` and Clone is roughly the same cost.
fn fanqie_base_headers() -> HeaderMap {
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
    h.insert("priority", HeaderValue::from_static("u=1, i"));
    h.insert("referer", HeaderValue::from_static(REFERER));
    h.insert("origin", HeaderValue::from_static("https://kol.fanqieopen.com"));
    h.insert("content-type", HeaderValue::from_static("application/json"));
    h.insert("sec-ch-ua", HeaderValue::from_static(TOMATO_SEC_CH_UA));
    h.insert(
        "sec-ch-ua-mobile",
        HeaderValue::from_static(TOMATO_SEC_CH_UA_MOBILE),
    );
    h.insert(
        "sec-ch-ua-platform",
        HeaderValue::from_static(TOMATO_SEC_CH_UA_PLATFORM),
    );
    h.insert("sec-fetch-dest", HeaderValue::from_static("empty"));
    h.insert("sec-fetch-mode", HeaderValue::from_static("cors"));
    h.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
    h.insert("x-kol-token", HeaderValue::from_static("undefined"));
    h
}

#[derive(Debug, Deserialize)]
struct PostCreateEnvelope {
    code: i32,
    #[serde(default)]
    message: String,
}

/// Submit one post (Douyin link) against a previously-submitted alias.
/// Success body has no useful payload — only validates `code == 0`.
///
/// `post_date` is `YYYY-MM-DD` in the platform's local timezone.
pub async fn submit_post(
    http: &Client,
    abogus_url: &str,
    cookie_header: &str,
    alias_id: &str,
    alias_type: i32,
    post_link: &str,
    post_date: &str,
) -> CallOutcome<()> {
    let mut snap = ResponseSnapshot::default();
    let result = submit_post_inner(
        http,
        abogus_url,
        cookie_header,
        alias_id,
        alias_type,
        post_link,
        post_date,
        &mut snap,
    )
    .await;
    CallOutcome::wrap(result, snap)
}

async fn submit_post_inner(
    http: &Client,
    abogus_url: &str,
    cookie_header: &str,
    alias_id: &str,
    alias_type: i32,
    post_link: &str,
    post_date: &str,
    snap: &mut ResponseSnapshot,
) -> Result<(), UpstreamError> {
    let body = json!({
        "alias_id": alias_id,
        "post_records": [{
            "post_date": post_date,
            "post_link": post_link,
        }],
        "alias_type": alias_type,
        "promotion_type": 1,
        "alias_post_type": 3,
    });
    let body_str = body.to_string();

    let signed = sign_url(http, abogus_url, POST_CREATE_BASE_URL, &body_str, TOMATO_UA)
        .await
        .map_err(UpstreamError::Sign)?;

    let body_text = post_signed(http, &signed, cookie_header, body_str, snap).await?;
    let envelope: PostCreateEnvelope =
        serde_json::from_str(&body_text).map_err(|e| UpstreamError::Parse(e.to_string()))?;

    match envelope.code {
        // ── Known success ──────────────────────────────────────────────
        0 => Ok(()),
        // ── Known business errors ──────────────────────────────────────
        // 10004: 推广计划不存在或当前状态无法回填 — retry with cooldown
        10004 => Err(UpstreamError::ApiCode {
            code: 10004,
            message: envelope.message,
        }),
        // ── Unknown ────────────────────────────────────────────────────
        other => Err(UpstreamError::ApiCode {
            code: other,
            message: format!("[UNKNOWN CODE {other}] {}", envelope.message),
        }),
    }
}

/// Subset of the per-alias item returned by `promotion/plan/list`. Only
/// the fields backfill_submitter actually uses — the upstream returns
/// many more (book_name, thumb_url, …) that the audit log keeps as raw
/// JSON if anything ever needs them.
#[derive(Debug, Clone, Deserialize)]
pub struct AliasStatusInfo {
    /// Echoed back so a future cross-check can verify we got the row
    /// we expected; not currently consumed but worth keeping in the
    /// shape so we notice if the upstream stops sending it.
    #[allow(dead_code)]
    pub alias_id: String,
    pub alias_status: i32,
    /// Why the platform rejected the alias, if it did. The upstream
    /// uses `null` for non-rejected statuses (生效中 / 待审核 / 即将
    /// 失效) and an array of reasons for rejected ones (审核不通过 /
    /// 强制失效). Optional + default so both shapes deserialize.
    #[serde(default)]
    pub audit_reason: Option<Vec<String>>,
}

impl AliasStatusInfo {
    /// Convenience accessor — null/missing audit_reason is treated as
    /// "no reason", same as an empty array. Most call sites only care
    /// about the strings themselves.
    pub fn audit_reasons(&self) -> &[String] {
        self.audit_reason.as_deref().unwrap_or(&[])
    }
}

#[derive(Debug, Deserialize)]
struct PlanListEnvelope {
    code: i32,
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: Option<PlanListData>,
}

#[derive(Debug, Deserialize)]
struct PlanListData {
    #[serde(default)]
    promotion_list: Vec<AliasStatusInfo>,
}

/// Look up the platform-side status for a single (alias_name, task_type)
/// pair. Used by backfill_submitter to gate post/create calls — we
/// only proceed when the alias is in `ALIAS_STATUS_ACTIVE` (1) or
/// `ALIAS_STATUS_EXPIRING` (6); other states are either terminal
/// (2/4/5) or still under review (3).
///
/// Returns `Ok(Some(_))` if the platform has a record for this pair,
/// `Ok(None)` if the list came back empty (alias not found —
/// shouldn't happen in normal flow but worth distinguishing from
/// transport errors), `Err` for everything else (signing, HTTP,
/// JSON parse, code != 0).
pub async fn query_alias_status(
    http: &Client,
    abogus_url: &str,
    cookie_header: &str,
    alias_name: &str,
    task_type: i32,
) -> CallOutcome<Option<AliasStatusInfo>> {
    let mut snap = ResponseSnapshot::default();
    let result = query_alias_status_inner(
        http,
        abogus_url,
        cookie_header,
        alias_name,
        task_type,
        &mut snap,
    )
    .await;
    CallOutcome::wrap(result, snap)
}

async fn query_alias_status_inner(
    http: &Client,
    abogus_url: &str,
    cookie_header: &str,
    alias_name: &str,
    task_type: i32,
    snap: &mut ResponseSnapshot,
) -> Result<Option<AliasStatusInfo>, UpstreamError> {
    // Platform expects alias_name percent-encoded (Chinese keywords).
    // All other params are static or small integers.
    let encoded_name = urlencoding::encode(alias_name);
    let base_url = format!(
        "{PLAN_LIST_BASE_URL}&alias_name={encoded_name}&task_type={task_type}"
    );
    let signed = sign_url(http, abogus_url, &base_url, "", TOMATO_UA)
        .await
        .map_err(UpstreamError::Sign)?;

    let body_text = get_signed(http, &signed, cookie_header, snap).await?;
    let envelope: PlanListEnvelope =
        serde_json::from_str(&body_text).map_err(|e| UpstreamError::Parse(e.to_string()))?;

    match envelope.code {
        // ── Known success ──────────────────────────────────────────────
        0 => {}
        // ── Unknown ────────────────────────────────────────────────────
        other => return Err(UpstreamError::ApiCode {
            code: other,
            message: format!("[UNKNOWN CODE {other}] {}", envelope.message),
        }),
    }

    Ok(envelope
        .data
        .map(|d| d.promotion_list)
        .unwrap_or_default()
        .into_iter()
        .next())
}
