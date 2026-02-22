//! Replay command implementation.

use crate::client::ApiClient;
use crate::error::CliError;
use crate::output::{self, OutputFormat};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use tabled::Tabled;
use uuid::Uuid;

#[derive(Subcommand, Clone)]
pub enum ReplayCommand {
    /// Start a replay
    Start {
        /// Shadow ID
        #[arg(long)]
        shadow: String,

        /// Query time window (e.g., "1h", "30m")
        #[arg(long, default_value = "1h")]
        query_window: String,
    },

    /// List replays
    List {
        /// Filter by shadow ID
        #[arg(long)]
        shadow: Option<String>,
    },

    /// Show replay details
    Show {
        /// Replay ID
        id: String,
    },

    /// Wait for replay to complete
    Wait {
        /// Replay ID
        id: String,

        /// Timeout duration
        #[arg(long, default_value = "10m")]
        timeout: String,

        /// Poll interval
        #[arg(long, default_value = "5s")]
        poll_interval: String,
    },

    /// Stop a replay
    Stop {
        /// Replay ID
        id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartReplayRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayResponse {
    pub id: Uuid,
    pub shadow_id: Uuid,
    pub status: String,
    pub queries_total: u64,
    pub queries_completed: u64,
    pub queries_failed: u64,
    pub created_at: u64,
    #[serde(default)]
    pub completed_at: Option<u64>,
    #[serde(default)]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct ReplayListItem {
    #[tabled(rename = "ID")]
    pub id: String,
    #[tabled(rename = "Shadow")]
    pub shadow_id: String,
    #[tabled(rename = "Status")]
    pub status: String,
    #[tabled(rename = "Progress")]
    pub progress: String,
}

impl From<ReplayResponse> for ReplayListItem {
    fn from(r: ReplayResponse) -> Self {
        let progress = if r.queries_total > 0 {
            format!(
                "{}/{} ({:.1}%)",
                r.queries_completed,
                r.queries_total,
                (r.queries_completed as f64 / r.queries_total as f64) * 100.0
            )
        } else {
            "0/0".to_string()
        };

        Self {
            id: r.id.to_string()[..8].to_string(),
            shadow_id: r.shadow_id.to_string()[..8].to_string(),
            status: r.status,
            progress,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageResponse<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResponse {
    pub success: bool,
    pub message: Option<String>,
}

impl ReplayCommand {
    pub async fn run(
        self,
        client: &ApiClient,
        format: OutputFormat,
        quiet: bool,
    ) -> Result<(), CliError> {
        match self {
            ReplayCommand::Start {
                shadow,
                query_window,
            } => {
                let shadow_id = shadow.parse::<Uuid>()
                    .map_err(|_| CliError::Other(format!("Invalid shadow ID: {}", shadow)))?;

                let window_secs = parse_window(&query_window)?;

                // Convert window to absolute timestamps
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| CliError::Other(format!("Failed to get current time: {}", e)))?
                    .as_millis() as u64;
                let window_ms = window_secs * 1000;

                let request = StartReplayRequest {
                    start_time_ms: Some(now_ms.saturating_sub(window_ms)),
                    end_time_ms: Some(now_ms),
                };

                let path = format!("/api/v1/shadows/{}/replays", shadow_id);
                let response: ReplayResponse = client.post(&path, &request).await?;
                if quiet {
                    // Output only the ID for scripting
                    println!("{}", response.id);
                } else {
                    output::print_item(&response, format)?;
                }
                Ok(())
            }

            ReplayCommand::List { shadow } => {
                let path = if let Some(s) = shadow {
                    let uuid = s.parse::<Uuid>()
                        .map_err(|_| CliError::Other(format!("Invalid shadow ID: {}", s)))?;
                    format!("/api/v1/shadows/{}/replays", uuid)
                } else {
                    "/api/v1/replays".to_string()
                };

                let response: PageResponse<ReplayResponse> = client.get(&path).await?;
                let items: Vec<ReplayListItem> = response.items.into_iter().map(Into::into).collect();
                output::print_table(&items, format)?;
                Ok(())
            }

            ReplayCommand::Show { id } => {
                let uuid = id.parse::<Uuid>()
                    .map_err(|_| CliError::Other(format!("Invalid replay ID: {}", id)))?;
                let path = format!("/api/v1/replays/{}", uuid);
                let response: ReplayResponse = client.get(&path).await?;
                output::print_item(&response, format)?;
                Ok(())
            }

            ReplayCommand::Wait {
                id,
                timeout,
                poll_interval,
            } => {
                let uuid = id.parse::<Uuid>()
                    .map_err(|_| CliError::Other(format!("Invalid replay ID: {}", id)))?;
                let timeout_duration = parse_duration(&timeout)?;
                let poll_duration = parse_duration(&poll_interval)?;

                let start = std::time::Instant::now();

                loop {
                    let path = format!("/api/v1/replays/{}", uuid);
                    let response: ReplayResponse = client.get(&path).await?;

                    match response.status.as_str() {
                        "completed" => {
                            output::print_success(&format!("Replay {} completed", id), quiet);
                            return Ok(());
                        }
                        "failed" => {
                            let msg = response.error_message.unwrap_or_else(|| "Unknown error".to_string());
                            return Err(CliError::Api(format!("Replay failed: {}", msg)));
                        }
                        status => {
                            if start.elapsed() > timeout_duration {
                                return Err(CliError::Timeout(format!(
                                    "replay {} to complete (current status: {})",
                                    id, status
                                )));
                            }
                            if !quiet {
                                let progress = if response.queries_total > 0 {
                                    format!(
                                        "{}/{} ({:.1}%)",
                                        response.queries_completed,
                                        response.queries_total,
                                        (response.queries_completed as f64 / response.queries_total as f64) * 100.0
                                    )
                                } else {
                                    "starting...".to_string()
                                };
                                eprintln!("Replay {} {}: {}", id, status, progress);
                            }
                            tokio::time::sleep(poll_duration).await;
                        }
                    }
                }
            }

            ReplayCommand::Stop { id } => {
                let uuid = id.parse::<Uuid>()
                    .map_err(|_| CliError::Other(format!("Invalid replay ID: {}", id)))?;
                let path = format!("/api/v1/replays/{}", uuid);
                let _response: ActionResponse = client.delete(&path).await?;
                output::print_success(&format!("Replay {} stopped", id), quiet);
                Ok(())
            }
        }
    }
}

fn parse_window(s: &str) -> Result<u64, CliError> {
    parse_duration(s).map(|d| d.as_secs())
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
