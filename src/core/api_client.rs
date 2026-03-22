use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::de::DeserializeOwned;
use std::time::Duration;

use super::auth;

pub struct ApiClient {
    client: Client,
    base_url: String,
    api_key: String,
}

impl ApiClient {
    pub fn from_credentials() -> Result<Self, String> {
        let creds = auth::load().ok_or("Not logged in. Run `blameprompt login` first.")?;
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {}", e))?;
        Ok(Self {
            client,
            base_url: creds.api_url,
            api_key: creds.api_key,
        })
    }

    fn headers(&self) -> Result<HeaderMap, String> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-API-Key",
            HeaderValue::from_str(&self.api_key)
                .map_err(|_| "Invalid API key — contains non-ASCII or control characters. Re-run `blameprompt login`.".to_string())?,
        );
        Ok(headers)
    }

    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let url = format!("{}{}", self.base_url, path);
        let res = self
            .client
            .get(&url)
            .headers(self.headers()?)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if res.status().as_u16() == 401 {
            return Err("Session expired. Please run `blameprompt login` again.".to_string());
        }

        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().unwrap_or_default();
            let detail = body
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(200)
                .collect::<String>();
            return Err(if detail.is_empty() {
                format!("API error: {}", status)
            } else {
                format!("API error ({}): {}", status, detail)
            });
        }

        res.json::<T>().map_err(|e| format!("Parse error: {}", e))
    }

    pub fn post<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, String> {
        let url = format!("{}{}", self.base_url, path);
        let res = self
            .client
            .post(&url)
            .headers(self.headers()?)
            .json(body)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if res.status().as_u16() == 401 {
            return Err("Session expired. Please run `blameprompt login` again.".to_string());
        }

        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().unwrap_or_default();
            let detail = body
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(200)
                .collect::<String>();
            return Err(if detail.is_empty() {
                format!("API error: {}", status)
            } else {
                format!("API error ({}): {}", status, detail)
            });
        }

        res.json::<T>().map_err(|e| format!("Parse error: {}", e))
    }
}
