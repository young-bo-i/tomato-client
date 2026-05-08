//! Registry of upstream API error patterns we've already learned and
//! decided to *stop* re-recording into `external_api_responses`.
//!
//! ## Why this exists
//!
//! The audit table was built to surface response shapes we *don't*
//! understand yet. Once a `(service, endpoint, code)` combination has
//! a known meaning and is acted on by the caller (e.g. the alias row
//! is moved to `failed` with `error_reason` filled in), continuing
//! to log every occurrence is just noise — the per-row reason is
//! already preserved on `tomato_aliases`, and the audit table balloons
//! with the same N rows we've already triaged.
//!
//! ## What goes in here
//!
//! Only **business-layer rejections** the platform returns with HTTP
//! 200 + a non-zero `code` field. Specifically NOT:
//!
//! - HTTP transport / TLS / DNS errors (we want these — they signal
//!   real outages)
//! - HTTP 4xx/5xx (auth/upstream failures — also want these)
//! - Successful responses (HTTP 200 + code=0) — those are the shape
//!   contract; if the upstream changes the success body we want to
//!   notice
//! - New, never-seen `code` values — let them accumulate so we can
//!   classify them later
//!
//! ## Adding new entries
//!
//! Add a `(service, endpoint, code)` tuple here once you've confirmed
//! the meaning AND that the calling worker handles the failure
//! correctly (records reason, moves row to a terminal state, doesn't
//! retry pointlessly). The `note` field is for humans skimming the
//! list — it doesn't affect logic.

/// One known business-error pattern to suppress from audit logging.
///
/// `success_code` matters because each platform uses a different
/// success marker — fanqie returns `code=0` on success, qimao returns
/// `code=200`. Without it, registering a fanqie known-error of code=0
/// would also accidentally match qimao's success responses.
struct KnownError {
    service: &'static str,
    endpoint: &'static str,
    /// The platform's "success" code value. We'll only suppress audit
    /// rows when the response code is non-success AND matches `code`.
    success_code: i32,
    /// The non-success code we recognize as already-classified.
    code: i32,
    /// Human note for future maintainers; not used by code.
    #[allow(dead_code)]
    note: &'static str,
}

/// The full registry. Append-only as we triage new error codes.
const KNOWN: &[KnownError] = &[
    KnownError {
        service: "fanqie_promotion",
        endpoint: "promotion/plan/create",
        success_code: 0,
        code: 30001,
        note: "别名审核不通过 — handled by alias_submitter (row → failed, reason recorded)",
    },
    KnownError {
        service: "fanqie_promotion",
        endpoint: "promotion/plan/create",
        success_code: 0,
        code: 10004,
        note: "平台内部错误(内部错误) — alias_submitter leaves row pending and retries next round",
    },
    KnownError {
        service: "fanqie_promotion",
        endpoint: "promotion/post/create",
        success_code: 0,
        code: 10004,
        note: "推广计划不存在或当前状态无法回填 — backfill_submitter retries with cooldown",
    },
    // qimao would register success_code: 200 entries here when we have
    // enough samples to classify recurring rejection codes.
];

/// Returns true if this `(service, endpoint, code)` combination is
/// already classified and can be safely dropped from the audit log.
///
/// `code` comes from the JSON response body. Only meaningful when the
/// upstream returned HTTP 200 (a 4xx/5xx is a separate signal we want
/// to keep regardless of what's in the body).
///
/// We only suppress when `code != success_code` for the matched
/// registry entry — guards against a platform-A "known business
/// error" code accidentally matching platform-B's success code.
pub fn is_known_business_error(
    service: &str,
    endpoint: &str,
    http_status: Option<u16>,
    code: Option<i32>,
) -> bool {
    if http_status != Some(200) {
        return false;
    }
    let Some(code) = code else { return false };
    KNOWN.iter().any(|k| {
        k.service == service
            && k.endpoint == endpoint
            && k.code == code
            && k.code != k.success_code
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_alias_review_is_suppressed() {
        assert!(is_known_business_error(
            "fanqie_promotion",
            "promotion/plan/create",
            Some(200),
            Some(30001),
        ));
    }

    #[test]
    fn known_post_status_is_suppressed() {
        assert!(is_known_business_error(
            "fanqie_promotion",
            "promotion/post/create",
            Some(200),
            Some(10004),
        ));
    }

    #[test]
    fn unknown_code_is_logged() {
        assert!(!is_known_business_error(
            "fanqie_promotion",
            "promotion/plan/create",
            Some(200),
            Some(99999),
        ));
    }

    #[test]
    fn non_200_is_logged_even_if_code_matches() {
        // 401 with a familiar-looking body is still an auth signal we
        // need to capture.
        assert!(!is_known_business_error(
            "fanqie_promotion",
            "promotion/plan/create",
            Some(401),
            Some(30001),
        ));
    }

    #[test]
    fn missing_code_is_logged() {
        // Body didn't parse / had no `code` field — definitely keep.
        assert!(!is_known_business_error(
            "fanqie_promotion",
            "promotion/plan/create",
            Some(200),
            None,
        ));
    }

    #[test]
    fn unknown_endpoint_is_logged() {
        assert!(!is_known_business_error(
            "fanqie_promotion",
            "promotion/something_new",
            Some(200),
            Some(30001),
        ));
    }
}
