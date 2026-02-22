//! HTTP client for scry-platform API.

use crate::error::CliError;
use reqwest::{Client, Response, StatusCode};
use serde::{de::DeserializeOwned, Serialize};
use std::future::Future;

/// API client for scry-platform.
#[derive(Clone)]
pub struct ApiClient {
    client: Client,
    base_url: String,
    token: String,
}

impl ApiClient {
    /// Create a new API client.
    pub fn new(base_url: String, token: String) -> Result<Self, CliError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        // Ensure base URL doesn't end with slash
        let base_url = base_url.trim_end_matches('/').to_string();

        Ok(Self {
            client,
            base_url,
            token,
        })
    }

    /// Get the base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Get the authentication token.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Check if an error is retryable.
    fn is_retryable_error(status: StatusCode) -> bool {
        matches!(
            status,
            StatusCode::SERVICE_UNAVAILABLE
                | StatusCode::GATEWAY_TIMEOUT
                | StatusCode::TOO_MANY_REQUESTS
        )
    }

    /// Execute a request with retry logic.
    async fn execute_with_retry<F, Fut, T>(&self, mut request_fn: F) -> Result<T, CliError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<Response, reqwest::Error>>,
        T: DeserializeOwned,
    {
        let max_retries = 3;
        let mut last_error = None;

        for attempt in 0..=max_retries {
            if attempt > 0 {
                let delay = std::time::Duration::from_millis(500 * 2u64.pow(attempt as u32 - 1));
                tokio::time::sleep(delay).await;
            }

            match request_fn().await {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        let body = response.json().await?;
                        return Ok(body);
                    } else if Self::is_retryable_error(status) && attempt < max_retries {
                        last_error = Some(CliError::Api(format!("{}: retrying...", status)));
                        continue;
                    } else {
                        return self.handle_error_response(response).await;
                    }
                }
                Err(e) if e.is_connect() || e.is_timeout() => {
                    if attempt < max_retries {
                        last_error = Some(CliError::Api(format!("Connection error: {}", e)));
                        continue;
                    }
                    return Err(CliError::Request(e));
                }
                Err(e) => return Err(CliError::Request(e)),
            }
        }

        Err(last_error.unwrap_or_else(|| CliError::Other("Request failed after retries".to_string())))
    }

    /// Handle error response, extracting error text.
    async fn handle_error_response<T>(&self, response: Response) -> Result<T, CliError> {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());

        match status {
            StatusCode::NOT_FOUND => Err(CliError::NotFound(error_text)),
            StatusCode::UNAUTHORIZED => Err(CliError::Api("Unauthorized - check your token".to_string())),
            StatusCode::FORBIDDEN => Err(CliError::Api("Forbidden - insufficient permissions".to_string())),
            _ => Err(CliError::Api(format!("{}: {}", status, error_text))),
        }
    }

    /// Make a GET request with retry logic.
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, CliError> {
        let url = format!("{}{}", self.base_url, path);
        let token = self.token.clone();
        let client = self.client.clone();

        self.execute_with_retry(|| {
            let url = url.clone();
            let token = token.clone();
            let client = client.clone();
            async move {
                client
                    .get(&url)
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await
            }
        })
        .await
    }

    /// Make a GET request without authentication (for health checks).
    pub async fn get_no_auth<T: DeserializeOwned>(&self, path: &str) -> Result<T, CliError> {
        let url = format!("{}{}", self.base_url, path);
        let client = self.client.clone();

        self.execute_with_retry(|| {
            let url = url.clone();
            let client = client.clone();
            async move { client.get(&url).send().await }
        })
        .await
    }

    /// Make a POST request with retry logic.
    pub async fn post<T: DeserializeOwned, B: Serialize + Clone>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, CliError> {
        let url = format!("{}{}", self.base_url, path);
        let token = self.token.clone();
        let client = self.client.clone();
        let body = body.clone();

        self.execute_with_retry(|| {
            let url = url.clone();
            let token = token.clone();
            let client = client.clone();
            let body = body.clone();
            async move {
                client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", token))
                    .json(&body)
                    .send()
                    .await
            }
        })
        .await
    }

    /// Make a POST request without a body with retry logic.
    #[allow(dead_code)]
    pub async fn post_empty<T: DeserializeOwned>(&self, path: &str) -> Result<T, CliError> {
        let url = format!("{}{}", self.base_url, path);
        let token = self.token.clone();
        let client = self.client.clone();

        self.execute_with_retry(|| {
            let url = url.clone();
            let token = token.clone();
            let client = client.clone();
            async move {
                client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await
            }
        })
        .await
    }

    /// Make a DELETE request with retry logic.
    pub async fn delete<T: DeserializeOwned>(&self, path: &str) -> Result<T, CliError> {
        let url = format!("{}{}", self.base_url, path);
        let token = self.token.clone();
        let client = self.client.clone();

        self.execute_with_retry(|| {
            let url = url.clone();
            let token = token.clone();
            let client = client.clone();
            async move {
                client
                    .delete(&url)
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_strips_trailing_slash() {
        let client = ApiClient::new(
            "https://example.com/".to_string(),
            "token".to_string(),
        ).unwrap();
        assert_eq!(client.base_url, "https://example.com");
    }
}
