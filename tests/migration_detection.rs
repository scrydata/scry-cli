//! Integration tests for migration detection.

use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn setup_git_repo() -> TempDir {
    let dir = TempDir::new().unwrap();

    Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Create initial commit
    fs::write(dir.path().join("README.md"), "# Test").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Tag for base reference
    Command::new("git")
        .args(["tag", "base"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    dir
}

#[test]
fn test_detect_migration_in_migrations_dir() {
    let dir = setup_git_repo();

    // Add a migration file
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    fs::write(
        dir.path().join("migrations/001_create_users.sql"),
        "CREATE TABLE users (id SERIAL PRIMARY KEY);",
    )
    .unwrap();

    Command::new("git")
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Add migration"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Run detect-migrations
    let output = Command::new(env!("CARGO_BIN_EXE_scry"))
        .args(["ci", "detect-migrations", "--base", "base", "--json"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "Command failed: {}", stdout);

    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(result["has_migrations"], true);
    assert!(result["confidence"].as_str().unwrap() == "medium" || result["confidence"].as_str().unwrap() == "high");
}

#[test]
fn test_detect_no_migrations() {
    let dir = setup_git_repo();

    // Add a non-migration file
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();

    Command::new("git")
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Add code"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Run detect-migrations
    let output = Command::new(env!("CARGO_BIN_EXE_scry"))
        .args(["ci", "detect-migrations", "--base", "base", "--json"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "Command failed: {}", stdout);

    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(result["has_migrations"], false);
}

#[test]
fn test_detect_ddl_in_arbitrary_file() {
    let dir = setup_git_repo();

    // Add a file with DDL keywords outside migration paths
    fs::write(
        dir.path().join("schema.sql"),
        "ALTER TABLE users ADD COLUMN email TEXT;",
    )
    .unwrap();

    Command::new("git")
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Add schema"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Run detect-migrations
    let output = Command::new(env!("CARGO_BIN_EXE_scry"))
        .args(["ci", "detect-migrations", "--base", "base", "--json"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "Command failed: {}", stdout);

    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(result["has_migrations"], true);
    assert_eq!(result["confidence"], "low"); // DDL but not in migration path
}

#[test]
fn test_scry_toml_config() {
    let dir = setup_git_repo();

    // Create custom config
    fs::write(
        dir.path().join(".scry.toml"),
        r#"
[migration_detection]
paths = ["custom/migrations/"]
extensions = [".sql"]
scan_ddl = false
"#,
    )
    .unwrap();

    // Add migration in custom path
    fs::create_dir_all(dir.path().join("custom/migrations")).unwrap();
    fs::write(
        dir.path().join("custom/migrations/001.sql"),
        "CREATE TABLE test (id INT);",
    )
    .unwrap();

    // Add migration in default path (should be ignored with custom config)
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    fs::write(
        dir.path().join("migrations/001.sql"),
        "CREATE TABLE ignored (id INT);",
    )
    .unwrap();

    Command::new("git")
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Add migrations"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Run detect-migrations
    let output = Command::new(env!("CARGO_BIN_EXE_scry"))
        .args(["ci", "detect-migrations", "--base", "base", "--json"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "Command failed: {}", stdout);

    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(result["has_migrations"], true);

    // Check that only custom/migrations is detected
    let evidence = result["evidence"].as_array().unwrap();
    assert_eq!(evidence.len(), 1);
    assert!(evidence[0]["file"].as_str().unwrap().contains("custom/migrations"));
}
