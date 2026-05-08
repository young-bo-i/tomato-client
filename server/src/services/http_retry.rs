//! Shared retry policy for upstream HTTP calls (fanqie + qimao).
//!
//! Retries ONLY on connection-establishment failures — i.e. the request
//! never landed on the server. We deliberately do NOT retry on:
//!   * 5xx responses
//!   * 408/429 responses
//!   * read-timeouts after the request was written
//!
//! Reason: every promotion endpoint we touch is non-idempotent
//! (`add_keywords`, `add_keyword_links`, `submit_alias`, `submit_post`),
//! so a retry on 5xx risks double-submitting an alias or post-link.
//! Connection-establishment errors are the only class where the server
//! provably did not see the request, making retry safe.
//!
//! Backoff: 200ms → 1s → 5s, max 3 attempts (i.e. 2 retries). The
//! upstream is healthy in the common case; this only smooths over brief
//! DNS / TCP hiccups.

use std::time::Duration;

use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{
    policies::ExponentialBackoff, RetryTransientMiddleware, Retryable, RetryableStrategy,
};

/// Wrap a `reqwest::Client` with our connection-only retry middleware.
pub fn with_connect_retries(inner: reqwest::Client) -> ClientWithMiddleware {
    let policy = ExponentialBackoff::builder()
        .retry_bounds(Duration::from_millis(200), Duration::from_secs(5))
        .build_with_max_retries(2);
    let strategy = RetryTransientMiddleware::new_with_policy_and_strategy(policy, ConnectOnly);
    ClientBuilder::new(inner).with(strategy).build()
}

/// Strategy: retry only when reqwest reports `is_connect()`. Everything
/// else (HTTP status codes, read timeouts, body decode errors) is left
/// alone — the worker will surface it via `UpstreamError`.
struct ConnectOnly;

impl RetryableStrategy for ConnectOnly {
    fn handle(
        &self,
        res: &Result<reqwest::Response, reqwest_middleware::Error>,
    ) -> Option<Retryable> {
        match res {
            Ok(_) => None,
            Err(reqwest_middleware::Error::Reqwest(e)) if e.is_connect() => {
                Some(Retryable::Transient)
            }
            Err(_) => None,
        }
    }
}
