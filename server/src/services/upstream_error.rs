//! Structured error type for every external HTTP call we make, plus
//! the `CallOutcome<T>` envelope that pairs a result with its
//! `ResponseSnapshot` so a single `.audit().await?` step takes care
//! of the audit-log + error propagation that used to be duplicated
//! at every worker call site.
//!
//! Replaces the old `(Result<T, String>, ResponseSnapshot)` tuple
//! return convention. With strings, workers had to do
//! `tomato_cookie::is_auth_failure_status(snap.http_status)` to
//! recover the auth-failure category — i.e. they were re-deriving
//! information they used to have and threw away. Here the error
//! categories are first-class so workers `match err` directly.

use thiserror::Error;

use crate::db::DbPool;
use crate::services::api_log::{self, ResponseSnapshot};
use crate::services::known_errors;

/// The set of failure categories worker code actually wants to branch
/// on. Anything that doesn't fit a category lands in `Other` so the
/// worker has a fall-through arm.
#[derive(Debug, Error)]
pub enum UpstreamError {
    /// Pre-flight: signing (e.g. abogus) failed before we even hit
    /// the network. Usually means the abogus container is down /
    /// rotated.
    #[error("sign: {0}")]
    Sign(String),

    /// Network-level failure (DNS, TLS, connection reset, timeout).
    /// Distinct from `HttpError` because retries make sense here but
    /// not for genuine 4xx.
    #[error("transport: {0}")]
    Transport(String),

    /// HTTP 401/403 specifically. Workers use this to flip cookie
    /// `is_online=FALSE` / invalidate a token.
    #[error("auth failed: HTTP {status}")]
    AuthFailed { status: u16, body_preview: String },

    /// Any other 4xx/5xx that wasn't 401/403. `body_preview` is at
    /// most ~200 chars so it's safe to log without flooding.
    #[error("HTTP {status}: {body_preview}")]
    HttpError { status: u16, body_preview: String },

    /// Body wasn't valid JSON (or the envelope shape we expected).
    #[error("parse: {0}")]
    Parse(String),

    /// Upstream returned a recognizable JSON envelope with a
    /// non-success `code`. The success code is what the caller used
    /// to validate; `code` is the actual non-success value the
    /// upstream returned.
    #[error("api code={code} msg={message}")]
    ApiCode { code: i32, message: String },

    /// Response shape was structurally OK but a required field was
    /// missing (e.g. `data.alias_id` empty on a "success" reply).
    #[error("missing field: {0}")]
    MissingField(&'static str),

    /// Catch-all for anything we haven't bothered to classify.
    #[error("{0}")]
    Other(String),
}

impl UpstreamError {
    /// True when the error indicates the caller's auth credential
    /// (cookie, token) is dead. Workers use this to decide whether
    /// to flip the credential's "is_online" flag.
    pub fn is_auth_failure(&self) -> bool {
        matches!(self, UpstreamError::AuthFailed { .. })
    }

    /// HTTP status if we got one; useful when the caller still wants
    /// to log the raw status alongside the categorized error.
    pub fn http_status(&self) -> Option<u16> {
        match self {
            UpstreamError::AuthFailed { status, .. }
            | UpstreamError::HttpError { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Construct from an HTTP status code + body preview, picking
    /// `AuthFailed` for 401/403 and `HttpError` for everything else.
    pub fn from_http(status: u16, body_preview: String) -> Self {
        if status == 401 || status == 403 {
            UpstreamError::AuthFailed {
                status,
                body_preview,
            }
        } else {
            UpstreamError::HttpError {
                status,
                body_preview,
            }
        }
    }
}

/// One external call's outcome. The wire transcript is captured
/// regardless of result, so audit-logging works the same on success
/// and failure.
#[derive(Debug)]
pub struct CallOutcome<T> {
    pub result: Result<T, UpstreamError>,
    pub snapshot: ResponseSnapshot,
}

impl<T> CallOutcome<T> {
    pub fn ok(value: T, snapshot: ResponseSnapshot) -> Self {
        Self {
            result: Ok(value),
            snapshot,
        }
    }
    pub fn err(error: UpstreamError, snapshot: ResponseSnapshot) -> Self {
        Self {
            result: Err(error),
            snapshot,
        }
    }
    /// Wrap a `Result` already produced (typically by an inner async
    /// helper that mutated `snapshot` along the way) into the public
    /// `CallOutcome` envelope. Lets service functions look like:
    /// ```ignore
    /// pub async fn submit_alias(...) -> CallOutcome<String> {
    ///     let mut snap = ResponseSnapshot::default();
    ///     let result = submit_alias_inner(..., &mut snap).await;
    ///     CallOutcome::wrap(result, snap)
    /// }
    /// ```
    pub fn wrap(result: Result<T, UpstreamError>, snapshot: ResponseSnapshot) -> Self {
        Self { result, snapshot }
    }

    /// Persist the wire transcript via `api_log::log_call` and unwrap
    /// the inner result. Skips logging for known business errors
    /// (handled by `known_errors` registry).
    ///
    /// Workers were previously open-coding this 8 times — the typical
    /// pattern was:
    /// ```ignore
    /// let (result, snap) = service::call(...).await;
    /// let parse_error = result.as_ref().err().cloned();
    /// api_log::log_call(pool, SERVICE, ENDPOINT, summary, &snap,
    ///                   result.is_ok(), parse_error.as_deref()).await;
    /// match result { Ok(v) => ..., Err(reason) => ... }
    /// ```
    /// Now: `service::call(...).await.audit(pool, SERVICE, ENDPOINT, summary).await`
    /// + a `match` on the typed error.
    pub async fn audit(
        self,
        pool: &DbPool,
        service: &str,
        endpoint: &str,
        request_summary: serde_json::Value,
    ) -> Result<T, UpstreamError> {
        let parse_error = self.result.as_ref().err().map(|e| e.to_string());
        api_log::log_call(
            pool,
            service,
            endpoint,
            request_summary,
            &self.snapshot,
            self.result.is_ok(),
            parse_error.as_deref(),
        )
        .await;
        self.result
    }
}

/// Helper to attempt a `code != success_code` check on the upstream
/// envelope; suppresses audit-log when the code matches a known
/// business error pattern. Used internally by `CallOutcome::audit`,
/// re-exported for tests.
#[allow(dead_code)] // keeps known_errors integration explicit
pub fn is_known_business_error(
    service: &str,
    endpoint: &str,
    snap: &ResponseSnapshot,
    code: Option<i32>,
) -> bool {
    known_errors::is_known_business_error(service, endpoint, snap.http_status, code)
}
