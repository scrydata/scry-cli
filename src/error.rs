//! CLI error types.

use thiserror::Error;

/// CLI error type.
#[derive(Debug, Error)]
pub enum CliError {
    #[error("API error: {0}")]
    Api(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("Timeout waiting for {0}")]
    Timeout(String),

    #[error("Missing required configuration: {0}")]
    MissingConfig(String),

    #[error("Checkpoint not found: {0}")]
    CheckpointNotFound(String),

    #[error("Regressions detected: {0} regressions found")]
    RegressionsDetected(u64),

    #[error("Drift threshold exceeded: max drift was {0}ms")]
    DriftExceeded(u64),

    #[error("{0}")]
    Other(String),

    #[error("{0}")]
    Anyhow(#[from] anyhow::Error),

    #[error("Command exited with code {0}")]
    ExitCode(i32),
}

/// Exit codes for CI integration.
pub mod exit_codes {
    pub const GENERAL_ERROR: i32 = 1;
    pub const TIMEOUT: i32 = 2;
    pub const NOT_FOUND: i32 = 3;
    pub const CHECKPOINT_NOT_FOUND: i32 = 4;
    pub const REGRESSIONS_DETECTED: i32 = 5;
    pub const DRIFT_EXCEEDED: i32 = 6;
}

impl CliError {
    /// Get the exit code for this error.
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::NotFound(_) => exit_codes::NOT_FOUND,
            CliError::Timeout(_) => exit_codes::TIMEOUT,
            CliError::CheckpointNotFound(_) => exit_codes::CHECKPOINT_NOT_FOUND,
            CliError::RegressionsDetected(_) => exit_codes::REGRESSIONS_DETECTED,
            CliError::DriftExceeded(_) => exit_codes::DRIFT_EXCEEDED,
            CliError::ExitCode(code) => *code,
            _ => exit_codes::GENERAL_ERROR,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exit_codes() {
        assert_eq!(CliError::Other("test".into()).exit_code(), 1);
        assert_eq!(CliError::Timeout("test".into()).exit_code(), 2);
        assert_eq!(CliError::NotFound("test".into()).exit_code(), 3);
        assert_eq!(CliError::CheckpointNotFound("test".into()).exit_code(), 4);
        assert_eq!(CliError::RegressionsDetected(5).exit_code(), 5);
        assert_eq!(CliError::DriftExceeded(1000).exit_code(), 6);
    }
}
