//! Client for the 七猫达人 site-wide message feed:
//!
//!   GET https://kol.wtzw.com/api/v1/message/notice/list
//!     ?start_time=&end_time=&title=&page=1&page_size=50
//!
//! Authenticated with `x-qm-devops-token` (per-profile token, refreshed
//! by `jobs::qimao_token_refresh`). Returns a paginated list of notices
//! sorted newest-first. Each notice has `id`, `title`, `content` (HTML
//! string with inline styles), `status`, `create_time` ("YYYY-MM-DD").
//!
//! Used by `jobs::qimao_income_notice` to find the monthly
//! "X月KOC七猫免费小说收益明细" notices and forward them as email to
//! the profile owner.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue};
use reqwest_middleware::ClientWithMiddleware as Client;
use serde::Deserialize;

use crate::services::api_log::ResponseSnapshot;
use crate::services::upstream_error::{CallOutcome, UpstreamError};

pub const SERVICE_NAME: &str = "qimao_message";
pub const ENDPOINT_NOTICE_LIST: &str = "message/notice/list";

/// Page 1, size 50 — empirically large enough to cover the income
/// notice plus the noisy "keyword expired" messages that pile up
/// between checks. If the platform ever tightens this, paginate.
const NOTICE_LIST_URL: &str = concat!(
    "https://kol.wtzw.com/api/v1/message/notice/list",
    "?start_time=&end_time=&title=&page=1&page_size=50"
);

const REFERER: &str = "https://dmp.wtzw.com/";
const ORIGIN: &str = "https://dmp.wtzw.com";
const QIMAO_UA: &str = concat!(
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 ",
    "(KHTML, like Gecko) Chrome/147.0.0.0 Safari/537.36"
);

#[derive(Debug, Clone, Deserialize)]
pub struct MessageItem {
    pub id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub status: i32,
    /// "YYYY-MM-DD" — the upstream sends a date string, not a
    /// timestamp. Parsing into `chrono::NaiveDate` is the caller's
    /// responsibility (formats: see chrono::NaiveDate::parse_from_str).
    #[serde(default)]
    pub create_time: String,
}

#[derive(Debug, Deserialize)]
struct MessageListEnvelope {
    code: i32,
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: Option<MessageListData>,
}

#[derive(Debug, Deserialize)]
struct MessageListData {
    #[serde(default)]
    list: Vec<MessageItem>,
}

/// Process-wide HTTP client for the message feed. Connect-only retry —
/// GET is idempotent so retrying-on-5xx would be safe, but staying
/// consistent with the other qimao clients keeps surprise low.
static HTTP: once_cell::sync::Lazy<Client> = once_cell::sync::Lazy::new(|| {
    let inner = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("qimao_message reqwest::Client init");
    crate::services::http_retry::with_connect_retries(inner)
});

pub fn build_http_client() -> Result<Client, String> {
    Ok(HTTP.clone())
}

/// `GET /api/v1/message/notice/list?...&page=1&page_size=50`. Returns
/// the latest 50 notices for the calling token's account.
pub async fn list_notices(http: &Client, token: &str) -> CallOutcome<Vec<MessageItem>> {
    let mut snap = ResponseSnapshot::default();
    let result = list_notices_inner(http, token, &mut snap).await;
    CallOutcome::wrap(result, snap)
}

async fn list_notices_inner(
    http: &Client,
    token: &str,
    snap: &mut ResponseSnapshot,
) -> Result<Vec<MessageItem>, UpstreamError> {
    let mut headers = qimao_headers(token);
    headers.insert("sec-fetch-dest", HeaderValue::from_static("empty"));
    headers.insert("sec-fetch-mode", HeaderValue::from_static("cors"));
    headers.insert("sec-fetch-site", HeaderValue::from_static("same-site"));

    let res = http
        .get(NOTICE_LIST_URL)
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

    let env: MessageListEnvelope =
        serde_json::from_str(&body_text).map_err(|e| UpstreamError::Parse(e.to_string()))?;

    match env.code {
        // qimao convention: code=200 = success.
        200 => Ok(env.data.map(|d| d.list).unwrap_or_default()),
        other => Err(UpstreamError::ApiCode {
            code: other,
            message: format!("[UNKNOWN CODE {other}] {}", env.message),
        }),
    }
}

/// Headers expected by the qimao backend. Mirrors the other qimao
/// services (qimao_promotion::base_headers) but redeclared here to
/// avoid leaking that module's private helper.
fn qimao_headers(token: &str) -> HeaderMap {
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
    h.insert(
        "sec-ch-ua",
        HeaderValue::from_static(
            "\"Google Chrome\";v=\"147\", \"Not.A/Brand\";v=\"8\", \"Chromium\";v=\"147\"",
        ),
    );
    h.insert("sec-ch-ua-mobile", HeaderValue::from_static("?0"));
    h.insert("sec-ch-ua-platform", HeaderValue::from_static("\"macOS\""));
    if let Ok(v) = HeaderValue::from_str(token) {
        h.insert("x-qm-devops-token", v);
    }
    h
}
