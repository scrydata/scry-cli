//! Integration tests for CI workflow commands.
//!
//! These tests verify CLI argument parsing and help text.
//! Tests that require a running server are marked with #[ignore].

use std::process::Command;

fn scry_cli() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_scry"));
    cmd.env("SCRY_API_URL", "http://localhost:8081");
    cmd.env("SCRY_API_TOKEN", "test-token");
    cmd
}

#[test]
fn test_ci_help() {
    let output = scry_cli()
        .args(["ci", "--help"])
        .output()
        .expect("failed to run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("wait-ready"));
    assert!(stdout.contains("checkpoint"));
    assert!(stdout.contains("connect"));
    assert!(stdout.contains("replay"));
    assert!(stdout.contains("reset"));
}

#[test]
fn test_ci_wait_ready_help() {
    let output = scry_cli()
        .args(["ci", "wait-ready", "--help"])
        .output()
        .expect("failed to run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--timeout"));
    assert!(stdout.contains("10m")); // default value
}

#[test]
fn test_ci_replay_help() {
    let output = scry_cli()
        .args(["ci", "replay", "--help"])
        .output()
        .expect("failed to run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--wait"));
    assert!(stdout.contains("--fail-on-regression"));
    assert!(stdout.contains("--last"));
    assert!(stdout.contains("--since-checkpoint"));
}

#[test]
fn test_ci_reset_help() {
    let output = scry_cli()
        .args(["ci", "reset", "--help"])
        .output()
        .expect("failed to run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--to"));
    assert!(stdout.contains("--wait"));
    assert!(stdout.contains("--dry-run"));
    assert!(stdout.contains("--max-restore-time"));
    assert!(stdout.contains("--force"));
}

#[test]
fn test_checkpoints_help() {
    let output = scry_cli()
        .args(["checkpoints", "--help"])
        .output()
        .expect("failed to run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("list"));
    assert!(stdout.contains("create"));
    assert!(stdout.contains("delete"));
}

#[test]
fn test_shadows_list_help() {
    let output = scry_cli()
        .args(["shadow", "list", "--help"])
        .output()
        .expect("failed to run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--source"));
    assert!(stdout.contains("--state"));
}

// Integration tests requiring a running server

#[test]
#[ignore] // Requires running server
fn test_source_list() {
    let output = scry_cli()
        .args(["source", "list"])
        .output()
        .expect("failed to run command");

    // Should succeed (even if empty)
    assert!(
        output.status.success(),
        "stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore] // Requires running server
fn test_ci_connect_invalid_shadow() {
    let output = scry_cli()
        .args(["ci", "connect", "nonexistent/shadow"])
        .output()
        .expect("failed to run command");

    // Should fail with not found
    assert!(!output.status.success());
    // Exit code 3 = NotFound
    assert_eq!(output.status.code(), Some(3));
}

#[test]
#[ignore] // Requires running server
fn test_checkpoints_list_invalid_shadow() {
    let output = scry_cli()
        .args(["checkpoints", "list", "nonexistent/shadow"])
        .output()
        .expect("failed to run command");

    // Should fail with not found
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(3));
}
