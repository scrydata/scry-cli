//! Source command implementation.

use crate::client::ApiClient;
use crate::error::CliError;
use crate::output::{self, OutputFormat};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use tabled::Tabled;

#[derive(Subcommand, Clone)]
pub enum SourceCommand {
    /// List registered sources (derived from shadows).
    List,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SourceSummary {
    pub source: String,
    pub shadow_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListSourcesResponse {
    pub sources: Vec<SourceSummary>,
    #[allow(dead_code)]
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct SourceListItem {
    #[tabled(rename = "Source")]
    pub source: String,
    #[tabled(rename = "Shadows")]
    pub shadows: usize,
}

impl From<SourceSummary> for SourceListItem {
    fn from(s: SourceSummary) -> Self {
        Self {
            source: s.source,
            shadows: s.shadow_count,
        }
    }
}

impl SourceCommand {
    pub async fn run(
        self,
        client: &ApiClient,
        format: OutputFormat,
        _quiet: bool,
    ) -> Result<(), CliError> {
        match self {
            SourceCommand::List => {
                let response: ListSourcesResponse = client.get("/api/v1/sources").await?;
                let items: Vec<SourceListItem> =
                    response.sources.into_iter().map(Into::into).collect();
                output::print_table(&items, format)?;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_summary_maps_to_list_item() {
        let item: SourceListItem = SourceSummary {
            source: "prod-db".to_string(),
            shadow_count: 3,
        }
        .into();
        assert_eq!(item.source, "prod-db");
        assert_eq!(item.shadows, 3);
    }
}
