//! Shadow command implementation.

use crate::client::ApiClient;
use crate::error::CliError;
use crate::output::{self, OutputFormat};
use crate::resolve::ShadowRef;
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use tabled::Tabled;
use uuid::Uuid;

#[derive(Subcommand, Clone)]
pub enum ShadowCommand {
    /// Create a new shadow instance
    Create {
        /// Source to consume journal from (project/database format)
        #[arg(long)]
        source: String,

        /// Shadow name/ID
        #[arg(long)]
        name: Option<String>,

        /// Database name
        #[arg(long, default_value = "shadow")]
        database: String,

        /// Database username
        #[arg(long, default_value = "postgres")]
        username: String,

        /// Database password
        #[arg(long, default_value = "postgres")]
        password: String,

        /// PostgreSQL image
        #[arg(long)]
        image: Option<String>,

        /// Time-to-live (e.g., "30m", "1h")
        #[arg(long)]
        ttl: Option<String>,
    },

    /// List shadow instances
    List {
        /// Filter by source
        #[arg(long)]
        source: Option<String>,

        /// Filter by state
        #[arg(long)]
        state: Option<String>,
    },

    /// Show shadow details
    Show {
        /// Shadow ID or name
        id: String,
    },

    /// Wait for shadow to be ready
    WaitReady {
        /// Shadow ID or name
        id: String,

        /// Timeout duration (e.g., "5m", "10m")
        #[arg(long, default_value = "5m")]
        timeout: String,

        /// Poll interval (e.g., "5s", "10s")
        #[arg(long, default_value = "5s")]
        poll_interval: String,
    },

    /// Destroy a shadow instance
    Destroy {
        /// Shadow ID or name
        id: String,

        /// Don't error if shadow doesn't exist
        #[arg(long)]
        ignore_not_found: bool,
    },

    /// Verify shadow matches source (schema and row counts)
    Verify {
        /// Shadow ID
        id: String,

        /// Wait for shadow to be ready before verifying
        #[arg(long)]
        wait: bool,

        /// Timeout when waiting (e.g., "5m")
        #[arg(long, default_value = "5m")]
        timeout: String,
    },

    /// Show shadow database statistics (table row counts)
    Stats {
        /// Shadow ID
        id: String,
    },
}

