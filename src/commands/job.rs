//! Job command implementation.

use crate::client::ApiClient;
use crate::error::CliError;
use crate::output::{self, OutputFormat};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Subcommand, Clone)]
pub enum JobCommand {
    /// Show job status
    Show {
        /// Job ID
        id: String,
    },

    /// Wait for job to complete
    Wait {
        /// Job ID
        id: String,

        /// Timeout duration
        #[arg(long, default_value = "5m")]
        timeout: String,

        /// Poll interval
        #[arg(long, default_value = "5s")]
        poll_interval: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobResponse {
    pub id: Uuid,
    pub job_type: String,
    pub status: String,
    pub created_at: u64,
    #[serde(default)]
    pub started_at: Option<u64>,
    #[serde(default)]
    pub completed_at: Option<u64>,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

impl JobCommand {
    pub async fn run(
        self,
        client: &ApiClient,
        format: OutputFormat,
        quiet: bool,
    ) -> Result<(), CliError> {
        match self {
            JobCommand::Show { id } => {
                let uuid = id.parse::<Uuid>()
                    .map_err(|_| CliError::Other(format!("Invalid job ID: {}", id)))?;
                let path = format!("/api/v1/jobs/{}", uuid);
                let response: JobResponse = client.get(&path).await?;
                output::print_item(&response, format)?;
                Ok(())
            }

            JobCommand::Wait { id, timeout, poll_interval } => {
                let uuid = id.parse::<Uuid>()
                    .map_err(|_| CliError::Other(format!("Invalid job ID: {}", id)))?;
                let timeout_duration = parse_duration(&timeout)?;
                let poll_duration = parse_duration(&poll_interval)?;

                let start = std::time::Instant::now();

                loop {
                    let path = format!("/api/v1/jobs/{}", uuid);
                    let response: JobResponse = client.get(&path).await?;

                    match response.status.as_str() {
                        "completed" => {
                            output::print_success(&format!("Job {} completed", id), quiet);
                            return Ok(());
                        }
                        "failed" => {
                            let msg = response.error.unwrap_or_else(|| "Unknown error".to_string());
                            return Err(CliError::Api(format!("Job failed: {}", msg)));
                        }
                        status => {
                            if start.elapsed() > timeout_duration {
                                return Err(CliError::Timeout(format!(
                                    "job {} to complete (current status: {})",
                                    id, status
                                )));
                            }
                            if !quiet {
                                eprintln!("Job {} status: {}...", id, status);
                            }
                            tokio::time::sleep(poll_duration).await;
                        }
                    }
                }
            }
        }
    }
}

fn parse_duration(s: &str) -> Result<std::time::Duration, CliError> {
    let s = s.trim();

    if let Some(num) = s.strip_suffix("ms") {
        let value: u64 = num.parse()
            .map_err(|_| CliError::Other(format!("Invalid duration number: {}", num)))?;
        return Ok(std::time::Duration::from_millis(value));
    }
    if let Some(num) = s.strip_suffix('s') {
        let value: u64 = num.parse()
            .map_err(|_| CliError::Other(format!("Invalid duration number: {}", num)))?;
        return Ok(std::time::Duration::from_secs(value));
    }
    if let Some(num) = s.strip_suffix('m') {
        let value: u64 = num.parse()
            .map_err(|_| CliError::Other(format!("Invalid duration number: {}", num)))?;
        return Ok(std::time::Duration::from_secs(value * 60));
    }
    if let Some(num) = s.strip_suffix('h') {
        let value: u64 = num.parse()
            .map_err(|_| CliError::Other(format!("Invalid duration number: {}", num)))?;
        return Ok(std::time::Duration::from_secs(value * 3600));
    }

    Err(CliError::Other(format!("Invalid duration: {}. Use format like '5s', '10m', '1h'", s)))
}
