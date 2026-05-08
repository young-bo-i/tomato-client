//! Client for the bundled abogus signing service (`abogusapp` container,
//! reachable at `http://abogus:3000/api/get-a-bogus` on the docker
//! internal network). Used to sign requests to TomatoNovel/Douyin/etc.
//!
//! Mirrors the legacy C# `ABogusHelper` flow:
//!  1. Build URL, append `&msToken=<160-char random>`
//!  2. POST `{params, body, ua}` to abogus → `{result}` (the a_bogus value)
//!  3. Append `&a_bogus=<value>` and request the upstream URL
//!
//! UA must match what the abogus algorithm was tuned for — tomato's
//! signature checker rejects mismatches.

use rand::seq::SliceRandom;
use reqwest_middleware::ClientWithMiddleware as Client;
use serde::{Deserialize, Serialize};

/// User-Agent must be byte-identical between the abogus signing call
/// and the actual outbound HTTP request — abogus mixes the UA into the
/// signature and the upstream rejects mismatches. Captured from a real
/// browser session against kol.fanqieopen.com.
pub const TOMATO_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
                             AppleWebKit/537.36 (KHTML, like Gecko) \
                             Chrome/147.0.0.0 Safari/537.36";

/// Chromium "Client Hints" trio that pairs with the UA above. Servers
/// sometimes cross-check these against UA, so keep them in sync.
pub const TOMATO_SEC_CH_UA: &str =
    r#""Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#;
pub const TOMATO_SEC_CH_UA_PLATFORM: &str = r#""macOS""#;
pub const TOMATO_SEC_CH_UA_MOBILE: &str = "?0";

/// Same charset the legacy C# uses (note the original repeated G/I —
/// kept verbatim so anything that happened to be sensitive to the exact
/// distribution still works).
const MS_TOKEN_CHARSET: &[u8] =
    b"ABCDEFGHIGKLMNOPQRSTUVWXYZabcdefghigklmnopqrstuvwxyz0123456789_-";

pub fn random_ms_token(len: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| *MS_TOKEN_CHARSET.choose(&mut rng).unwrap() as char)
        .collect()
}

#[derive(Debug, Serialize)]
struct AbogusRequest<'a> {
    params: &'a str,
    body: &'a str,
    ua: &'a str,
}

#[derive(Debug, Deserialize)]
struct AbogusResponse {
    result: String,
}

/// POST the (params, body, ua) tuple to abogus and return the signed
/// `a_bogus` value. `params` is the query string AFTER any `?`,
/// including the freshly-appended `msToken`. `body` is empty for GETs.
pub async fn sign(
    http: &Client,
    abogus_url: &str,
    params: &str,
    body: &str,
    ua: &str,
) -> Result<String, String> {
    let req = AbogusRequest { params, body, ua };
    let res = http
        .post(abogus_url)
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("abogus request: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("abogus status {}", res.status()));
    }
    let parsed: AbogusResponse = res
        .json()
        .await
        .map_err(|e| format!("abogus parse: {e}"))?;
    Ok(parsed.result)
}

/// Convenience: take a URL with already-built query params, append both
/// `msToken` and `a_bogus`, and return the final ready-to-fetch URL.
pub async fn sign_url(
    http: &Client,
    abogus_url: &str,
    base_url: &str,
    body: &str,
    ua: &str,
) -> Result<String, String> {
    let token = random_ms_token(160);
    let url = format!("{base_url}&msToken={token}");
    let params = url.split_once('?').map(|(_, q)| q).unwrap_or(&url);
    let a_bogus = sign(http, abogus_url, params, body, ua).await?;
    Ok(format!("{url}&a_bogus={a_bogus}"))
}