// API response types (matching scry-platform-api DTOs)

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateShadowRequest {
    pub source: Option<String>,
    pub database: String,
    pub username: String,
    pub password: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowResponse {
    pub id: Uuid,
    pub source: Option<String>,
    pub state: String,
    pub database: String,
    pub image: String,
    pub port: Option<u16>,
    pub created_at: u64,
    pub events_applied: u64,
    #[serde(default)]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct ShadowListItem {
    #[tabled(rename = "ID")]
    pub id: String,
    #[tabled(rename = "Source")]
    pub source: String,
    #[tabled(rename = "State")]
    pub state: String,
    #[tabled(rename = "Database")]
    pub database: String,
    #[tabled(rename = "Port")]
    pub port: String,
    #[tabled(rename = "Events")]
    pub events_applied: u64,
}

impl From<ShadowResponse> for ShadowListItem {
    fn from(s: ShadowResponse) -> Self {
        Self {
            id: s.id.to_string()[..8].to_string(), // Short ID
            source: s.source.unwrap_or_else(|| "-".to_string()),
            state: s.state,
            database: s.database,
            port: s.port.map(|p| p.to_string()).unwrap_or_else(|| "-".to_string()),
            events_applied: s.events_applied,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationSummary {
    pub tables_matched: u32,
    pub tables_mismatched: u32,
    pub tables_missing: u32,
    pub tables_unexpected: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableVerification {
    pub schema: String,
    pub table: String,
    pub expected_count: Option<u64>,
    pub actual_count: Option<u64>,
    pub matches: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResponse {
    pub id: Uuid,
    pub shadow_id: Uuid,
    pub verified_at: u64,
    pub status: String,
    pub summary: VerificationSummary,
    pub tables: Vec<TableVerification>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableStats {
    pub schema: String,
    pub table: String,
    pub row_count: i64,
    pub size_bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct TableStatsRow {
    #[tabled(rename = "Schema")]
    pub schema: String,
    #[tabled(rename = "Table")]
    pub table: String,
    #[tabled(rename = "Rows")]
    pub row_count: i64,
    #[tabled(rename = "Size")]
    pub size: String,
}

impl From<TableStats> for TableStatsRow {
    fn from(s: TableStats) -> Self {
        let size = match s.size_bytes {
            Some(b) if b >= 1_073_741_824 => format!("{:.1} GB", b as f64 / 1_073_741_824.0),
            Some(b) if b >= 1_048_576 => format!("{:.1} MB", b as f64 / 1_048_576.0),
            Some(b) if b >= 1024 => format!("{:.1} KB", b as f64 / 1024.0),
            Some(b) => format!("{} B", b),
            None => "-".to_string(),
        };
        Self {
            schema: s.schema,
            table: s.table,
            row_count: s.row_count,
            size,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowStatsResponse {
    pub shadow_id: Uuid,
    pub database: String,
    pub tables: Vec<TableStats>,
    pub total_rows: i64,
    pub collected_at: u64,
}

impl ShadowCommand {
    pub async fn run(
        self,
        client: &ApiClient,
        format: OutputFormat,
        quiet: bool,
    ) -> Result<(), CliError> {
        match self {
            ShadowCommand::Create {
                source,
                name,
                database,
                username,
                password,
                image,
                ttl: _ttl, // TODO: Implement TTL support on server
            } => {
                let shadow_id = name.map(|n| {
                    // Try to parse as UUID, otherwise generate one
                    n.parse::<Uuid>().unwrap_or_else(|_| Uuid::new_v4())
                });

                let request = CreateShadowRequest {
                    source: Some(source),
                    database,
                    username,
                    password,
                    image,
                    shadow_id,
                };

                let response: ShadowResponse = client.post("/api/v1/shadows", &request).await?;
                if quiet {
                    // Output only the ID for scripting
                    println!("{}", response.id);
                } else {
                    output::print_item(&response, format)?;
                }
                Ok(())
            }

            ShadowCommand::List { source, state } => {
                let mut path = "/api/v1/shadows".to_string();
                let mut params = Vec::new();

                if let Some(src) = source {
                    params.push(format!("source={}", src));
                }
                if let Some(st) = state {
                    params.push(format!("state={}", st));
                }

                if !params.is_empty() {
                    path = format!("{}?{}", path, params.join("&"));
                }

                let response: PageResponse<ShadowResponse> = client.get(&path).await?;
                let items: Vec<ShadowListItem> = response.items.into_iter().map(Into::into).collect();
                output::print_table(&items, format)?;
                Ok(())
            }

            ShadowCommand::Show { id } => {
                let shadow_id = ShadowRef::parse(&id)?.resolve(client).await?;
                let path = format!("/api/v1/shadows/{}", shadow_id);
                let response: ShadowResponse = client.get(&path).await?;
                output::print_item(&response, format)?;
                Ok(())
            }

            ShadowCommand::WaitReady {
                id,
                timeout,
                poll_interval,
            } => {
                let shadow_id = ShadowRef::parse(&id)?.resolve(client).await?;
                let timeout_duration = parse_duration(&timeout)?;
                let poll_duration = parse_duration(&poll_interval)?;

                let start = std::time::Instant::now();

                loop {
                    let path = format!("/api/v1/shadows/{}", shadow_id);
                    let response: ShadowResponse = client.get(&path).await?;

                    match response.state.as_str() {
                        "ready" => {
                            output::print_success(&format!("Shadow {} is ready", id), quiet);
                            return Ok(());
                        }
                        "failed" => {
                            let msg = response.error_message.unwrap_or_else(|| "Unknown error".to_string());
                            return Err(CliError::Api(format!("Shadow failed: {}", msg)));
                        }
                        state => {
                            if start.elapsed() > timeout_duration {
                                return Err(CliError::Timeout(format!(
                                    "shadow {} to become ready (current state: {})",
                                    id, state
                                )));
                            }
                            if !quiet {
                                eprintln!("Shadow {} state: {}...", id, state);
                            }
                            tokio::time::sleep(poll_duration).await;
                        }
                    }
                }
            }

            ShadowCommand::Destroy { id, ignore_not_found } => {
                let shadow_id = ShadowRef::parse(&id)?.resolve(client).await?;
                let path = format!("/api/v1/shadows/{}", shadow_id);

                match client.delete::<ActionResponse>(&path).await {
                    Ok(_) => {
                        output::print_success(&format!("Shadow {} destroyed", id), quiet);
                        Ok(())
                    }
                    Err(CliError::NotFound(_)) if ignore_not_found => {
                        output::print_success(&format!("Shadow {} not found (ignored)", id), quiet);
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }

            ShadowCommand::Verify { id, wait, timeout } => {
                let shadow_id = ShadowRef::parse(&id)?.resolve(client).await?;

                if wait {
                    // Wait for ready first
                    let timeout_duration = parse_duration(&timeout)?;
                    let poll_duration = std::time::Duration::from_secs(5);
                    let start = std::time::Instant::now();

                    loop {
                        let path = format!("/api/v1/shadows/{}", shadow_id);
                        let response: ShadowResponse = client.get(&path).await?;

                        match response.state.as_str() {
                            "ready" => break,
                            "failed" => {
                                let msg = response.error_message.unwrap_or_else(|| "Unknown error".to_string());
                                return Err(CliError::Api(format!("Shadow failed: {}", msg)));
                            }
                            state => {
                                if start.elapsed() > timeout_duration {
                                    return Err(CliError::Timeout(format!(
                                        "shadow {} to become ready (current state: {})",
                                        id, state
                                    )));
                                }
                                if !quiet {
                                    eprintln!("Waiting for shadow (state: {})...", state);
                                }
                                tokio::time::sleep(poll_duration).await;
                            }
                        }
                    }
                }

                // Trigger verification
                let path = format!("/api/v1/shadows/{}/verify", shadow_id);
                let response: VerificationResponse = client.post_empty(&path).await?;

                // Print result
                if quiet {
                    println!("{}", response.status);
                } else {
                    match format {
                        OutputFormat::Json => {
                            output::print_item(&response, format)?;
                        }
                        OutputFormat::Table => {
                            println!("Verification: {}", response.status.to_uppercase());
                            println!("  Tables matched:    {}", response.summary.tables_matched);
                            println!("  Tables mismatched: {}", response.summary.tables_mismatched);
                            println!("  Tables missing:    {}", response.summary.tables_missing);
                            println!("  Tables unexpected: {}", response.summary.tables_unexpected);
                        }
                    }
                }

                // Exit with error if not matched
                if response.status != "matched" {
                    return Err(CliError::Other(format!(
                        "Verification failed: {}",
                        response.status
                    )));
                }

                Ok(())
            }

            ShadowCommand::Stats { id } => {
                let shadow_id = ShadowRef::parse(&id)?.resolve(client).await?;
                let path = format!("/api/v1/shadows/{}/stats", shadow_id);
                let response: ShadowStatsResponse = client.get(&path).await?;

                match format {
                    OutputFormat::Json => {
                        output::print_item(&response, format)?;
                    }
                    OutputFormat::Table => {
                        let rows: Vec<TableStatsRow> =
                            response.tables.into_iter().map(Into::into).collect();
                        output::print_table(&rows, format)?;
                        if !quiet {
                            println!("\nTotal rows: {}", response.total_rows);
                        }
                    }
                }
                Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("5s").unwrap(), std::time::Duration::from_secs(5));
        assert_eq!(parse_duration("10m").unwrap(), std::time::Duration::from_secs(600));
        assert_eq!(parse_duration("1h").unwrap(), std::time::Duration::from_secs(3600));
        assert_eq!(parse_duration("500ms").unwrap(), std::time::Duration::from_millis(500));
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert!(parse_duration("5").is_err());
        assert!(parse_duration("abc").is_err());
    }
}
