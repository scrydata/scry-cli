//! Embedded demo assets and extraction.

use std::fs;
use std::path::Path;

// --- Embedded assets via include_str! ---

const DOCKER_COMPOSE: &str = include_str!("../../../assets/demo/docker-compose.yml");
const SCHEMA_SQL: &str = include_str!("../../../assets/demo/sample-data/01-schema.sql");
const SEED_SQL: &str = include_str!("../../../assets/demo/sample-data/02-seed-data.sql");

// Scenario: missing-index
const MISSING_INDEX_MIGRATION: &str =
    include_str!("../../../assets/demo/scenarios/missing-index/migration.sql");
const MISSING_INDEX_WORKLOAD: &str =
    include_str!("../../../assets/demo/scenarios/missing-index/workload.sql");
const MISSING_INDEX_FIX: &str =
    include_str!("../../../assets/demo/scenarios/missing-index/fix.sql");

// Scenario: table-locking
const TABLE_LOCKING_MIGRATION: &str =
    include_str!("../../../assets/demo/scenarios/table-locking/migration.sql");
const TABLE_LOCKING_WORKLOAD: &str =
    include_str!("../../../assets/demo/scenarios/table-locking/workload.sql");
const TABLE_LOCKING_FIX: &str =
    include_str!("../../../assets/demo/scenarios/table-locking/fix.sql");

// Scenario: index-drop
const INDEX_DROP_MIGRATION: &str =
    include_str!("../../../assets/demo/scenarios/index-drop/migration.sql");
const INDEX_DROP_WORKLOAD: &str =
    include_str!("../../../assets/demo/scenarios/index-drop/workload.sql");
const INDEX_DROP_FIX: &str =
    include_str!("../../../assets/demo/scenarios/index-drop/fix.sql");

/// Metadata about a demo scenario.
pub struct ScenarioInfo {
    pub name: String,
    pub description: String,
}

/// List all available scenarios.
pub fn list_scenarios() -> Vec<ScenarioInfo> {
    vec![
        ScenarioInfo {
            name: "missing-index".to_string(),
            description: "Query regresses due to missing index after adding a column".to_string(),
        },
        ScenarioInfo {
            name: "table-locking".to_string(),
            description: "Large UPDATE locks table, blocking production queries".to_string(),
        },
        ScenarioInfo {
            name: "index-drop".to_string(),
            description: "Dropped 'unused' index causes widespread regression".to_string(),
        },
    ]
}

/// Check if a scenario exists.
pub fn scenario_exists(name: &str) -> bool {
    matches!(name, "missing-index" | "table-locking" | "index-drop")
}

/// SQL files for a scenario.
pub struct ScenarioFiles {
    pub migration: &'static str,
    pub workload: &'static str,
    pub fix: &'static str,
}

/// Get the embedded SQL content for a scenario.
pub fn scenario_sql(name: &str) -> Option<ScenarioFiles> {
    match name {
        "missing-index" => Some(ScenarioFiles {
            migration: MISSING_INDEX_MIGRATION,
            workload: MISSING_INDEX_WORKLOAD,
            fix: MISSING_INDEX_FIX,
        }),
        "table-locking" => Some(ScenarioFiles {
            migration: TABLE_LOCKING_MIGRATION,
            workload: TABLE_LOCKING_WORKLOAD,
            fix: TABLE_LOCKING_FIX,
        }),
        "index-drop" => Some(ScenarioFiles {
            migration: INDEX_DROP_MIGRATION,
            workload: INDEX_DROP_WORKLOAD,
            fix: INDEX_DROP_FIX,
        }),
        _ => None,
    }
}

/// Get the embedded docker-compose.yml content.
#[allow(dead_code)]
pub fn docker_compose_yml() -> &'static str {
    DOCKER_COMPOSE
}

/// Extract all demo assets to the given directory.
pub fn extract_to(dir: &Path) -> Result<(), anyhow::Error> {
    // Create directory structure
    fs::create_dir_all(dir.join("sample-data"))?;
    fs::create_dir_all(dir.join("scenarios/missing-index"))?;
    fs::create_dir_all(dir.join("scenarios/table-locking"))?;
    fs::create_dir_all(dir.join("scenarios/index-drop"))?;

    // Write files
    fs::write(dir.join("docker-compose.yml"), DOCKER_COMPOSE)?;
    fs::write(dir.join("sample-data/01-schema.sql"), SCHEMA_SQL)?;
    fs::write(dir.join("sample-data/02-seed-data.sql"), SEED_SQL)?;

    fs::write(
        dir.join("scenarios/missing-index/migration.sql"),
        MISSING_INDEX_MIGRATION,
    )?;
    fs::write(
        dir.join("scenarios/missing-index/workload.sql"),
        MISSING_INDEX_WORKLOAD,
    )?;
    fs::write(
        dir.join("scenarios/missing-index/fix.sql"),
        MISSING_INDEX_FIX,
    )?;

    fs::write(
        dir.join("scenarios/table-locking/migration.sql"),
        TABLE_LOCKING_MIGRATION,
    )?;
    fs::write(
        dir.join("scenarios/table-locking/workload.sql"),
        TABLE_LOCKING_WORKLOAD,
    )?;
    fs::write(
        dir.join("scenarios/table-locking/fix.sql"),
        TABLE_LOCKING_FIX,
    )?;

    fs::write(
        dir.join("scenarios/index-drop/migration.sql"),
        INDEX_DROP_MIGRATION,
    )?;
    fs::write(
        dir.join("scenarios/index-drop/workload.sql"),
        INDEX_DROP_WORKLOAD,
    )?;
    fs::write(
        dir.join("scenarios/index-drop/fix.sql"),
        INDEX_DROP_FIX,
    )?;

    Ok(())
}
