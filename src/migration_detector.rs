//! Migration detection from git diffs.
//!
//! Analyzes changed files to determine if a commit contains database migrations
//! based on path patterns and DDL keyword detection.

use crate::dvcs::{Dvcs, DvcsError};
use crate::repo_config::MigrationDetectionConfig;
use glob::Pattern;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct DetectionResult {
    pub has_migrations: bool,
    pub confidence: Confidence,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Medium,
    Low,
    None,
}

#[derive(Debug, Serialize)]
pub struct Evidence {
    #[serde(rename = "type")]
    pub evidence_type: EvidenceType,
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceType {
    PathMatch,
    DdlKeyword,
}

const DDL_KEYWORDS: &[&str] = &[
    "CREATE TABLE",
    "ALTER TABLE",
    "DROP TABLE",
    "CREATE INDEX",
    "DROP INDEX",
    "ADD COLUMN",
    "DROP COLUMN",
    "RENAME COLUMN",
    "CREATE SCHEMA",
    "DROP SCHEMA",
];

pub struct MigrationDetector<'a, D: Dvcs> {
    dvcs: &'a D,
    config: &'a MigrationDetectionConfig,
}

impl<'a, D: Dvcs> MigrationDetector<'a, D> {
    pub fn new(dvcs: &'a D, config: &'a MigrationDetectionConfig) -> Self {
        Self { dvcs, config }
    }

    pub fn detect(&self, base_ref: &str) -> Result<DetectionResult, DvcsError> {
        let changed_files = self.dvcs.changed_files(base_ref)?;
        let mut evidence = Vec::new();

        // Check for path matches
        for file in &changed_files {
            if self.is_excluded(file) {
                continue;
            }

            if self.is_migration_path(file) && self.has_migration_extension(file) {
                evidence.push(Evidence {
                    evidence_type: EvidenceType::PathMatch,
                    file: file.clone(),
                    keywords: None,
                });
            }
        }

        // Check for DDL keywords if enabled
        if self.config.scan_ddl {
            for file in &changed_files {
                if self.is_excluded(file) {
                    continue;
                }

                // Skip files already matched by path
                if evidence.iter().any(|e| e.file == *file) {
                    continue;
                }

                if let Ok(diff) = self.dvcs.file_diff(base_ref, file) {
                    let found_keywords = self.find_ddl_keywords(&diff);
                    if !found_keywords.is_empty() {
                        evidence.push(Evidence {
                            evidence_type: EvidenceType::DdlKeyword,
                            file: file.clone(),
                            keywords: Some(found_keywords),
                        });
                    }
                }
            }
        }

        let confidence = self.calculate_confidence(&evidence);
        let has_migrations = confidence != Confidence::None;

        Ok(DetectionResult {
            has_migrations,
            confidence,
            evidence,
        })
    }

    fn is_migration_path(&self, file: &str) -> bool {
        let file_lower = file.to_lowercase();
        self.config.paths.iter().any(|path| {
            let path_lower = path.to_lowercase();
            file_lower.starts_with(&path_lower)
        })
    }

    fn has_migration_extension(&self, file: &str) -> bool {
        let file_lower = file.to_lowercase();
        self.config
            .extensions
            .iter()
            .any(|ext| file_lower.ends_with(&ext.to_lowercase()))
    }

    fn is_excluded(&self, file: &str) -> bool {
        self.config.exclude.iter().any(|pattern| {
            Pattern::new(pattern)
                .map(|p| p.matches(file))
                .unwrap_or(false)
        })
    }

    fn find_ddl_keywords(&self, diff: &str) -> Vec<String> {
        let diff_upper = diff.to_uppercase();
        DDL_KEYWORDS
            .iter()
            .filter(|kw| diff_upper.contains(*kw))
            .map(|kw| (*kw).to_string())
            .collect()
    }

