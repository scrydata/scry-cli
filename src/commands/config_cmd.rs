//! Config command implementation.

use crate::config::Config;
use crate::output::OutputFormat;
use clap::Subcommand;
use serde::Serialize;

#[derive(Subcommand, Clone)]
pub enum ConfigCommand {
    /// Set configuration values
    Set {
        /// API endpoint URL
        #[arg(long)]
        api_url: Option<String>,

        /// API authentication token
        #[arg(long)]
        token: Option<String>,
    },

    /// Show current configuration
    Show,
}

#[derive(Debug, Serialize)]
struct ConfigDisplay {
    api_url: String,
    token: String,
    source: String,
}

impl ConfigCommand {
    pub fn run(self, format: OutputFormat) -> anyhow::Result<()> {
        match self {
            ConfigCommand::Set { api_url, token } => {
                let mut config = Config::load().unwrap_or_default();

                if let Some(url) = api_url {
                    config.api_url = Some(url);
                }
                if let Some(t) = token {
                    config.token = Some(t);
                }

                config.save()?;
                println!("Configuration saved to {:?}", Config::config_path()?);
                Ok(())
            }
            ConfigCommand::Show => {
                let config = Config::load().unwrap_or_default();
                let config_path = Config::config_path()?;

                let display = ConfigDisplay {
                    api_url: config.api_url.clone().unwrap_or_else(|| "(not set)".to_string()),
                    token: config.token
                        .as_ref()
                        .map(|t| Config::mask_token(t))
                        .unwrap_or_else(|| "(not set)".to_string()),
                    source: if config_path.exists() {
                        config_path.display().to_string()
                    } else {
                        "(no config file)".to_string()
                    },
                };

                match format {
                    OutputFormat::Json => {
                        println!("{}", serde_json::to_string_pretty(&display)?);
                    }
                    OutputFormat::Table => {
                        println!("API URL: {}", display.api_url);
                        println!("Token:   {}", display.token);
                        println!("Source:  {}", display.source);
                    }
                }

                Ok(())
            }
        }
    }
}
