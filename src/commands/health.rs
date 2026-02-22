//! Health command implementation.

use crate::client::ApiClient;
use crate::error::CliError;
use crate::output::{self, OutputFormat};
use clap::Args;
use serde::{Deserialize, Serialize};

#[derive(Args, Clone)]
pub struct HealthCommand {
    /// Wait for service to become healthy
    #[arg(long)]
    pub wait: bool,

    /// Timeout when waiting (e.g., "60s", "2m")
    #[arg(long, default_value = "30s")]
    pub timeout: String,

    /// Poll interval when waiting (e.g., "2s", "5s")
    #[arg(long, default_value = "2s")]
    pub poll_interval: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub healthy: bool,
    pub api: bool,
    pub version: Option<String>,
}

impl HealthCommand {
    pub async fn run(
        self,
        client: &ApiClient,
        format: OutputFormat,
        quiet: bool,
    ) -> Result<(), CliError> {
        if self.wait {
            self.wait_for_healthy(client, quiet).await?;
        } else {
            self.check_once(client, format, quiet).await?;
        }
        Ok(())
    }

    async fn check_once(
        &self,
        client: &ApiClient,
        format: OutputFormat,
        quiet: bool,
    ) -> Result<(), CliError> {
        match client.get_no_auth::<HealthResponse>("/health").await {
            Ok(response) => {
                let status = HealthStatus {
                    healthy: response.status == "ok",
                    api: true,
                    version: Some(response.version),
                };
                if quiet {
                    if status.healthy {
                        println!("healthy");
                    } else {
                        println!("unhealthy");
                    }
                } else {
                    output::print_item(&status, format)?;
                }
                if status.healthy {
                    Ok(())
                } else {
                    Err(CliError::Api("Service unhealthy".to_string()))
                }
            }
            Err(e) => {
                if quiet {
                    println!("unhealthy");
                }
                Err(e)
            }
        }
    }

    async fn wait_for_healthy(&self, client: &ApiClient, quiet: bool) -> Result<(), CliError> {
        let timeout_duration = parse_duration(&self.timeout)?;
        let poll_duration = parse_duration(&self.poll_interval)?;
        let start = std::time::Instant::now();

        loop {
            match client.get_no_auth::<HealthResponse>("/health").await {
                Ok(response) if response.status == "ok" => {
                    output::print_success(
                        &format!("Service healthy (v{})", response.version),
                        quiet,
                    );
                    return Ok(());
                }
                Ok(_) => {
                    if !quiet {
                        eprintln!("Service not ready...");
                    }
                }
                Err(_) => {
                    if !quiet {
                        eprintln!("Waiting for service...");
                    }
                }
            }

            if start.elapsed() > timeout_duration {
                return Err(CliError::Timeout("service to become healthy".to_string()));
            }

            tokio::time::sleep(poll_duration).await;
        }
    }
}

fn parse_duration(s: &str) -> Result<std::time::Duration, CliError> {
    let s = s.trim();

    if let Some(num) = s.strip_suffix("ms") {
        let value: u64 = num
            .parse()
            .map_err(|_| CliError::Other(format!("Invalid duration: {}", s)))?;
        return Ok(std::time::Duration::from_millis(value));
    }
    if let Some(num) = s.strip_suffix('s') {
        let value: u64 = num
            .parse()
            .map_err(|_| CliError::Other(format!("Invalid duration: {}", s)))?;
        return Ok(std::time::Duration::from_secs(value));
    }
    if let Some(num) = s.strip_suffix('m') {
        let value: u64 = num
            .parse()
            .map_err(|_| CliError::Other(format!("Invalid duration: {}", s)))?;
        return Ok(std::time::Duration::from_secs(value * 60));
    }

    Err(CliError::Other(format!(
        "Invalid duration: {}. Use format like '5s', '10m'",
        s
    )))
}
