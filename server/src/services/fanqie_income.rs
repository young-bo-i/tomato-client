//! Client for the 番茄达人 income/stats endpoint:
//!
//!   GET https://kol.fanqieopen.com/api/platform/user/income/stats/v:version
//!     ?app_id=457699&aid=457699&origin_app_id=457699&host_app_id=457699
//!     &msToken=...&a_bogus=...
//!
//! Per-tomato-account "我的收益" snapshot. Polled every 10 minutes by
//! `jobs::tomato_income`. Identical signing dance to the rank/promotion
//! endpoints (abogus + cookie). Response envelope:
//!
//!   { code: 0, message: "", log_id: "...", data: { total_income, ... } }
//!
//! All amounts in 分 (cents). `latest_update_time` is unix seconds.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue};
use reqwest_middleware::ClientWithMiddleware as Client;
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::services::abogus::{
    sign_url, TOMATO_SEC_CH_UA, TOMATO_SEC_CH_UA_MOBILE, TOMATO_SEC_CH_UA_PLATFORM, TOMATO_UA,
};
use crate::services::api_log::ResponseSnapshot;
use crate::services::upstream_error::{CallOutcome, UpstreamError};

/// Distinct service name in the audit log so income-fetch errors don't
/// blend into promotion endpoints when admins filter by service.
pub const SERVICE_NAME: &str = "fanqie_income";
pub const ENDPOINT_INCOME_STATS: &str = "user/income/stats";

const REFERER: &str = "https://kol.fanqieopen.com/page/income";
const BASE_URL: &str = concat!(
    "https://kol.fanqieopen.com/api/platform/user/income/stats/v:version",
    "?app_id=457699&aid=457699&origin_app_id=457699&host_app_id=457699"
);

/// Process-wide HTTP client for the income endpoint, mirroring the
/// pattern in `fanqie_promotion`. Connect-only retry — GET is
/// idempotent so we *could* loosen this to retry-on-5xx, but staying
/// consistent with the other fanqie clients keeps surprise low.
static HTTP: once_cell::sync::Lazy<Client> = once_cell::sync::Lazy::new(|| {
    let inner = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("fanqie_income reqwest::Client init");
    crate::services::http_retry::with_connect_retries(inner)
});

pub fn build_http_client() -> Result<Client, String> {
    Ok(HTTP.clone())
}

/// Parsed income payload — flat fields the poller actually needs to
/// diff/persist, plus the verbatim arrays we store as JSONB without
/// re-parsing. Anything else upstream sends ends up in `raw` via the
/// outer `IncomeRecord`.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct IncomeData {
    #[serde(default)]
    pub total_income: i64,
    #[serde(default)]
    pub regular_income: i64,
    #[serde(default)]
    pub bonus_income: i64,
    #[serde(default)]
    pub current_week_income: i64,
    #[serde(default)]
    pub current_month_income: i64,
    /// Unix seconds. 0 when upstream hasn't computed any income yet
    /// (brand-new account); the poller treats 0 as "no LUT" and skips
    /// the skew/idempotency gates for that row.
    #[serde(default)]
    pub latest_update_time: i64,
    /// Lists kept verbatim — the UI renders them as-is.
    #[serde(default)]
    pub weekly_income_list: JsonValue,
    #[serde(default)]
    pub monthly_income_list: JsonValue,
    #[serde(default)]
    pub task_income_list: JsonValue,
}

/// What the service hands back to the poller: the parsed convenience
/// view plus the raw `data` object for forensic storage. Both come
/// from a single deserialization pass.
#[derive(Debug, Clone)]
pub struct IncomeRecord {
    pub data: IncomeData,
    pub raw: JsonValue,
}

#[derive(Debug, Deserialize)]
struct IncomeEnvelope {
    code: i32,
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: Option<JsonValue>,
}

/// Fetch one account's income snapshot. Returns `CallOutcome` so the
/// caller can `.audit(...)` to the external_api_responses table in
/// the same `match` it uses to handle the typed error.
pub async fn fetch_income(
    http: &Client,
    abogus_url: &str,
    cookie_header: &str,
) -> CallOutcome<IncomeRecord> {
    let mut snap = ResponseSnapshot::default();
    let result = fetch_income_inner(http, abogus_url, cookie_header, &mut snap).await;
    CallOutcome::wrap(result, snap)
}

async fn fetch_income_inner(
    http: &Client,
    abogus_url: &str,
    cookie_header: &str,
    snap: &mut ResponseSnapshot,
) -> Result<IncomeRecord, UpstreamError> {
    let signed = sign_url(http, abogus_url, BASE_URL, "", TOMATO_UA)
        .await
        .map_err(UpstreamError::Sign)?;

    let res = http
        .get(&signed)
        .headers(income_base_headers())
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

    let envelope: IncomeEnvelope =
        serde_json::from_str(&body_text).map_err(|e| UpstreamError::Parse(e.to_string()))?;

    if envelope.code != 0 {
        return Err(UpstreamError::ApiCode {
            code: envelope.code,
            message: format!("[UNKNOWN CODE {}] {}", envelope.code, envelope.message),
        });
    }

    let raw = envelope.data.unwrap_or(JsonValue::Null);
    // Convert the raw JSON `data` object back to an IncomeData. Cheap
    // (the same JSON was already parsed once into the envelope) and
    // keeps a single source-of-truth for `raw` rather than parsing
    // body_text twice.
    let data: IncomeData = serde_json::from_value(raw.clone())
        .map_err(|e| UpstreamError::Parse(format!("data: {e}")))?;

    Ok(IncomeRecord { data, raw })
}

/// Headers captured from a real browser session against the income
/// page. Cookie + UA are added per-call.
fn income_base_headers() -> HeaderMap {
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
    // Real browser sent the literal string "undefined" for this header
    // on this page too — keep it for signature parity with promotion
    // endpoints.
    h.insert("x-kol-token", HeaderValue::from_static("undefined"));
    h
}
