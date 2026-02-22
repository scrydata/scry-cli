//! Abstraction over version control systems.
//!
//! Currently only Git is supported; designed for future extension.

use std::process::Command;

/// Abstraction over version control systems.
pub trait Dvcs {
    /// Get list of changed files between base ref and HEAD
    fn changed_files(&self, base_ref: &str) -> Result<Vec<String>, DvcsError>;

    /// Get diff content for a specific file
    fn file_diff(&self, base_ref: &str, file: &str) -> Result<String, DvcsError>;
}

#[derive(Debug, thiserror::Error)]
pub enum DvcsError {
    #[error("Git command failed: {0}")]
    GitError(String),
    #[error("Not a git repository")]
    NotARepo,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct Git {
    repo_root: std::path::PathBuf,
}

impl Git {
    pub fn discover() -> Result<Self, DvcsError> {
        let mut current = std::env::current_dir()?;
        loop {
            if current.join(".git").exists() {
                return Ok(Self { repo_root: current });
            }
            if !current.pop() {
                return Err(DvcsError::NotARepo);
            }
        }
    }
}

impl Dvcs for Git {
    fn changed_files(&self, base_ref: &str) -> Result<Vec<String>, DvcsError> {
        let output = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["diff", "--name-only", &format!("{}...HEAD", base_ref)])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DvcsError::GitError(stderr.to_string()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.lines().map(|s| s.to_string()).collect())
    }

    fn file_diff(&self, base_ref: &str, file: &str) -> Result<String, DvcsError> {
        let output = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["diff", &format!("{}...HEAD", base_ref), "--", file])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DvcsError::GitError(stderr.to_string()));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discover_in_git_repo() {
        // This test runs from within the scry-platform repo, so discover should succeed
        let git = Git::discover();
        assert!(git.is_ok());
    }
}
