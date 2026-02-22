//! Repository-level configuration for scry-cli.
//!
//! Reads `.scry.toml` from the repository root to configure migration detection
//! and CI/CD integration.

use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize, Default)]
pub struct RepoConfig {
    #[serde(default)]
    pub migration_detection: MigrationDetectionConfig,
    #[serde(default)]
    pub ci: CiConfig,
}

#[derive(Debug, Deserialize)]
pub struct MigrationDetectionConfig {
    #[serde(default = "default_paths")]
    pub paths: Vec<String>,
    #[serde(default = "default_extensions")]
    pub extensions: Vec<String>,
    #[serde(default = "default_scan_ddl")]
    pub scan_ddl: bool,
    #[serde(default)]
    pub exclude: Vec<String>,
}

fn default_paths() -> Vec<String> {
    vec![
        "migrations/".into(),
        "db/migrate/".into(),
        "alembic/".into(),
        "prisma/migrations/".into(),
        "flyway/".into(),
        "liquibase/".into(),
    ]
}

fn default_extensions() -> Vec<String> {
    vec![
        ".sql".into(),
        ".py".into(),
        ".rb".into(),
        ".ts".into(),
        ".js".into(),
    ]
}

fn default_scan_ddl() -> bool {
    true
}

impl Default for MigrationDetectionConfig {
    fn default() -> Self {
        Self {
            paths: default_paths(),
            extensions: default_extensions(),
            scan_ddl: default_scan_ddl(),
            exclude: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CiConfig {
    /// Default shadow reference for test-migration command
    pub shadow_ref: Option<String>,
    /// Default replay window (e.g., "2h", "4h")
    #[serde(default = "default_replay_window")]
    pub replay_window: String,
    /// Default speed multiplier (1.0 = real-time, 0 = fast-forward)
    #[allow(dead_code)]
    pub replay_speed: Option<f64>,
    /// Default max drift threshold (e.g., "10s", "500ms")
    #[allow(dead_code)]
    pub replay_max_drift: Option<String>,
}

fn default_replay_window() -> String {
    "2h".into()
}

impl Default for CiConfig {
    fn default() -> Self {
        Self {
            shadow_ref: None,
            replay_window: default_replay_window(),
            replay_speed: None,
            replay_max_drift: None,
        }
    }
}

impl RepoConfig {
    /// Load from .scry.toml in repo root, or return defaults if not found
    pub fn load(repo_root: &Path) -> Self {
        let config_path = repo_root.join(".scry.toml");
        if config_path.exists() {
            match std::fs::read_to_string(&config_path) {
                Ok(contents) => match toml::from_str(&contents) {
                    Ok(config) => return config,
                    Err(e) => {
                        eprintln!("Warning: Failed to parse .scry.toml: {}", e);
                    }
                },
                Err(e) => {
                    eprintln!("Warning: Failed to read .scry.toml: {}", e);
                }
            }
        }
        Self::default()
    }

    /// Find repo root by walking up from current directory looking for .git
    pub fn find_repo_root() -> Option<std::path::PathBuf> {
        let mut current = std::env::current_dir().ok()?;
        loop {
            if current.join(".git").exists() {
                return Some(current);
            }
            if !current.pop() {
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = RepoConfig::default();
        assert!(config.migration_detection.paths.contains(&"migrations/".to_string()));
        assert!(config.migration_detection.scan_ddl);
        assert_eq!(config.ci.replay_window, "2h");
    }

    #[test]
    fn test_load_from_file() {
        let dir = TempDir::new().unwrap();
        let config_content = r#"
[migration_detection]
paths = ["custom/migrations/"]
scan_ddl = false

[ci]
shadow_ref = "prod-db/test"
"#;
        let config_path = dir.path().join(".scry.toml");
        let mut file = std::fs::File::create(&config_path).unwrap();
        file.write_all(config_content.as_bytes()).unwrap();

        let config = RepoConfig::load(dir.path());
        assert_eq!(config.migration_detection.paths, vec!["custom/migrations/"]);
        assert!(!config.migration_detection.scan_ddl);
        assert_eq!(config.ci.shadow_ref, Some("prod-db/test".to_string()));
    }

    #[test]
    fn test_load_missing_file() {
        let dir = TempDir::new().unwrap();
        let config = RepoConfig::load(dir.path());
        // Should return defaults
        assert!(config.migration_detection.paths.contains(&"migrations/".to_string()));
    }

    #[test]
    fn test_ci_config_replay_options() {
        let dir = TempDir::new().unwrap();
        let config_content = r#"
[ci]
shadow_ref = "prod-db/test"
replay_speed = 2.0
replay_max_drift = "10s"
"#;
        let config_path = dir.path().join(".scry.toml");
        let mut file = std::fs::File::create(&config_path).unwrap();
        file.write_all(config_content.as_bytes()).unwrap();

        let config = RepoConfig::load(dir.path());
        assert_eq!(config.ci.replay_speed, Some(2.0));
        assert_eq!(config.ci.replay_max_drift, Some("10s".to_string()));
    }
}
