//! Docker/Podman Compose lifecycle management for the demo.

use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use super::terminal;

/// Detected container runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    Docker,
    Podman,
}

impl Runtime {
    pub fn container_cmd(&self) -> &'static str {
        match self {
            Runtime::Docker => "docker",
            Runtime::Podman => "podman",
        }
    }
}

/// Detect whether docker or podman is available.
pub async fn detect_runtime() -> Result<Runtime, anyhow::Error> {
    // Prefer docker if available
    if command_exists("docker").await {
        return Ok(Runtime::Docker);
    }
    if command_exists("podman").await {
        return Ok(Runtime::Podman);
    }
    Err(anyhow::anyhow!(
        "Neither 'docker' nor 'podman' found in PATH. Install one to run the demo."
    ))
}

/// Check that the Docker daemon is actually running (not just installed).
///
/// `docker version` succeeds even without a daemon; `docker info` requires one.
/// Short-circuits for Podman (which uses a socket-activated service).
pub async fn check_docker_daemon(runtime: Runtime) -> Result<(), anyhow::Error> {
    if runtime != Runtime::Docker {
        return Ok(());
    }

    let output = Command::new("docker")
        .arg("info")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() => Ok(()),
        _ => {
            let hint = if cfg!(target_os = "macos") {
                "Try: open -a Docker"
            } else {
                "Try: sudo systemctl start docker"
            };
            Err(anyhow::anyhow!(
                "Docker is installed but the daemon is not running.\n  {hint}"
            ))
        }
    }
}

async fn command_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Build the compose command parts for the detected runtime.
fn compose_args(runtime: Runtime) -> Vec<String> {
    match runtime {
        Runtime::Docker => vec!["docker".to_string(), "compose".to_string()],
        Runtime::Podman => vec!["podman".to_string(), "compose".to_string()],
    }
}

/// Generate docker-compose.override.yml for the detected runtime.
///
/// Docker needs the host socket mounted; Podman needs the user socket +
/// SELinux label=disable.
///
/// If `dev_mode` is true, disables the scry-platform container so we can run locally.
pub async fn generate_override(dir: &Path, dev_mode: bool, scenario: Option<&str>) -> Result<(), anyhow::Error> {
    let runtime = detect_runtime().await?;
    let override_path = dir.join("docker-compose.override.yml");

    let content = if dev_mode {
        // Dev mode: reconfigure backfill and proxy to connect to host.
        // compose::up() only starts postgres and nats; backfill and proxy
        // are started later via restart_service after platform is healthy.
        // Override depends_on to remove scry-platform dependency.
        let hostname = match runtime {
            Runtime::Docker => "host.docker.internal",
            Runtime::Podman => "host.containers.internal",
        };
        format!(
            "# Auto-generated for dev mode - do not edit\n\
             services:\n\
             \x20 scry-backfill:\n\
             \x20   depends_on:\n\
             \x20     source-postgres:\n\
             \x20       condition: service_healthy\n\
             \x20   extra_hosts:\n\
             \x20     - {hostname}:host-gateway\n\
             \x20   environment:\n\
             \x20     SCRY_BACKFILL_PRODUCER__ENDPOINT: http://{hostname}:8080/events/cdc/binary\n\
             \x20 scry-proxy:\n\
             \x20   depends_on:\n\
             \x20     source-postgres:\n\
             \x20       condition: service_healthy\n\
             \x20   extra_hosts:\n\
             \x20     - {hostname}:host-gateway\n\
             \x20   environment:\n\
             \x20     SCRY_PUBLISHER__HTTP_ENDPOINT: http://{hostname}:8080/events/queries/binary\n"
        )
    } else {
        match runtime {
            Runtime::Docker => {
                "# Auto-generated for Docker runtime - do not edit\n\
                 services:\n\
                 \x20 scry-platform:\n\
                 \x20   volumes:\n\
                 \x20     - /var/run/docker.sock:/var/run/docker.sock\n"
                    .to_string()
            }
            Runtime::Podman => {
                // Rootless podman socket mounts don't work across namespaces.
                // Use the TCP API instead (started by maybe_start_podman_tcp).
                // Both Docker and Podman paths use GHCR images (no local binary
                // mounting — avoids glibc mismatch between host and container OS).
                // Use `--dev` flag to run the platform binary locally instead.
                "# Auto-generated for Podman runtime - do not edit\n\
                 services:\n\
                 \x20 scry-platform:\n\
                 \x20   security_opt:\n\
                 \x20     - label=disable\n\
                 \x20   extra_hosts:\n\
                 \x20     - host.containers.internal:host-gateway\n\
                 \x20   environment:\n\
                 \x20     DOCKER_HOST: tcp://host.containers.internal:2375\n"
                    .to_string()
            }
        }
    };

    // Append scenario env var for query-generator if specified
    let content = if let Some(scenario) = scenario {
        format!(
            "{content}\
             \x20 query-generator:\n\
             \x20   environment:\n\
             \x20     SCENARIO: {scenario}\n"
        )
    } else {
        content
    };

    std::fs::write(&override_path, content)?;
    Ok(())
}

