//! Checkpoints command implementation.

use crate::client::ApiClient;
use crate::error::CliError;
use crate::output::{self, OutputFormat};
use crate::resolve::ShadowRef;
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use tabled::Tabled;
use uuid::Uuid;

#[derive(Subcommand, Clone)]
pub enum CheckpointsCommand {
    /// List checkpoints for a shadow
    List {
        /// Shadow reference (UUID or source/shadow-name)
        shadow: String,
    },

    /// Create a checkpoint
    Create {
        /// Shadow reference (UUID or source/shadow-name)
        shadow: String,

        /// Checkpoint name
        #[arg(long)]
        name: String,
    },

    /// Delete a checkpoint
    Delete {
        /// Shadow reference (UUID or source/shadow-name)
        shadow: String,

        /// Checkpoint name
        #[arg(long)]
        name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointResponse {
    pub id: Uuid,
    pub name: String,
    pub shadow_id: Uuid,
    pub cdc_position: u64,
    pub query_position: Option<u64>,
    pub size_bytes: Option<u64>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct CheckpointListItem {
    #[tabled(rename = "NAME")]
    pub name: String,
    #[tabled(rename = "CREATED")]
    pub created: String,
    #[tabled(rename = "JOURNAL POS")]
    pub journal_pos: String,
    #[tabled(rename = "SIZE")]
    pub size: String,
    #[tabled(rename = "EST. RESTORE")]
    pub est_restore: String,
}

impl From<CheckpointResponse> for CheckpointListItem {
    fn from(c: CheckpointResponse) -> Self {
        let created = format_relative_time(c.created_at);
        let size_bytes = c.size_bytes.unwrap_or(0);
        let size = format_bytes(size_bytes);
        let est_restore = estimate_restore_time(size_bytes);

        Self {
            name: c.name,
            created,
            journal_pos: c.cdc_position.to_string(),
            size,
            est_restore,
        }
    }
}

fn format_relative_time(unix_millis: u64) -> String {
    // SystemTime is always after UNIX_EPOCH on supported platforms
    #[allow(clippy::unwrap_used)]
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let diff_secs = (now.saturating_sub(unix_millis)) / 1000;

    if diff_secs < 60 {
        format!("{}s ago", diff_secs)
    } else if diff_secs < 3600 {
        format!("{}m ago", diff_secs / 60)
    } else if diff_secs < 86400 {
        format!("{}h ago", diff_secs / 3600)
    } else {
        format!("{}d ago", diff_secs / 86400)
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn estimate_restore_time(size_bytes: u64) -> String {
    // Rough estimate: ~100 MB/s restore speed
    let restore_speed_bps = 100 * 1024 * 1024;
    let secs = size_bytes / restore_speed_bps;

    if secs < 60 {
        format!("~{}s", secs.max(1))
    } else {
        format!("~{}m", (secs + 30) / 60)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCheckpointRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageResponse<T> {
    pub items: Vec<T>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResponse {
    pub success: bool,
    pub message: Option<String>,
}

impl CheckpointsCommand {
    pub async fn run(
        self,
        client: &ApiClient,
        format: OutputFormat,
        quiet: bool,
    ) -> Result<(), CliError> {
        match self {
            CheckpointsCommand::List { shadow } => {
                let shadow_id = ShadowRef::parse(&shadow)?.resolve(client).await?;
                let path = format!("/api/v1/shadows/{}/checkpoints", shadow_id);
                let response: PageResponse<CheckpointResponse> = client.get(&path).await?;
                let items: Vec<CheckpointListItem> =
                    response.items.into_iter().map(Into::into).collect();
                output::print_table(&items, format)?;
                Ok(())
            }

            CheckpointsCommand::Create { shadow, name } => {
                let shadow_id = ShadowRef::parse(&shadow)?.resolve(client).await?;
                let path = format!("/api/v1/shadows/{}/checkpoints", shadow_id);
                let request = CreateCheckpointRequest { name: name.clone() };
                let response: CheckpointResponse = client.post(&path, &request).await?;
                output::print_success(&format!("Checkpoint '{}' created", response.name), quiet);
                Ok(())
            }

            CheckpointsCommand::Delete { shadow, name } => {
                let shadow_id = ShadowRef::parse(&shadow)?.resolve(client).await?;
                let path = format!("/api/v1/shadows/{}/checkpoints/{}", shadow_id, name);
                client.delete::<ActionResponse>(&path).await?;
                output::print_success(&format!("Checkpoint '{}' deleted", name), quiet);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(2_500_000), "2.4 MB");
        assert_eq!(format_bytes(2_500_000_000), "2.3 GB");
    }

    #[test]
    fn test_estimate_restore_time() {
        // 100 MB/s = 104_857_600 bytes/s
        assert_eq!(estimate_restore_time(50_000_000), "~1s");   // 50MB -> ~0.5s -> 1s min
        assert_eq!(estimate_restore_time(500_000_000), "~4s");  // 500MB -> ~4.76s
        assert_eq!(estimate_restore_time(5_000_000_000), "~47s"); // 5GB -> ~47.68s
        assert_eq!(estimate_restore_time(10_000_000_000), "~2m"); // 10GB -> ~95s -> 2m
    }
}
