use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde::{Deserialize, Serialize};

const BASE_URL: &str = "https://kol.wtzw.com/api/v1";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub struct QiMaoClient {
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct QiMaoResponse {
    code: i32,
    msg: Option<String>,
    data: Option<serde_json::Value>,
}

impl QiMaoClient {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");
        Self { client }
    }

    fn build_headers(token: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(UA));
        if let Some(t) = token {
            if let Ok(v) = HeaderValue::from_str(t) {
                headers.insert("x-qm-devops-token", v);
            }
        }
        headers
    }

    /// Sign in to get token
    pub async fn signin(&self, phone: &str, password: &str) -> anyhow::Result<Option<String>> {
        let url = format!("{}/user/signin", BASE_URL);
        let body = serde_json::json!({
            "phone": phone,
            "password": password,
        });

        let resp = self.client
            .post(&url)
            .headers(Self::build_headers(None))
            .json(&body)
            .send()
            .await?;

        let api_resp: QiMaoResponse = resp.json().await?;
        if api_resp.code == 0 {
            let token = api_resp.data
                .as_ref()
                .and_then(|d| d.get("token"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            return Ok(token);
        }
        Ok(None)
    }

    /// Pre-check keyword before submission
    pub async fn keyword_precheck(
        &self,
        token: &str,
        keyword: &str,
    ) -> anyhow::Result<bool> {
        let url = format!("{}/promotion/keyword_precheck", BASE_URL);
        let body = serde_json::json!({ "keyword": keyword });
        let headers = Self::build_headers(Some(token));

        let resp = self.client.post(&url).headers(headers).json(&body).send().await?;
        let api_resp: QiMaoResponse = resp.json().await?;
        Ok(api_resp.code == 0)
    }

    /// Submit keyword/alias
    pub async fn add_words(
        &self,
        token: &str,
        keyword: &str,
        book_id: &str,
    ) -> anyhow::Result<bool> {
        let url = format!("{}/promotion/add_keywords", BASE_URL);
        let body = serde_json::json!({
            "keyword": keyword,
            "book_id": book_id,
        });
        let headers = Self::build_headers(Some(token));

        let resp = self.client.post(&url).headers(headers).json(&body).send().await?;
        let api_resp: QiMaoResponse = resp.json().await?;
        Ok(api_resp.code == 0)
    }

    /// Write back link to keyword
    pub async fn add_keyword_links(
        &self,
        token: &str,
        keyword_id: &str,
        share_url: &str,
    ) -> anyhow::Result<bool> {
        let url = format!("{}/promotion/add_keyword_links", BASE_URL);
        let body = serde_json::json!({
            "keyword_id": keyword_id,
            "link": share_url,
        });
        let headers = Self::build_headers(Some(token));

        let resp = self.client.post(&url).headers(headers).json(&body).send().await?;
        let api_resp: QiMaoResponse = resp.json().await?;
        Ok(api_resp.code == 0)
    }

    /// Get keyword status list
    pub async fn get_keywords(
        &self,
        token: &str,
        page: i32,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let url = format!("{}/promotion/keyword_page?page={}&page_size=20", BASE_URL, page);
        let headers = Self::build_headers(Some(token));

        let resp = self.client.get(&url).headers(headers).send().await?;
        let api_resp: QiMaoResponse = resp.json().await?;

        if api_resp.code == 0 {
            if let Some(data) = api_resp.data {
                if let Some(list) = data.get("list").and_then(|l| l.as_array()) {
                    return Ok(list.clone());
                }
            }
        }
        Ok(vec![])
    }

    /// Get books
    pub async fn get_books(&self, token: &str, page: i32) -> anyhow::Result<Vec<serde_json::Value>> {
        let url = format!("{}/data/book/index?page={}&page_size=20", BASE_URL, page);
        let headers = Self::build_headers(Some(token));

        let resp = self.client.get(&url).headers(headers).send().await?;
        let api_resp: QiMaoResponse = resp.json().await?;

        if api_resp.code == 0 {
            if let Some(data) = api_resp.data {
                if let Some(list) = data.get("list").and_then(|l| l.as_array()) {
                    return Ok(list.clone());
                }
            }
        }
        Ok(vec![])
    }
}