/// Start scry-platform locally for dev mode.
pub async fn start_local_platform() -> Result<tokio::process::Child, anyhow::Error> {
    let binary = find_local_binary()
        .ok_or_else(|| anyhow::anyhow!("No local scry-platform binary found. Run 'cargo build --release' first."))?;

    let runtime = detect_runtime().await?;

    let mut cmd = Command::new(&binary);
    cmd.arg("serve")
        .env("RUST_LOG", "info,scry_platform=debug")
        .env("SCRY_PLATFORM_RECEIVER__LISTEN_ADDRESS", "0.0.0.0:8080")
        .env("SCRY_PLATFORM_RECEIVER__AUTH_TOKEN", "demo-token")
        .env("SCRY_PLATFORM_API__LISTEN_ADDRESS", "0.0.0.0:8081")
        .env("SCRY_PLATFORM_API__AUTH_TOKEN", "demo-token")
        .env("SCRY_PLATFORM_API__ENABLE_SWAGGER_UI", "true")
        .env("SCRY_PLATFORM_QUEUE__BACKEND", "nats")
        .env("SCRY_PLATFORM_QUEUE__NATS__URL", "nats://localhost:4222")
        .env("SCRY_PLATFORM_STORAGE__PATH", "./data/scry-platform.db")
        .env("SCRY_PLATFORM_SHADOW__PROVIDER", "docker")
        .env("SCRY_PLATFORM_SHADOW__NETWORK", "scry-demo-net")
        .env("SCRY_PLATFORM_SHADOW__PROVISIONING_TIMEOUT_SECS", "300")
        .env("SCRY_PLATFORM_SHADOW__DEFAULT_DISK_GB", "0")
        // Platform runs on the host, so connect to shadow containers via
        // 127.0.0.1 and the mapped host port instead of Docker DNS.
        .env("SCRY_PLATFORM_SHADOW__CONNECT_VIA_HOST", "true");

    // Log to /tmp so it survives demo cleanup
    let log_path = "/tmp/scry-platform-demo.log";
    // /dev/null always exists on Unix
    #[allow(clippy::expect_used)]
    let log_file = std::fs::File::create(log_path)
        .unwrap_or_else(|_| std::fs::File::create("/dev/null").expect("/dev/null"));
    #[allow(clippy::expect_used)]
    let log_err = log_file.try_clone().unwrap_or_else(|_| {
        std::fs::File::create("/dev/null").expect("/dev/null")
    });
    eprintln!("  Platform log: {log_path}");
    eprintln!("  Platform binary: {binary}");
    cmd.stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_err));

    // For Podman, point the Docker client at the TCP API (started by
    // maybe_start_podman_tcp). Without this, bollard can't find the socket.
    if runtime == Runtime::Podman {
        cmd.env("DOCKER_HOST", "tcp://localhost:2375");
    }

    let child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to start local scry-platform: {}", e))?;

    Ok(child)
}

/// Find local scry-platform binary if available.
fn find_local_binary() -> Option<String> {
    // Check common locations relative to current directory
    let candidates = [
        "target/release/scry-platform",
        "../scry-platform/target/release/scry-platform",
    ];

    for candidate in candidates {
        let path = std::path::Path::new(candidate);
        if path.exists() {
            return path.canonicalize().ok().map(|p| p.display().to_string());
        }
    }
    None
}

#[allow(dead_code)]
async fn podman_socket_path() -> String {
    // Try `podman info` first
    if let Ok(output) = Command::new("podman")
        .args(["info", "--format", "{{.Host.RemoteSocket.Path}}"])
        .output()
        .await
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return path;
        }
    }

    // Fallback to XDG_RUNTIME_DIR
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return format!("{xdg}/podman/podman.sock");
    }

    // Last resort
    let uid = users::get_current_uid();
    format!("/run/user/{uid}/podman/podman.sock")
}

