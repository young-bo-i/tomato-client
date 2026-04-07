use reqwest::header::{HeaderMap, HeaderValue, COOKIE, USER_AGENT};
use serde::{Deserialize, Serialize};

const BASE_URL: &str = "https://kol.fanqieopen.com/api/platform";
const PROMOTER_URL: &str = "https://promoter.fanqieopen.com/api/platform";
const COMMON_PARAMS: &str = "app_id=457699&aid=457699&origin_app_id=457699&host_app_id=457699";

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub struct TomatoClient {
    client: reqwest::Client,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SendWordResult {
    pub is_succeed: bool,
    pub is_limit: bool,
    pub is_alias_error: bool,
    pub is_change_book: bool,
    pub frequency_limiting: bool,
    pub alias_id: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    code: i32,
    msg: Option<String>,
    data: Option<serde_json::Value>,
}

impl TomatoClient {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");
        Self { client }
    }

    fn build_headers(cookies: &str, x_kol_token: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(UA));
        if let Ok(v) = HeaderValue::from_str(cookies) {
            headers.insert(COOKIE, v);
        }
        if let Some(token) = x_kol_token {
            if let Ok(v) = HeaderValue::from_str(token) {
                headers.insert("x-kol-token", v);
            }
        }
        headers
    }

    /// Submit a word/alias to Tomato platform
    pub async fn send_word(
        &self,
        cookies: &str,
        x_kol_token: Option<&str>,
        alias_name: &str,
        book_id: &str,
        platform: i16,
        version: &str,
    ) -> anyhow::Result<SendWordResult> {
        let url = format!(
            "{}/promotion/plan/create/v:{}?{}",
            BASE_URL, version, COMMON_PARAMS
        );

        let body = build_send_word_body(alias_name, book_id, platform);
        let headers = Self::build_headers(cookies, x_kol_token);

        let resp = self.client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await?;

        let api_resp: ApiResponse = resp.json().await?;
        Ok(parse_send_word_response(api_resp))
    }

    /// Write back sharing URL to a submitted task
    pub async fn write_back(
        &self,
        cookies: &str,
        x_kol_token: Option<&str>,
        alias_id: &str,
        share_url: &str,
    ) -> anyhow::Result<bool> {
        let url = format!(
            "{}/promotion/post/create?{}",
            BASE_URL, COMMON_PARAMS
        );

        let body = serde_json::json!({
            "alias_id": alias_id,
            "share_url": share_url,
        });
        let headers = Self::build_headers(cookies, x_kol_token);

        let resp = self.client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await?;

        let api_resp: ApiResponse = resp.json().await?;
        Ok(api_resp.code == 0)
    }

    /// Get books from platform
    pub async fn get_books(
        &self,
        cookies: &str,
        content_tab: &str,
        page: i32,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let url = format!(
            "{}/content/book/list?{}&content_tab={}&page={}&page_size=10",
            BASE_URL, COMMON_PARAMS, content_tab, page
        );

        let headers = Self::build_headers(cookies, None);
        let resp = self.client.get(&url).headers(headers).send().await?;
        let api_resp: ApiResponse = resp.json().await?;

        if api_resp.code == 0 {
            if let Some(data) = api_resp.data {
                if let Some(books) = data.get("book_list").and_then(|b| b.as_array()) {
                    return Ok(books.clone());
                }
            }
        }
        Ok(vec![])
    }

    /// Get income data
    pub async fn get_income(&self, cookies: &str) -> anyhow::Result<serde_json::Value> {
        let url = format!("{}/user/income/stats?{}", BASE_URL, COMMON_PARAMS);
        let headers = Self::build_headers(cookies, None);
        let resp = self.client.get(&url).headers(headers).send().await?;
        let api_resp: ApiResponse = resp.json().await?;
        Ok(api_resp.data.unwrap_or(serde_json::json!({})))
    }

    /// Get alias/promotion plan status
    pub async fn get_alias_status(
        &self,
        cookies: &str,
        alias_id: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let url = format!(
            "{}/promotion/plan/list?{}&alias_id={}",
            BASE_URL, COMMON_PARAMS, alias_id
        );
        let headers = Self::build_headers(cookies, None);
        let resp = self.client.get(&url).headers(headers).send().await?;
        let api_resp: ApiResponse = resp.json().await?;
        Ok(api_resp.data.unwrap_or(serde_json::json!({})))
    }

    /// Create invite code
    pub async fn create_invite_code(&self, cookies: &str) -> anyhow::Result<serde_json::Value> {
        let url = format!("{}/user/invite_code/create?{}", PROMOTER_URL, COMMON_PARAMS);
        let headers = Self::build_headers(cookies, None);
        let resp = self.client.post(&url).headers(headers).send().await?;
        let api_resp: ApiResponse = resp.json().await?;
        Ok(api_resp.data.unwrap_or(serde_json::json!({})))
    }

    /// Login with invite code to get x_kol_token
    pub async fn invite_code_login(
        &self,
        invite_code: &str,
        share_token: &str,
    ) -> anyhow::Result<Option<String>> {
        let url = format!(
            "{}/user/invite_code/log_in?{}&invite_code={}&share_token={}",
            BASE_URL, COMMON_PARAMS, invite_code, share_token
        );

        let resp = self.client.post(&url).send().await?;
        let cookies = resp.headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .filter_map(|v| {
                if v.contains("x_kol_token=") {
                    v.split(';').next().map(|s| s.trim().to_string())
                } else {
                    None
                }
            })
            .next();

        Ok(cookies)
    }
}

fn build_send_word_body(alias_name: &str, book_id: &str, platform: i16) -> serde_json::Value {
    let content_type = match platform {
        1 => "novel",
        2 => "toutiao",
        3 => "changting",
        4 => "wukong",
        _ => "novel",
    };

    serde_json::json!({
        "alias_name": alias_name,
        "book_id": book_id,
        "content_type": content_type,
    })
}

fn parse_send_word_response(resp: ApiResponse) -> SendWordResult {
    match resp.code {
        0 => {
            let alias_id = resp.data
                .as_ref()
                .and_then(|d| d.get("alias_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            SendWordResult {
                is_succeed: true,
                is_limit: false,
                is_alias_error: false,
                is_change_book: false,
                frequency_limiting: false,
                alias_id,
                message: None,
            }
        }
        10004 => SendWordResult {
            is_succeed: false,
            is_limit: false,
            is_alias_error: false,
            is_change_book: false,
            frequency_limiting: false,
            alias_id: None,
            message: Some("Not logged in".into()),
        },
        10011 => SendWordResult {
            is_succeed: false,
            is_limit: false,
            is_alias_error: false,
            is_change_book: false,
            frequency_limiting: true,
            alias_id: None,
            message: resp.msg,
        },
        30001 => {
            let msg = resp.msg.clone().unwrap_or_default();
            SendWordResult {
                is_succeed: false,
                is_limit: msg.contains("每日上限") || msg.contains("limit"),
                is_alias_error: msg.contains("已被申请") || msg.contains("敏感词"),
                is_change_book: msg.contains("更换书籍") || msg.contains("book"),
                frequency_limiting: false,
                alias_id: None,
                message: resp.msg,
            }
        }
        _ => SendWordResult {
            is_succeed: false,
            is_limit: false,
            is_alias_error: false,
            is_change_book: false,
            frequency_limiting: false,
            alias_id: None,
            message: resp.msg,
        },
    }
}
