//! HTTP client for demo API calls with retries and verbose logging.

use super::error::DemoError;
use reqwest::{Client, StatusCode};
use serde_json::Value;
use std::time::Duration;

/// Demo API client with retry logic and optional verbose output.
pub struct DemoClient {
    client: Client,
    base_url: String,
    token: String,
    verbose: bool,
}

impl DemoClient {
    /// Create a new demo client.
    pub fn new(base_url: &str, token: &str, verbose: bool) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            verbose,
        }
    }

    /// Make an authenticated GET request with retries.
    pub async fn get(&self, path: &str) -> Result<Value, DemoError> {
        self.request_with_retry("GET", path, None).await
    }

    /// Make an authenticated POST request with retries.
    pub async fn post(&self, path: &str, body: &Value) -> Result<Value, DemoError> {
        self.request_with_retry("POST", path, Some(body)).await
    }

    /// Make an authenticated DELETE request with retries.
    pub async fn delete(&self, path: &str) -> Result<Value, DemoError> {
        self.request_with_retry("DELETE", path, None).await
    }

    /// Poll GET /api/v1/ready until 200 or timeout.
    pub async fn wait_for_ready(&self, timeout_secs: u64) -> Result<(), DemoError> {
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(timeout_secs);

        loop {
            if start.elapsed() >= timeout {
                return Err(DemoError::Timeout {
                    what: "API ready endpoint".to_string(),
                    elapsed_secs: timeout_secs,
                });
            }

            let url = format!("{}/api/v1/ready", self.base_url);
            let resp = self
                .client
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.token))
                .send()
                .await;

            match resp {
                Ok(r) if r.status().is_success() => return Ok(()),
                Ok(r) if self.verbose => {
                    eprintln!("[demo-client] GET /api/v1/ready -> {}", r.status());
                }
                _ => {}
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Execute an HTTP request with up to 3 retries and exponential backoff.
    async fn request_with_retry(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, DemoError> {
        let url = format!("{}{}", self.base_url, path);
        let max_retries: u32 = 3;
        let mut last_err = None;

        for attempt in 0..=max_retries {
            if attempt > 0 {
                let delay = Duration::from_millis(500 * 2u64.pow(attempt - 1));
                tokio::time::sleep(delay).await;
            }

            let mut req = match method {
                "POST" => self.client.post(&url),
                "DELETE" => self.client.delete(&url),
                _ => self.client.get(&url),
            };
            req = req.header("Authorization", format!("Bearer {}", self.token));
            if let Some(b) = body {
                req = req.json(b);
            }

            if self.verbose {
                eprintln!("[demo-client] {method} {path} (attempt {})", attempt + 1);
            }

            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) if (e.is_connect() || e.is_timeout()) && attempt < max_retries => {
                    last_err = Some(format!("{e}"));
                    continue;
                }
                Err(e) => {
                    return Err(DemoError::ApiRequest {
                        method: method.to_string(),
                        url,
                        reason: format!("{e}"),
                    });
                }
            };

            let status = resp.status();

            if self.verbose {
                eprintln!("[demo-client] {method} {path} -> {status}");
            }

            if status.is_success() {
                let value: Value = resp.json().await.unwrap_or(Value::Null);
                return Ok(value);
            }

            // Retry on transient errors
            if is_retryable(status) && attempt < max_retries {
                last_err = Some(format!("HTTP {status}"));
                continue;
            }

            let body_text = resp.text().await.unwrap_or_default();

            // Retry on 500 with transient backend errors (e.g. SQLite lock contention)
            if status == StatusCode::INTERNAL_SERVER_ERROR
                && attempt < max_retries
                && body_text.contains("database is locked")
            {
                last_err = Some(format!("HTTP {status}: {body_text}"));
                continue;
            }

            return Err(DemoError::ApiResponse {
                method: method.to_string(),
                url,
                status: status.as_u16(),
                body: body_text,
            });
        }

        Err(DemoError::ApiRequest {
            method: method.to_string(),
            url,
            reason: last_err.unwrap_or_else(|| "request failed after retries".to_string()),
        })
    }
}

fn is_retryable(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::SERVICE_UNAVAILABLE | StatusCode::GATEWAY_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
    )
}