/// Start Podman TCP API if needed for rootless Podman.
/// Returns the PID if started.
pub async fn maybe_start_podman_tcp(runtime: Runtime) -> Option<u32> {
    if runtime != Runtime::Podman {
        return None;
    }
    if std::env::var("DOCKER_HOST").is_ok() {
        return None;
    }

    // Check if TCP API is already running
    let already_running = reqwest::Client::new()
        .get("http://localhost:2375/version")
        .send()
        .await
        .is_ok();

    if already_running {
        return None;
    }

    let child = Command::new("podman")
        .args(["system", "service", "tcp:0.0.0.0:2375", "--time=0"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    match child {
        Ok(child) => {
            let pid = child.id();
            // Poll until the TCP API is ready (or timeout after 5s)
            for _ in 0..50 {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let ready = reqwest::Client::new()
                    .get("http://localhost:2375/version")
                    .send()
                    .await
                    .is_ok();
                if ready {
                    return pid;
                }
            }
            // Timed out but still return the PID
            pid
        }
        Err(_) => None,
    }
}

/// Kill the Podman TCP API process.
pub fn stop_podman_tcp(pid: u32) {
    // Use kill command instead of libc to avoid unsafe
    let _ = std::process::Command::new("kill")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Run `compose up -d` in the given directory.
/// If `dev_mode` is true, only starts postgres and nats (platform runs locally,
/// backfill and proxy are started later after platform is healthy).
pub async fn up(dir: &Path, dev_mode: bool) -> Result<(), anyhow::Error> {
    let runtime = detect_runtime().await?;
    let args = compose_args(runtime);

    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..])
        .arg("up")
        .arg("-d")
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if dev_mode {
        // Only start postgres and nats. Platform runs locally; backfill and
        // proxy are started later (via restart_service) once the platform is
        // healthy. Using explicit service names instead of --scale because
        // podman-compose silently ignores --scale flags.
        cmd.args(["source-postgres", "nats"]);
    }

    let status = cmd.status().await?;

    if !status.success() {
        let hint = match runtime {
            Runtime::Docker => "Is the Docker daemon running? Try: sudo systemctl start docker",
            Runtime::Podman => "Is Podman running? Try: systemctl --user start podman",
        };
        return Err(anyhow::anyhow!(
            "{} compose up failed (exit {}). {hint}",
            runtime.container_cmd(),
            status.code().unwrap_or(-1),
        ));
    }
    Ok(())
}

/// Restart a single compose service, bypassing dependency checks.
///
/// Uses `--no-deps` because podman-compose merges `depends_on` from base and
/// override files (unlike docker compose which replaces). Without `--no-deps`,
/// the command would hang waiting for dependencies that are scaled to 0.
pub async fn restart_service(dir: &Path, service: &str) -> Result<(), anyhow::Error> {
    let runtime = detect_runtime().await?;
    let args = compose_args(runtime);

    let status = Command::new(&args[0])
        .args(&args[1..])
        .args(["up", "-d", "--no-deps", "--force-recreate", service])
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await?;

    if !status.success() {
        return Err(anyhow::anyhow!(
            "Failed to restart service '{service}' (exit {})",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

/// Run `compose down` (optionally with `-v` to remove volumes).
pub async fn down(dir: &Path, remove_volumes: bool) -> Result<(), anyhow::Error> {
    let runtime = detect_runtime().await?;
    let args = compose_args(runtime);

    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..]).arg("down").current_dir(dir);
    if remove_volumes {
        cmd.arg("-v");
    }

    let status = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await?;

    if !status.success() {
        // Non-fatal — might already be stopped
        terminal::print_info("compose down returned non-zero (services may already be stopped)");
    }

    if remove_volumes {
        // Kill any locally-running scry-platform process (dev mode).
        kill_local_platform().await;

        // Remove shadow containers created by the platform (not managed by compose).
        remove_shadow_containers(runtime).await;

        // Remove the SQLite state file so the next run starts fresh.
        remove_storage_db();
    }

    Ok(())
}

/// Kill any locally-running scry-platform process started in dev mode.
async fn kill_local_platform() {
    let output = Command::new("pgrep")
        .args(["-f", "scry-platform serve"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await;

    let pids: Vec<String> = match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        }
        _ => return,
    };

    if pids.is_empty() {
        return;
    }

    let mut args = vec!["--signal".to_string(), "TERM".to_string()];
    args.extend(pids.clone());

    let _ = Command::new("kill")
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;

    terminal::print_info(&format!(
        "Stopped {} local scry-platform process(es)",
        pids.len()
    ));
}

/// Remove Docker containers whose names start with `scry-shadow-`.
/// These are provisioned by the platform at runtime, not by compose.
async fn remove_shadow_containers(runtime: Runtime) {
    let output = Command::new(runtime.container_cmd())
        .args(["ps", "-aq", "--filter", "name=scry-shadow-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await;

    let ids: Vec<String> = match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        }
        _ => return,
    };

    if ids.is_empty() {
        return;
    }

    let mut args = vec!["rm".to_string(), "-f".to_string()];
    args.extend(ids.iter().cloned());

    let _ = Command::new(runtime.container_cmd())
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;

    terminal::print_info(&format!(
        "Removed {} shadow container(s)",
        ids.len()
    ));
}

/// Remove the SQLite database file used by scry-platform.
fn remove_storage_db() {
    let db_path = std::path::Path::new("./data/scry-platform.db");
    for suffix in &["", "-wal", "-shm"] {
        let path = if suffix.is_empty() {
            db_path.to_path_buf()
        } else {
            db_path.with_extension(format!("db{suffix}"))
        };
        let _ = std::fs::remove_file(&path);
    }
}

/// Show service status by checking health endpoints.
pub async fn status(dir: &Path) -> Result<(), anyhow::Error> {
    let _ = dir; // We check endpoints regardless of directory
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?;

    terminal::print_header("Demo Service Status");
    println!();

    // Check each service
    let pg_healthy = check_postgres_health().await;
    terminal::print_service_status("PostgreSQL (source:5432)", pg_healthy);

    let nats_healthy = client
        .get("http://localhost:8222/healthz")
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    terminal::print_service_status("NATS JetStream (:4222)", nats_healthy);

    let platform_healthy = client
        .get("http://localhost:8080/health")
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    terminal::print_service_status("Scry Platform (:8080/8081)", platform_healthy);

    let proxy_healthy = client
        .get("http://localhost:9091/metrics")
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    terminal::print_service_status("Scry Proxy (:5434)", proxy_healthy);

    println!();
    Ok(())
}

/// Check PostgreSQL via `docker exec` pg_isready.
async fn check_postgres_health() -> bool {
    let runtime = detect_runtime().await.ok();
    let cmd_name = runtime
        .map(|r| r.container_cmd())
        .unwrap_or("docker");

    Command::new(cmd_name)
        .args(["exec", "scry-demo-source", "pg_isready", "-U", "postgres"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Wait for PostgreSQL to be ready via `docker exec` pg_isready.
#[allow(dead_code)]
pub async fn wait_for_postgres(
    runtime: Runtime,
    container: &str,
    max_attempts: u32,
) -> bool {
    let cmd = runtime.container_cmd();
    for _ in 0..max_attempts {
        let ok = Command::new(cmd)
            .args(["exec", container, "pg_isready", "-U", "postgres"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);

        if ok {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    false
}

/// Wait for an HTTP health endpoint to return success.
pub async fn wait_for_http(url: &str, max_attempts: u32) -> bool {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    for _ in 0..max_attempts {
        let ok = client
            .get(url)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);

        if ok {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    false
}

/// Check if a TCP port is listening.
#[allow(dead_code)]
pub async fn port_is_open(port: u16) -> bool {
    tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .is_ok()
}

/// Container health/state as reported by `docker inspect`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ContainerStatus {
    /// Container not yet created or not found
    Pending,
    /// Has a healthcheck, waiting for it to pass
    Starting,
    /// Healthcheck passed
    Healthy,
    /// No healthcheck, just running
    Running,
    /// Healthcheck is failing
    Unhealthy,
    /// Container exited
    Exited(i32),
    /// Container is dead
    Dead,
}

impl ContainerStatus {
    fn is_ready(&self, has_healthcheck: bool) -> bool {
        match self {
            ContainerStatus::Healthy => true,
            ContainerStatus::Running => !has_healthcheck,
            _ => false,
        }
    }

    fn is_terminal_failure(&self) -> bool {
        matches!(self, ContainerStatus::Exited(_) | ContainerStatus::Dead)
    }
}

impl std::fmt::Display for ContainerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerStatus::Pending => write!(f, "pending"),
            ContainerStatus::Starting => write!(f, "starting"),
            ContainerStatus::Healthy => write!(f, "healthy"),
            ContainerStatus::Running => write!(f, "running"),
            ContainerStatus::Unhealthy => write!(f, "unhealthy"),
            ContainerStatus::Exited(code) => write!(f, "exited (code {code})"),
            ContainerStatus::Dead => write!(f, "dead"),
        }
    }
}

struct ServiceDef {
    name: &'static str,
    container: &'static str,
    has_healthcheck: bool,
}

/// Services in `demo/docker-compose.yaml` (used by `just demo-up`).
const DEMO_INTEGRATION_SERVICES: &[ServiceDef] = &[
    ServiceDef { name: "source-postgres", container: "scry-source-pg", has_healthcheck: true },
    ServiceDef { name: "nats", container: "scry-nats", has_healthcheck: true },
    ServiceDef { name: "scry-platform", container: "scry-platform", has_healthcheck: true },
    ServiceDef { name: "scry-backfill", container: "scry-backfill", has_healthcheck: false },
    ServiceDef { name: "scry-proxy", container: "scry-proxy", has_healthcheck: true },
    ServiceDef { name: "web-ui", container: "scry-web-ui", has_healthcheck: false },
    ServiceDef { name: "query-generator", container: "scry-query-generator", has_healthcheck: false },
];

/// Services in `scry-demo/docker-compose.yml` (used by `scry demo start` walkthrough).
const DEMO_WALKTHROUGH_SERVICES: &[ServiceDef] = &[
    ServiceDef { name: "source-postgres", container: "scry-demo-source", has_healthcheck: true },
    ServiceDef { name: "nats", container: "scry-demo-nats", has_healthcheck: true },
    ServiceDef { name: "scry-platform", container: "scry-demo-platform", has_healthcheck: true },
    ServiceDef { name: "scry-backfill", container: "scry-demo-backfill", has_healthcheck: false },
    ServiceDef { name: "scry-proxy", container: "scry-demo-proxy", has_healthcheck: true },
    ServiceDef { name: "query-generator", container: "scry-demo-query-gen", has_healthcheck: false },
];

/// Query a container's health/state via `docker inspect`.
async fn get_container_health(runtime: Runtime, container: &str) -> ContainerStatus {
    // Get both the container state and healthcheck status in one call.
    // We need to check the container state FIRST because a container can be
    // exited/dead but still have a stale healthcheck status of "starting".
    let format_str = "{{.State.Status}}|{{if .State.Health}}{{.State.Health.Status}}{{end}}";
    let output = Command::new(runtime.container_cmd())
        .args(["inspect", "--format", format_str, container])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await;

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return ContainerStatus::Pending,
    };

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let parts: Vec<&str> = raw.splitn(2, '|').collect();
    let state = parts.first().copied().unwrap_or("");
    let health = parts.get(1).copied().unwrap_or("");

    // Check container state first — exited/dead/created override healthcheck
    match state {
        "exited" => {
            let code = get_exit_code(runtime, container).await;
            return ContainerStatus::Exited(code);
        }
        "dead" => return ContainerStatus::Dead,
        "created" => return ContainerStatus::Pending,
        _ => {}
    }

    // Container is running — check healthcheck status if present
    match health {
        "healthy" => ContainerStatus::Healthy,
        "starting" => ContainerStatus::Starting,
        "unhealthy" => ContainerStatus::Unhealthy,
        "" => {
            // No healthcheck — "running" is the final state
            if state == "running" {
                ContainerStatus::Running
            } else {
                ContainerStatus::Pending
            }
        }
        _ => ContainerStatus::Pending,
    }
}

/// Get exit code from a stopped container.
async fn get_exit_code(runtime: Runtime, container: &str) -> i32 {
    let output = Command::new(runtime.container_cmd())
        .args(["inspect", "--format", "{{.State.ExitCode}}", container])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse()
                .unwrap_or(-1)
        }
        _ => -1,
    }
}

/// Get the last healthcheck log from a container (for debugging failures).
async fn get_healthcheck_log(runtime: Runtime, container: &str) -> Option<String> {
    let output = Command::new(runtime.container_cmd())
        .args([
            "inspect",
            "--format",
            "{{if .State.Health}}{{with (index .State.Health.Log (len .State.Health.Log | printf \"%d\" | call (index . \"sub\")))}}{{.Output}}{{end}}{{end}}",
            container,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await;

    // Fallback: just get the full health log as JSON
    let output = match output {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
            // Try simpler format
            Command::new(runtime.container_cmd())
                .args([
                    "inspect",
                    "--format",
                    "{{if .State.Health}}{{range .State.Health.Log}}{{.Output}}{{end}}{{end}}",
                    container,
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output()
                .await
        }
        _ => {
            Command::new(runtime.container_cmd())
                .args([
                    "inspect",
                    "--format",
                    "{{if .State.Health}}{{range .State.Health.Log}}{{.Output}}{{end}}{{end}}",
                    container,
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output()
                .await
        }
    };

    match output {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() { None } else { Some(s) }
        }
        _ => None,
    }
}

/// Wait for all demo services to become healthy/running.
///
/// Standalone entry point for `scry demo wait-services` CLI command.
/// Prints header and demo URLs on success.
pub async fn wait_for_services(timeout_secs: u64) -> Result<(), anyhow::Error> {
    println!();
    println!(
        "  {} Waiting for demo services (timeout: {timeout_secs}s)...",
        terminal::bold(">>")
    );
    println!();

    poll_service_readiness(timeout_secs, DEMO_INTEGRATION_SERVICES, false).await?;

    println!();
    print_demo_urls();
    Ok(())
}

/// Wait for demo services from the walkthrough context.
///
/// Skips header/URL output. In dev mode, skips the scry-platform container
/// (which runs locally rather than in Docker) and checks it via HTTP instead.
pub async fn wait_for_services_walkthrough(timeout_secs: u64, dev_mode: bool) -> Result<(), anyhow::Error> {
    poll_service_readiness(timeout_secs, DEMO_WALKTHROUGH_SERVICES, dev_mode).await
}

/// Core polling loop: checks container health via `docker inspect` every 2s.
///
/// When `dev_mode` is true, the scry-platform container is skipped (it runs
/// locally) and is checked via HTTP at `localhost:8080/health` instead.
async fn poll_service_readiness(timeout_secs: u64, all_services: &[ServiceDef], dev_mode: bool) -> Result<(), anyhow::Error> {
    let runtime = detect_runtime().await?;

    // Build the effective service list, skipping scry-platform container in dev mode
    let services: Vec<&ServiceDef> = all_services
        .iter()
        .filter(|s| !(dev_mode && s.container == "scry-demo-platform"))
        .collect();

    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(timeout_secs);
    let poll_interval = std::time::Duration::from_secs(2);

    let mut ready: Vec<bool> = vec![false; services.len()];
    let mut failed: Vec<Option<String>> = vec![None; services.len()];
    let mut spinner_frame: usize = 0;

    // In dev mode, track scry-platform HTTP readiness separately
    let mut platform_ready = !dev_mode; // already "done" if not dev mode

    loop {
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            break;
        }

        let mut all_ready = platform_ready;
        let mut any_failed = false;

        for (i, svc) in services.iter().enumerate() {
            if ready[i] || failed[i].is_some() {
                continue;
            }

            let status = get_container_health(runtime, svc.container).await;

            if status.is_ready(svc.has_healthcheck) {
                ready[i] = true;
                let elapsed_str = format!("[{:>2}s]", elapsed.as_secs());
                let status_label = if svc.has_healthcheck { "healthy" } else { "running" };
                terminal::clear_spinner();
                println!(
                    "  {:<24} {:<28} {}",
                    svc.name,
                    terminal::green(status_label),
                    terminal::dim(&elapsed_str),
                );
            } else if status.is_terminal_failure() {
                let reason = format!("{status}");
                terminal::clear_spinner();
                println!(
                    "  {:<24} {}",
                    svc.name,
                    terminal::red(&reason),
                );
                failed[i] = Some(reason);
                any_failed = true;
            } else {
                all_ready = false;
            }
        }

        // In dev mode, check scry-platform via HTTP
        if dev_mode && !platform_ready {
            let http_ok = check_http_health("http://localhost:8080/health").await;
            if http_ok {
                platform_ready = true;
                let elapsed_str = format!("[{:>2}s]", elapsed.as_secs());
                terminal::clear_spinner();
                println!(
                    "  {:<24} {:<28} {}",
                    "scry-platform (local)",
                    terminal::green("healthy"),
                    terminal::dim(&elapsed_str),
                );
            } else {
                all_ready = false;
            }
        }

        if all_ready && !any_failed {
            let total_secs = start.elapsed().as_secs();
            println!();
            terminal::print_success(&format!("All services ready! ({total_secs}s)"));
            return Ok(());
        }

        if any_failed {
            println!();
            terminal::print_error("Some services failed to start:");
            println!();
            for (i, svc) in services.iter().enumerate() {
                if let Some(reason) = &failed[i] {
                    println!(
                        "    {} {}: {}",
                        terminal::red("✗"),
                        svc.name,
                        reason,
                    );
                    if let Some(log) = get_healthcheck_log(runtime, svc.container).await {
                        let log_line = log.lines().last().unwrap_or(&log);
                        println!(
                            "      {}",
                            terminal::dim(&format!("Last healthcheck: {}", log_line.trim())),
                        );
                    }
                    println!(
                        "      {}",
                        terminal::dim(&format!("Try: docker compose -f ./scry-demo/docker-compose.yml logs {}", svc.name)),
                    );
                }
            }
            return Err(anyhow::anyhow!("Some demo services failed to start"));
        }

        // Show spinner for remaining services
        let mut pending: Vec<&str> = services
            .iter()
            .enumerate()
            .filter(|(i, _)| !ready[*i] && failed[*i].is_none())
            .map(|(_, s)| s.name)
            .collect();
        if dev_mode && !platform_ready {
            pending.push("scry-platform (local)");
        }
        terminal::print_spinner(
            spinner_frame,
            &format!("waiting for {}...", pending.join(", ")),
        );
        spinner_frame += 1;

        tokio::time::sleep(poll_interval).await;
    }

    // Timeout
    terminal::clear_spinner();
    println!();
    terminal::print_error(&format!("Timed out after {timeout_secs}s. Services not ready:"));
    println!();
    for (i, svc) in services.iter().enumerate() {
        if !ready[i] && failed[i].is_none() {
            let status = get_container_health(runtime, svc.container).await;
            let detail = if status == ContainerStatus::Pending {
                format!("{status} (container '{}' not found or not yet created)", svc.container)
            } else {
                format!("{status}")
            };
            println!(
                "    {} {}: {}",
                terminal::yellow("?"),
                svc.name,
                detail,
            );
            if let Some(log) = get_healthcheck_log(runtime, svc.container).await {
                let log_line = log.lines().last().unwrap_or(&log);
                println!(
                    "      {}",
                    terminal::dim(&format!("Last healthcheck: {}", log_line.trim())),
                );
            }
            println!(
                "      {}",
                terminal::dim(&format!("Try: docker compose -f ./scry-demo/docker-compose.yml logs {}", svc.name)),
            );
        }
    }
    if dev_mode && !platform_ready {
        println!(
            "    {} scry-platform (local): not responding at localhost:8080",
            terminal::yellow("?"),
        );
    }
    Err(anyhow::anyhow!("Timed out waiting for demo services"))
}

/// Quick HTTP health check (single attempt, 3s timeout).
async fn check_http_health(url: &str) -> bool {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    client
        .get(url)
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Print the demo service URLs.
fn print_demo_urls() {
    println!("  Demo is ready:");
    println!(
        "    Swagger UI:     {}",
        terminal::bold("http://localhost:8081/swagger-ui/")
    );
    println!(
        "    Web UI:         {}",
        terminal::bold("http://localhost:3000")
    );
    println!(
        "    Proxy (psql):   {}",
        terminal::bold("localhost:5434")
    );
    println!(
        "    Source DB:       {}",
        terminal::bold("localhost:5432")
    );
    println!(
        "    NATS Monitor:   {}",
        terminal::bold("http://localhost:8222")
    );
    println!();
    println!(
        "  Use '{}' to view logs.",
        terminal::dim("docker compose -f ./scry-demo/docker-compose.yml logs <service>")
    );
    println!();
}

/// Collect last N lines of logs from specified compose services.
pub async fn collect_service_logs(dir: &Path, services: &[&str], lines: u32) -> Vec<(String, String)> {
    let runtime = match detect_runtime().await {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let args = compose_args(runtime);
    let mut results = Vec::new();

    for service in services {
        let output = Command::new(&args[0])
            .args(&args[1..])
            .args(["logs", "--tail", &lines.to_string(), service])
            .current_dir(dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

        let logs = match output {
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
        };
        results.push((service.to_string(), logs));
    }

    results
}

/// Image build definitions for `scry demo build`.
struct ImageBuild {
    name: &'static str,
    tag: &'static str,
    context: &'static str,
    dockerfile: Option<&'static str>,
}

const DEMO_IMAGES: &[ImageBuild] = &[
    ImageBuild {
        name: "scry-platform-demo",
        tag: "ghcr.io/scrydata/scry-platform-demo:latest",
        context: ".",
        dockerfile: None,
    },
    ImageBuild {
        name: "scry-query-generator",
        tag: "ghcr.io/scrydata/scry-query-generator:latest",
        context: ".",
        dockerfile: Some("crates/scry-query-generator/Dockerfile"),
    },
];

/// Sibling repo images that can be built if the repos exist next to scry-platform.
const SIBLING_IMAGES: &[ImageBuild] = &[
    ImageBuild {
        name: "scry-backfill",
        tag: "ghcr.io/scrydata/scry-backfill:latest",
        context: "../scry-backfill",
        dockerfile: None,
    },
    ImageBuild {
        name: "scry-proxy",
        tag: "ghcr.io/scrydata/scry-proxy:latest",
        context: "../scry-proxy",
        dockerfile: None,
    },
];

/// Build demo container images locally.
pub async fn build_images(quiet: bool) -> Result<(), anyhow::Error> {
    let runtime = detect_runtime().await?;

    if !quiet {
        terminal::print_header("Building Demo Images");
        println!();
    }

    // Build images from this repo
    for img in DEMO_IMAGES {
        if !quiet {
            terminal::print_step(&format!("Building {}...", img.name));
        }

        let mut cmd = Command::new(runtime.container_cmd());
        cmd.args(["build", "-t", img.tag]);
        if let Some(dockerfile) = img.dockerfile {
            cmd.args(["-f", dockerfile]);
        }
        cmd.arg(img.context)
            .stdout(if quiet { Stdio::null() } else { Stdio::inherit() })
            .stderr(if quiet { Stdio::null() } else { Stdio::inherit() });

        let status = cmd.status().await?;
        if !status.success() {
            return Err(anyhow::anyhow!("Failed to build {}", img.name));
        }

        if !quiet {
            terminal::print_success(&format!("{} built", img.name));
        }
    }

    // Try sibling repos (non-fatal if missing)
    for img in SIBLING_IMAGES {
        let context_path = std::path::Path::new(img.context);
        if !context_path.exists() {
            if !quiet {
                terminal::print_info(&format!(
                    "Skipping {} (repo not found at {})",
                    img.name, img.context
                ));
            }
            continue;
        }

        // Check for Dockerfile in sibling
        let dockerfile_path = context_path.join("Dockerfile");
        if !dockerfile_path.exists() {
            if !quiet {
                terminal::print_info(&format!(
                    "Skipping {} (no Dockerfile in {})",
                    img.name, img.context
                ));
            }
            continue;
        }

        if !quiet {
            terminal::print_step(&format!("Building {}...", img.name));
        }

        let mut cmd = Command::new(runtime.container_cmd());
        cmd.args(["build", "-t", img.tag])
            .arg(img.context)
            .stdout(if quiet { Stdio::null() } else { Stdio::inherit() })
            .stderr(if quiet { Stdio::null() } else { Stdio::inherit() });

        let status = cmd.status().await?;
        if !status.success() {
            if !quiet {
                terminal::print_warning(&format!("Failed to build {} (non-fatal)", img.name));
            }
        } else if !quiet {
            terminal::print_success(&format!("{} built", img.name));
        }
    }

    if !quiet {
        println!();
        terminal::print_success("Done. Run 'scry demo start' to use local images.");
    }

    Ok(())
}

#[allow(dead_code)]
mod users {
    /// Get current user ID by reading /proc/self/status or falling back to `id -u`.
    pub fn get_current_uid() -> u32 {
        // Try /proc/self/status first (Linux)
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if let Some(uid_str) = line.strip_prefix("Uid:\t") {
                    // Format: real effective saved filesystem
                    if let Some(real_uid) = uid_str.split_whitespace().next() {
                        if let Ok(uid) = real_uid.parse::<u32>() {
                            return uid;
                        }
                    }
                }
            }
        }
        // Fallback
        1000
    }
}