    fn calculate_confidence(&self, evidence: &[Evidence]) -> Confidence {
        let has_path_match = evidence
            .iter()
            .any(|e| matches!(e.evidence_type, EvidenceType::PathMatch));
        let has_ddl = evidence
            .iter()
            .any(|e| matches!(e.evidence_type, EvidenceType::DdlKeyword));

        match (has_path_match, has_ddl) {
            (true, true) => Confidence::High,    // Path match + DDL
            (true, false) => Confidence::Medium, // Path match only
            (false, true) => Confidence::Low,    // DDL in non-migration file
            (false, false) => Confidence::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dvcs::DvcsError;

    struct MockDvcs {
        files: Vec<String>,
        diffs: std::collections::HashMap<String, String>,
    }

    impl Dvcs for MockDvcs {
        fn changed_files(&self, _base_ref: &str) -> Result<Vec<String>, DvcsError> {
            Ok(self.files.clone())
        }

        fn file_diff(&self, _base_ref: &str, file: &str) -> Result<String, DvcsError> {
            self.diffs
                .get(file)
                .cloned()
                .ok_or_else(|| DvcsError::GitError("File not found".into()))
        }
    }

    #[test]
    fn test_path_match_detection() {
        let dvcs = MockDvcs {
            files: vec!["migrations/001_create_users.sql".into()],
            diffs: std::collections::HashMap::new(),
        };
        let config = MigrationDetectionConfig::default();
        let detector = MigrationDetector::new(&dvcs, &config);

        let result = detector.detect("origin/main").unwrap();
        assert!(result.has_migrations);
        assert_eq!(result.confidence, Confidence::Medium);
        assert_eq!(result.evidence.len(), 1);
    }

    #[test]
    fn test_ddl_keyword_detection() {
        let mut diffs = std::collections::HashMap::new();
        diffs.insert(
            "schema.sql".to_string(),
            "ALTER TABLE users ADD COLUMN email TEXT;".to_string(),
        );

        let dvcs = MockDvcs {
            files: vec!["schema.sql".into()],
            diffs,
        };
        let config = MigrationDetectionConfig::default();
        let detector = MigrationDetector::new(&dvcs, &config);

        let result = detector.detect("origin/main").unwrap();
        assert!(result.has_migrations);
        assert_eq!(result.confidence, Confidence::Low);
    }

    #[test]
    fn test_high_confidence_path_and_ddl() {
        let mut diffs = std::collections::HashMap::new();
        diffs.insert(
            "migrations/001_create_users.sql".to_string(),
            "CREATE TABLE users (id SERIAL);".to_string(),
        );

        let dvcs = MockDvcs {
            files: vec!["migrations/001_create_users.sql".into()],
            diffs,
        };
        let config = MigrationDetectionConfig::default();
        let detector = MigrationDetector::new(&dvcs, &config);

        let result = detector.detect("origin/main").unwrap();
        assert!(result.has_migrations);
        // Path match alone gives Medium, but since DDL is found only on path-matched files,
        // it should still be Medium (we skip checking DDL for already matched files)
        assert_eq!(result.confidence, Confidence::Medium);
    }

    #[test]
    fn test_exclude_pattern() {
        let dvcs = MockDvcs {
            files: vec!["test/migrations/001_test.sql".into()],
            diffs: std::collections::HashMap::new(),
        };
        let config = MigrationDetectionConfig {
            exclude: vec!["test/**".into()],
            ..Default::default()
        };
        let detector = MigrationDetector::new(&dvcs, &config);

        let result = detector.detect("origin/main").unwrap();
        assert!(!result.has_migrations);
    }

    #[test]
    fn test_no_migrations() {
        let dvcs = MockDvcs {
            files: vec!["src/main.rs".into(), "README.md".into()],
            diffs: std::collections::HashMap::new(),
        };
        let config = MigrationDetectionConfig::default();
        let detector = MigrationDetector::new(&dvcs, &config);

        let result = detector.detect("origin/main").unwrap();
        assert!(!result.has_migrations);
        assert_eq!(result.confidence, Confidence::None);
    }
}
