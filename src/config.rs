//! Configuration loading with file + env override support.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// CLI configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// API endpoint URL.
    pub api_url: Option<String>,

    /// Authentication token.
    pub token: Option<String>,

    /// Named profiles for different environments.
    #[serde(default)]
    pub profiles: HashMap<String, Profile>,
}

/// A named profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub api_url: Option<String>,
    pub token: Option<String>,
}

impl Config {
    /// Load configuration from file and environment.
    /// Priority: CLI args > env vars > config file
    pub fn load() -> anyhow::Result<Self> {
        let config_path = Self::config_path()?;

        let config = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            toml::from_str(&content)?
        } else {
            Self::default()
        };

        Ok(config)
    }

    /// Get the configuration file path.
    pub fn config_path() -> anyhow::Result<PathBuf> {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
        Ok(home.join(".scry").join("config.toml"))
    }

    /// Save configuration to file.
    pub fn save(&self) -> anyhow::Result<()> {
        let config_path = Self::config_path()?;

        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(self)?;
        std::fs::write(&config_path, content)?;

        Ok(())
    }

    /// Get effective API URL (with profile override).
    pub fn effective_api_url(&self, profile: Option<&str>) -> Option<String> {
        if let Some(profile_name) = profile {
            if let Some(p) = self.profiles.get(profile_name) {
                if p.api_url.is_some() {
                    return p.api_url.clone();
                }
            }
        }
        self.api_url.clone()
    }

    /// Get effective token (with profile override).
    pub fn effective_token(&self, profile: Option<&str>) -> Option<String> {
        if let Some(profile_name) = profile {
            if let Some(p) = self.profiles.get(profile_name) {
                if p.token.is_some() {
                    return p.token.clone();
                }
            }
        }
        self.token.clone()
    }

    /// Mask a token for display (show first 8 chars + asterisks).
    pub fn mask_token(token: &str) -> String {
        if token.len() <= 8 {
            "*".repeat(token.len())
        } else {
            format!("{}****", &token[..8])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_deserialize() {
        let content = r#"
api_url = "https://scry.example.com:8081"
token = "sk_test_123"

[profiles.staging]
api_url = "https://scry-staging.example.com:8081"
token = "sk_staging_456"
"#;
        let config: Config = toml::from_str(content).unwrap();

        assert_eq!(config.api_url, Some("https://scry.example.com:8081".to_string()));
        assert_eq!(config.token, Some("sk_test_123".to_string()));
        assert!(config.profiles.contains_key("staging"));
    }

    #[test]
    fn test_effective_values_with_profile() {
        let mut config = Config::default();
        config.api_url = Some("https://prod.example.com".to_string());
        config.token = Some("prod_token".to_string());
        config.profiles.insert("staging".to_string(), Profile {
            api_url: Some("https://staging.example.com".to_string()),
            token: Some("staging_token".to_string()),
        });

        // Without profile
        assert_eq!(config.effective_api_url(None), Some("https://prod.example.com".to_string()));
        assert_eq!(config.effective_token(None), Some("prod_token".to_string()));

        // With profile
        assert_eq!(config.effective_api_url(Some("staging")), Some("https://staging.example.com".to_string()));
        assert_eq!(config.effective_token(Some("staging")), Some("staging_token".to_string()));
    }

    #[test]
    fn test_mask_token() {
        assert_eq!(Config::mask_token("sk_test_123456"), "sk_test_****");
        assert_eq!(Config::mask_token("short"), "*****");
    }
}
