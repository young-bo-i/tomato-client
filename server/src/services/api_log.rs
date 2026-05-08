//! Helper for capturing third-party API responses into the
//! `external_api_responses` table. Every external HTTP client we
//! write should funnel its response through here so we have a single
//! audit trail to diff against expected shapes.
//!
//! Convention: the upstream client returns a tuple
//! `(Result<T, String>, ResponseSnapshot)`. The Ok side is the
//! extracted value the caller cares about; the snapshot carries
//! the verbatim wire data so logging never needs to re-parse.
//!
//! ## Sampling
//!
//! Successful responses (parsed_ok=true) are by far the most common —
//! workers calling tomato/qimao at sustained 5–15 audits/sec produce
//! 200k–500k rows/day, dominated by happy-path success. We sample
//! these at 1/N (default N=10, env `KOL_API_LOG_SAMPLE_OK_RATE`) using
//! a deterministic atomic counter so every Nth success row is kept.
//!
//! Failures (parsed_ok=false) are NEVER sampled — they are the
//! signal we actually want for triage. Sampled-out success rows still
//! contribute to per-row business state (worker writes the alias row
//! status separately), so no operational data is lost.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use serde_json::{json, Value as JsonValue};

use crate::db::DbPool;
use crate::services::known_errors;

/// Sample 1 in N successful audits. Default 10 (90% reduction in
/// happy-path log volume); failures are always written.
static SAMPLE_OK_RATE: OnceLock<u64> = OnceLock::new();
static SAMPLE_OK_COUNTER: AtomicU64 = AtomicU64::new(0);

fn sample_ok_rate() -> u64 {
    *SAMPLE_OK_RATE.get_or_init(|| {
        std::env::var("KOL_API_LOG_SAMPLE_OK_RATE")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|n| *n >= 1)
            .unwrap_or(10)
    })
}

/// Returns true when this success row should be persisted.
/// Always true when sample rate is 1; otherwise true once every N calls.
fn should_keep_ok() -> bool {
    let rate = sample_ok_rate();
    if rate <= 1 {
        return true;
    }
    let n = SAMPLE_OK_COUNTER.fetch_add(1, Ordering::Relaxed);
    n % rate == 0
}

/// Verbatim per-call HTTP context. Fields are `None` when we never
/// reached that stage (e.g. signing failed → no http_status).
#[derive(Debug, Default, Clone)]
pub struct ResponseSnapshot {
    pub http_status: Option<u16>,
    pub body_text: Option<String>,
    /// `Some(json)` if the body parsed as JSON, `None` otherwise. The
    /// log writer prefers JSONB for queryability and falls back to
    /// `{"raw_text": ...}` only when the body wasn't valid JSON.
    pub body_json: Option<JsonValue>,
}

/// Insert one audit row. Best-effort: failures are logged but not
/// returned — we never want logging to take down a worker that's
/// otherwise succeeding.
pub async fn log_call(
    pool: &DbPool,
    service: &str,
    endpoint: &str,
    request_summary: JsonValue,
    snap: &ResponseSnapshot,
    parsed_ok: bool,
    parse_error: Option<&str>,
) {
    // Bail out early on already-classified business errors. The per-row
    // `error_reason` on the worker's domain table (e.g. tomato_aliases)
    // still captures what happened; we just don't pile up duplicate
    // audit rows for patterns we've already triaged. See
    // `services::known_errors` for the registry and rules for adding to it.
    let code = extract_response_code(snap);
    if !parsed_ok
        && known_errors::is_known_business_error(service, endpoint, snap.http_status, code)
    {
        return;
    }

    // Sampling: drop most success rows, keep all failures. With ~5-15
    // audit calls/sec sustained from background workers, dominated by
    // happy-path success, this trims the table by an order of magnitude.
    if parsed_ok && !should_keep_ok() {
        return;
    }

    // Prefer JSON if we got JSON; otherwise wrap the raw text so the
    // column stays JSONB-typed for querying. Truncate large bodies (e.g.
    // HTML error pages from WAF/CDN) so one runaway response can't
    // create a bloated row.
    const MAX_BODY_CHARS: usize = 8_000;
    let raw_response: JsonValue = match (&snap.body_json, &snap.body_text) {
        (Some(j), _) => j.clone(),
        (None, Some(t)) => {
            let truncated: String = t.chars().take(MAX_BODY_CHARS).collect();
            json!({ "raw_text": truncated })
        }
        (None, None) => JsonValue::Null,
    };

    let result = sqlx::query(
        r#"INSERT INTO external_api_responses
              (service, endpoint, request_summary, http_status,
               raw_response, parsed_ok, parse_error)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
    )
    .bind(service)
    .bind(endpoint)
    .bind(&request_summary)
    .bind(snap.http_status.map(|s| s as i32))
    .bind(&raw_response)
    .bind(parsed_ok)
    .bind(parse_error)
    .execute(pool)
    .await;

    if let Err(e) = result {
        // Surface but don't propagate — losing one audit row is far
        // less bad than wedging a worker.
        tracing::warn!(
            "api_log: failed to record {service}/{endpoint}: {e}"
        );
    }
}

/// Pull the upstream's `code` field out of the parsed body, if present.
/// All the APIs we currently audit follow the `{ "code": <int>, ... }`
/// envelope convention; if this stops being true we'll need a per-service
/// extractor.
fn extract_response_code(snap: &ResponseSnapshot) -> Option<i32> {
    snap.body_json
        .as_ref()?
        .get("code")?
        .as_i64()
        .map(|n| n as i32)
}
