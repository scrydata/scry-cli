//! Phase-gate readiness checks for the demo walkthrough.
//!
//! Each gate verifies a precondition before proceeding to the next step,
//! replacing implicit sleep-based assumptions with explicit verification.

use super::client::DemoClient;
use super::compose::Runtime;
use super::error::DemoError;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// Verify the platform API is fully initialized (not just health-check passing).
///
/// Container healthchecks hit `/health` which passes as soon as the HTTP server
/// starts. The `/api/v1/ready` endpoint additionally verifies NATS connectivity
/// and internal initialization.
pub async fn api_ready(client: &DemoClient, timeout_secs: u64) -> Result<(), DemoError> {
    client.wait_for_ready(timeout_secs).await
}

/// Verify a container is running (not exited or dead).
///
/// If the container has exited, collects the last 20 lines of logs and returns
/// them in the error for diagnostics.
pub async fn container_running(runtime: Runtime, container: &str) -> Result<(), DemoError> {
    let output = Command::new(runtime.container_cmd())
        .args(["inspect", "--format", "{{.State.Status}}", container])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await;

    let status_str = match output {
        Ok(ref o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        }
        _ => {
            return Err(DemoError::ServiceUnhealthy {
                service: container.to_string(),
                detail: "Container not found (docker inspect failed)".to_string(),
            });
        }
    };

    match status_str.as_str() {
        "running" => Ok(()),
        "exited" | "dead" => {
            let logs = collect_container_logs(runtime, container, 20).await;
            Err(DemoError::ServiceUnhealthy {
                service: container.to_string(),
                detail: format!(
                    "Container status: {status_str}\nLast logs:\n{logs}"
                ),
            })
        }
        other => Err(DemoError::ServiceUnhealthy {
            service: container.to_string(),
            detail: format!("Unexpected container status: {other}"),
        }),
    }
}

/// Verify the proxy port (5434) is accepting TCP connections.
pub async fn proxy_connectable(timeout_secs: u64) -> Result<(), DemoError> {
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    loop {
        if start.elapsed() >= timeout {
            return Err(DemoError::Timeout {
                what: "proxy TCP connection on port 5434".to_string(),
                elapsed_secs: timeout_secs,
            });
        }

        if tokio::net::TcpStream::connect(("127.0.0.1", 5434_u16))
            .await
            .is_ok()
        {
            return Ok(());
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Verify the Docker socket is accessible.
///
/// scry-platform needs Docker socket access to create shadow containers.
/// In container mode, the compose override mounts the socket. In dev mode,
/// the local binary accesses it directly.
pub async fn docker_socket_accessible(runtime: Runtime) -> Result<(), DemoError> {
    let output = Command::new(runtime.container_cmd())
        .args(["info", "--format", "{{.ServerVersion}}"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() => Ok(()),
        _ => Err(DemoError::PreconditionFailed {
            gate: "docker_socket".to_string(),
            detail: format!(
                "{} daemon not accessible. Ensure {} is running.",
                runtime.container_cmd(),
                runtime.container_cmd(),
            ),
        }),
    }
}

/// Collect the last N lines of logs from a container.
async fn collect_container_logs(runtime: Runtime, container: &str, lines: u32) -> String {
    let output = Command::new(runtime.container_cmd())
        .args(["logs", "--tail", &lines.to_string(), container])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await;

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            if !stdout.is_empty() {
                stdout.to_string()
            } else {
                stderr.to_string()
            }
        }
        Err(e) => format!("(could not collect logs: {e})"),
    }
}
