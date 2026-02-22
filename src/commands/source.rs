//! Source command implementation.

use crate::client::ApiClient;
use crate::error::CliError;
use crate::output::{self, OutputFormat};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use tabled::Tabled;

#[derive(Subcommand, Clone)]
pub enum SourceCommand {
    /// Register a new source
    Register {
        /// Source name in project/database format (e.g., "acme/orders")
        #[arg(long)]
        name: String,

        /// Connection string for the source database (not stored, used for validation)
        #[arg(long)]
        connection_string: Option<String>,
    },

    /// List registered sources
    List,

    /// Show source details
    Show {
        /// Source name (project/database format)
        name: String,
    },

    /// Delete a source
    Delete {
        /// Source name (project/database format)
        name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSourceRequest {
    pub project: String,
    pub database: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceResponse {
    pub project: String,
    pub database: String,
    pub source_id: String,
    pub subject_token: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct SourceListItem {
    #[tabled(rename = "Source")]
    pub source_id: String,
    #[tabled(rename = "Project")]
    pub project: String,
    #[tabled(rename = "Database")]
    pub database: String,
    #[tabled(rename = "Subject Token")]
    pub subject_token: String,
}

impl From<SourceResponse> for SourceListItem {
    fn from(s: SourceResponse) -> Self {
        Self {
            source_id: s.source_id,
            project: s.project,
            database: s.database,
            subject_token: s.subject_token,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSourcesResponse {
    pub sources: Vec<SourceResponse>,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResponse {
    pub success: bool,
    pub message: Option<String>,
}

impl SourceCommand {
    pub async fn run(
        self,
        client: &ApiClient,
        format: OutputFormat,
        quiet: bool,
    ) -> Result<(), CliError> {
        match self {
            SourceCommand::Register {
                name,
                connection_string: _,
            } => {
                let (project, database) = parse_source_name(&name)?;

                let request = CreateSourceRequest {
                    project: project.to_string(),
                    database: database.to_string(),
                };

                let response: SourceResponse = client.post("/api/v1/sources", &request).await?;
                output::print_item(&response, format)?;
                Ok(())
            }

            SourceCommand::List => {
                let response: ListSourcesResponse = client.get("/api/v1/sources").await?;
                let items: Vec<SourceListItem> = response.sources.into_iter().map(Into::into).collect();
                output::print_table(&items, format)?;
                Ok(())
            }

            SourceCommand::Show { name } => {
                let (project, database) = parse_source_name(&name)?;
                let path = format!("/api/v1/sources/{}/{}", project, database);
                let response: SourceResponse = client.get(&path).await?;
                output::print_item(&response, format)?;
                Ok(())
            }

            SourceCommand::Delete { name } => {
                let (project, database) = parse_source_name(&name)?;
                let path = format!("/api/v1/sources/{}/{}", project, database);
                let _response: ActionResponse = client.delete(&path).await?;
                output::print_success(&format!("Source {} deleted", name), quiet);
                Ok(())
            }
        }
    }
}

fn parse_source_name(name: &str) -> Result<(&str, &str), CliError> {
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() != 2 {
        return Err(CliError::Other(format!(
            "Invalid source name: '{}'. Expected format: project/database",
            name
        )));
    }
    Ok((parts[0], parts[1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_source_name_valid() {
        let (project, database) = parse_source_name("acme/orders").unwrap();
        assert_eq!(project, "acme");
        assert_eq!(database, "orders");
    }

    #[test]
    fn test_parse_source_name_invalid() {
        assert!(parse_source_name("invalid").is_err());
        assert!(parse_source_name("a/b/c").is_err());
    }
}
