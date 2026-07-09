//! CI/CD pipeline command implementation.

use crate::client::ApiClient;
use crate::dvcs::Git;
use crate::error::CliError;
use crate::migration_detector::MigrationDetector;
use crate::output::OutputFormat;
use crate::repo_config::RepoConfig;
use crate::resolve::ShadowRef;
use chrono;
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

#[derive(Subcommand, Clone)]
pub enum CiCommand {
    /// Wait for shadow to be ready
    WaitReady {
        /// Shadow reference (UUID or source/shadow-name)
        shadow: String,

        /// Timeout duration (e.g., "5m", "1h")
        #[arg(long, default_value = "10m")]
        timeout: String,
    },

    /// Create a checkpoint before migration
    Checkpoint {
        /// Shadow reference (UUID or source/shadow-name)
        shadow: String,

        /// Checkpoint name
        #[arg(long)]
        name: String,
    },

    /// Output connection string for shadow
    Connect {
        /// Shadow reference (UUID or source/shadow-name)
        shadow: String,
    },

    /// Start replay and wait for completion
    Replay {
        /// Shadow reference (UUID or source/shadow-name)
        shadow: String,

        /// Wait for replay to complete (default: true)
        #[arg(long, default_value = "true", action = clap::ArgAction::Set)]
        wait: bool,

        /// Exit with error if regressions detected (default: true)
        #[arg(long, default_value = "true", action = clap::ArgAction::Set)]
        fail_on_regression: bool,

        /// Exit with error if drift threshold exceeded
        #[arg(long)]
        fail_on_drift: bool,

        /// Speed multiplier (1.0 = real-time, 2.0 = 2x, 0 = fast-forward)
        #[arg(long)]
        speed: Option<f64>,

        /// Maximum drift threshold (e.g., "10s", "500ms")
        #[arg(long)]
        max_drift: Option<String>,

        /// Timeout for --wait (e.g., "30m")
        #[arg(long, default_value = "30m")]
        timeout: String,

        /// Replay traffic from last duration (e.g., "2h", "7d")
        #[arg(long, conflicts_with = "since_checkpoint")]
        last: Option<String>,

        /// Replay traffic since checkpoint was created
        #[arg(long, conflicts_with = "last")]
        since_checkpoint: Option<String>,

        /// Absolute start time (RFC3339)
        #[arg(long, conflicts_with_all = ["last", "since_checkpoint"])]
        from: Option<String>,

        /// Absolute end time (RFC3339)
        #[arg(long, requires = "from")]
        to: Option<String>,
    },

    /// Reset shadow to a checkpoint
    Reset {
        /// Shadow reference (UUID or source/shadow-name)
        shadow: String,

        /// Checkpoint name to reset to (defaults to latest checkpoint)
        #[arg(long)]
        to: Option<String>,

        /// Wait for reset to complete (default: true)
        #[arg(long, default_value = "true", action = clap::ArgAction::Set)]
        wait: bool,

        /// Timeout for --wait (e.g., "10m")
        #[arg(long, default_value = "10m")]
        timeout: String,

        /// Fail if estimated restore time exceeds this duration
        #[arg(long)]
        max_restore_time: Option<String>,

        /// Show estimate without executing
        #[arg(long)]
        dry_run: bool,

        /// Proceed even if over max_restore_time
        #[arg(long)]
        force: bool,
    },

    /// Detect migrations in the current change set
    DetectMigrations {
        /// Base git ref to compare against (e.g., origin/main)
        #[arg(long, default_value = "origin/main")]
        base: String,
    },

    /// Open a TCP tunnel to a shadow database
    ///
    /// Creates a local TCP listener that tunnels PostgreSQL connections
    /// to a shadow database via HTTP/2. Migration tools can connect to
    /// localhost:PORT to run migrations against the shadow.
    ///
    /// Note: Adds ~2-10ms latency per round-trip. Acceptable for migrations,
    /// slightly sluggish for interactive psql sessions.
    Tunnel {
        /// Shadow reference (UUID or source/shadow-name)
        shadow: String,

        /// Local port to listen on (auto-discovers if not specified)
        #[arg(long)]
        port: Option<u16>,

        /// Command to execute (tunnel closes when command exits)
        #[arg(last = true)]
        command: Vec<String>,
    },

    /// Run migration and test for regressions
    ///
    /// The recommended CI command for testing database migrations:
    /// 1. Acquires exclusive lock on shadow (prevents concurrent operations)
    /// 2. Waits for shadow to be ready (up to 10 minutes)
    /// 3. Opens tunnel to shadow database
    /// 4. Runs your migration command
    /// 5. Replays recent queries to detect regressions
    /// 6. Releases lock
    ///
    /// On failure, prints actionable reset command.
    #[command(name = "test-migration")]
    TestMigration {
        /// Shadow reference (UUID or source/shadow-name). Defaults to ci.shadow_ref in .scry.toml
        shadow: Option<String>,

        /// Time window for replay (e.g., "2h", "4h"). Default: 2h
        #[arg(long, default_value = "2h")]
        replay_window: String,

        /// Use specific checkpoint instead of latest (for error messages)
        #[arg(long)]
        checkpoint: Option<String>,

        /// Skip replay after migration (run migration only)
        #[arg(long)]
        skip_replay: bool,

        /// Force-break any existing lock
        #[arg(long)]
        force: bool,

        /// Migration command to execute
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
}

// API response types

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShadowStatusResponse {
    pub id: Uuid,
    pub state: String,
    #[serde(default)]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConnectionResponse {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub connection_url: Option<String>,
}

impl ConnectionResponse {
    fn connection_string(&self) -> String {
        match &self.password {
            Some(pw) => format!(
                "postgres://{}:{}@{}:{}/{}",
                self.username, pw, self.host, self.port, self.database
            ),
            None => self.connection_url.clone().unwrap_or_else(|| {
                format!(
                    "postgres://{}@{}:{}/{}",
                    self.username, self.host, self.port, self.database
                )
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CheckpointInfo {
    pub name: String,
    pub size_bytes: Option<u64>,
    pub cdc_position: u64,
    /// When the checkpoint was created (Unix milliseconds).
    #[serde(default)]
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CheckpointsResponse {
    pub items: Vec<CheckpointInfo>,
}

/// Async checkpoint response (202) when a snapshot provider is configured.
#[derive(Debug, Clone, Deserialize)]
struct CheckpointJobResponse {
    pub job_id: String,
    #[allow(dead_code)]
    pub status: String,
    #[allow(dead_code)]
    pub checkpoint_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReplayResponse {
    pub id: Uuid,
    pub state: String,
    #[serde(default)]
    pub queries_processed: u64,
    #[serde(default)]
    pub queries_succeeded: u64,
    #[serde(default)]
    pub queries_failed: u64,
    #[serde(default)]
    pub regressions_detected: u64,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub timing_summary: Option<TimingSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TimingSummary {
    #[serde(default)]
    pub max_drift_ms: u64,
    #[serde(default)]
    pub degraded: bool,
}

impl CiCommand {
    /// Run commands that don't require an API client (e.g., detect-migrations).
    pub fn run_local(self, format: OutputFormat) -> Result<(), CliError> {
        match self {
            CiCommand::DetectMigrations { base } => Self::detect_migrations(&base, format),
            _ => Err(CliError::MissingConfig(
                "API URL not configured. Set SCRY_API_URL or run 'scry config set'".to_string(),
            )),
        }
    }

    pub async fn run(
        self,
        client: &ApiClient,
        format: OutputFormat,
        quiet: bool,
    ) -> Result<(), CliError> {
        match self {
            CiCommand::WaitReady { shadow, timeout } => {
                Self::wait_ready(client, &shadow, &timeout, quiet).await
            }
            CiCommand::Checkpoint { shadow, name } => {
                Self::checkpoint(client, &shadow, &name, quiet).await
            }
            CiCommand::Connect { shadow } => {
                Self::connect(client, &shadow, format).await
            }
            CiCommand::Replay {
                shadow,
                wait,
                fail_on_regression,
                fail_on_drift,
                speed,
                max_drift,
                timeout,
                last,
                since_checkpoint,
                from,
                to,
            } => {
                Self::replay(
                    client,
                    &shadow,
                    wait,
                    fail_on_regression,
                    fail_on_drift,
                    speed,
                    max_drift,
                    &timeout,
                    last,
                    since_checkpoint,
                    from,
                    to,
                    quiet,
                )
                .await
            }
            CiCommand::Reset {
                shadow,
                to,
                wait,
                timeout,
                max_restore_time,
                dry_run,
                force,
            } => {
                Self::reset(
                    client,
                    &shadow,
                    to,
                    wait,
                    &timeout,
                    max_restore_time,
                    dry_run,
                    force,
                    quiet,
                )
                .await
            }
            CiCommand::DetectMigrations { base } => Self::detect_migrations(&base, format),
            CiCommand::Tunnel {
                shadow,
                port,
                command,
            } => Self::tunnel(client, &shadow, port, command, quiet).await,
            CiCommand::TestMigration {
                shadow,
                replay_window,
                checkpoint,
                skip_replay,
                force,
                command,
            } => {
                Self::test_migration(
                    client,
                    shadow,
                    &replay_window,
                    checkpoint,
                    skip_replay,
                    force,
                    command,
                    quiet,
                )
                .await
            }
        }
    }

    fn detect_migrations(base: &str, format: OutputFormat) -> Result<(), CliError> {
        let git = Git::discover().map_err(|e| CliError::Other(e.to_string()))?;
        let repo_root = RepoConfig::find_repo_root()
            .ok_or_else(|| CliError::Other("Not in a git repository".to_string()))?;
        let config = RepoConfig::load(&repo_root);

        let detector = MigrationDetector::new(&git, &config.migration_detection);
        let result = detector
            .detect(base)
            .map_err(|e| CliError::Other(e.to_string()))?;

        match format {
            OutputFormat::Json => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&result)
                        .map_err(|e| CliError::Other(e.to_string()))?
                );
            }
            OutputFormat::Table => {
                if result.has_migrations {
                    println!("Migrations detected (confidence: {:?})", result.confidence);
                    for evidence in &result.evidence {
                        match &evidence.keywords {
                            Some(kw) => println!(
                                "  {} ({:?}): {}",
                                evidence.file,
                                evidence.evidence_type,
                                kw.join(", ")
                            ),
                            None => println!("  {} ({:?})", evidence.file, evidence.evidence_type),
                        }
                    }
                } else {
                    println!("No migrations detected");
                }
            }
        }

        Ok(())
    }

    async fn tunnel(
        client: &ApiClient,
        shadow_ref: &str,
        port: Option<u16>,
        command: Vec<String>,
        quiet: bool,
    ) -> Result<(), CliError> {
        use crate::tunnel::TunnelClient;
        use std::process::Stdio;

        // Auto-discover port if not specified
        let port = match port {
            Some(p) => p,
            None => find_available_port()?,
        };

        // Resolve shadow reference
        let shadow_id = ShadowRef::parse(shadow_ref)?.resolve(client).await?;

        let tunnel = TunnelClient::new(
            client.base_url().to_string(),
            client.token().to_string(),
            shadow_id.to_string(),
        );

        let listener = tunnel
            .listen(port)
            .await
            .map_err(|e| CliError::Other(e.to_string()))?;

        if !quiet {
            eprintln!("Tunnel open at localhost:{}", port);
            eprintln!("  Note: ~2-10ms added latency per round-trip");
        }

        if command.is_empty() {
            // No command - keep tunnel open until Ctrl+C
            if !quiet {
                eprintln!("Press Ctrl+C to close tunnel");
            }
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let tunnel_clone = TunnelClient::new(
                            client.base_url().to_string(),
                            client.token().to_string(),
                            shadow_id.to_string(),
                        );
                        tokio::spawn(async move {
                            if let Err(e) = tunnel_clone.handle_connection(stream).await {
                                eprintln!("Tunnel error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        return Err(CliError::Other(format!("Accept failed: {}", e)));
                    }
                }
            }
        } else {
            // Run command with tunnel, exit when command exits
            let connection_url = format!("postgres://localhost:{}/postgres", port);

            // Clone values for spawned task
            let base_url = client.base_url().to_string();
            let token = client.token().to_string();
            let sid = shadow_id.to_string();

            // Spawn connection handler in background
            let tunnel_handle = tokio::spawn(async move {
                loop {
                    if let Ok((stream, _)) = listener.accept().await {
                        let tc = TunnelClient::new(base_url.clone(), token.clone(), sid.clone());
                        tokio::spawn(async move {
                            let _ = tc.handle_connection(stream).await;
                        });
                    }
                }
            });

            // Run the command
            let mut child = tokio::process::Command::new(&command[0])
                .args(&command[1..])
                .env("DATABASE_URL", &connection_url)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|e| CliError::Other(format!("Failed to spawn command: {}", e)))?;

            let status = child
                .wait()
                .await
                .map_err(|e| CliError::Other(format!("Failed to wait for command: {}", e)))?;

            tunnel_handle.abort();

            if !status.success() {
                let code = status.code().unwrap_or(1);
                // Pass through exit code, offset by 10 for migration commands
                return Err(CliError::ExitCode(if code >= 10 { code } else { code + 10 }));
            }

            Ok(())
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn test_migration(
        client: &ApiClient,
        shadow_override: Option<String>,
        replay_window_override: &str,
        checkpoint_override: Option<String>,
        skip_replay: bool,
        force: bool,
        command: Vec<String>,
        quiet: bool,
    ) -> Result<(), CliError> {
        use crate::tunnel::TunnelClient;
        use std::process::Stdio;

        // Resolve shadow reference from arg or config
        let repo_root = RepoConfig::find_repo_root();
        let config = repo_root.as_ref().map(|r| RepoConfig::load(r));

        let shadow_ref = shadow_override
            .as_ref()
            .or_else(|| config.as_ref().and_then(|c| c.ci.shadow_ref.as_ref()))
            .ok_or_else(|| {
                CliError::Other(
                    "No shadow specified. Use shadow argument or set ci.shadow_ref in .scry.toml"
                        .to_string(),
                )
            })?
            .clone();

        // Use config replay_window if not overridden (default "2h" matches arg default)
        let replay_window = if replay_window_override != "2h" {
            replay_window_override.to_string()
        } else {
            config
                .as_ref()
                .map(|c| c.ci.replay_window.clone())
                .unwrap_or_else(|| "2h".to_string())
        };

        let shadow_id = ShadowRef::parse(&shadow_ref)?.resolve(client).await?;

        if !quiet {
            eprintln!("Testing migration on shadow {}", shadow_ref);
        }

        // Step 1: Acquire lock
        if !quiet {
            eprintln!("Acquiring lock...");
        }
        #[derive(Serialize, Clone)]
        struct LockRequest {
            operation: String,
            force: bool,
        }
        let lock_path = format!("/api/v1/shadows/{}/lock", shadow_id);
        let lock_result: Result<serde_json::Value, _> = client
            .post(
                &lock_path,
                &LockRequest {
                    operation: "test-migration".to_string(),
                    force,
                },
            )
            .await;

        if let Err(e) = &lock_result {
            let err_str = e.to_string();
            if err_str.contains("locked") || err_str.contains("409") {
                eprintln!("Error: Shadow is locked by another operation");
                eprintln!("Hint: Wait for the other operation to complete, or use --force to break the lock");
                return Err(CliError::ExitCode(1));
            }
            return Err(CliError::Other(format!("Failed to acquire lock: {}", e)));
        }

        let unlock_path = format!("/api/v1/shadows/{}/lock", shadow_id);

        // Step 2: Wait for shadow to be ready (10 minute timeout)
        if !quiet {
            eprintln!("Waiting for shadow to be ready...");
        }
        if let Err(e) = Self::wait_ready(client, &shadow_ref, "10m", quiet).await {
            let _: Result<serde_json::Value, _> = client.delete(&unlock_path).await;
            return Err(e);
        }

        // Step 3: Get latest checkpoint for error messages
        let checkpoints_path = format!("/api/v1/shadows/{}/checkpoints", shadow_id);
        let checkpoints: CheckpointsResponse = match client.get(&checkpoints_path).await {
            Ok(c) => c,
            Err(e) => {
                let _: Result<serde_json::Value, _> = client.delete(&unlock_path).await;
                return Err(e);
            }
        };

        let checkpoint_name = checkpoint_override.unwrap_or_else(|| {
            checkpoints
                .items
                .first()
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "unknown".to_string())
        });

        if checkpoints.items.is_empty() {
            let _: Result<serde_json::Value, _> = client.delete(&unlock_path).await;
            eprintln!("Error: No checkpoint found for shadow {}", shadow_ref);
            eprintln!("Hint: Checkpoints are created daily. If this is a new shadow, wait for the next sync cycle.");
            return Err(CliError::ExitCode(4));
        }

        // Step 4: Find available port and start tunnel
        let port = match find_available_port() {
            Ok(p) => p,
            Err(e) => {
                let _: Result<serde_json::Value, _> = client.delete(&unlock_path).await;
                return Err(e);
            }
        };

        if !quiet {
            eprintln!("Opening tunnel on port {}...", port);
        }

        let tunnel = TunnelClient::new(
            client.base_url().to_string(),
            client.token().to_string(),
            shadow_id.to_string(),
        );

        let listener = match tunnel.listen(port).await {
            Ok(l) => l,
            Err(e) => {
                let _: Result<serde_json::Value, _> = client.delete(&unlock_path).await;
                return Err(CliError::Other(e.to_string()));
            }
        };

        let connection_url = format!("postgres://localhost:{}/postgres", port);

        // Clone values for spawned task
        let base_url = client.base_url().to_string();
        let token = client.token().to_string();
        let sid = shadow_id.to_string();

        // Spawn connection handler in background
        let tunnel_handle = tokio::spawn(async move {
            loop {
                if let Ok((stream, _)) = listener.accept().await {
                    let tc = TunnelClient::new(base_url.clone(), token.clone(), sid.clone());
                    tokio::spawn(async move {
                        let _ = tc.handle_connection(stream).await;
                    });
                }
            }
        });

        // Step 5: Run migration command
        if !quiet {
            eprintln!("Running migration: {}", command.join(" "));
        }

        let mut child = tokio::process::Command::new(&command[0])
            .args(&command[1..])
            .env("DATABASE_URL", &connection_url)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                tunnel_handle.abort();
                CliError::Other(format!("Failed to spawn migration command: {}", e))
            })?;

        let status = child.wait().await.map_err(|e| {
            tunnel_handle.abort();
            CliError::Other(format!("Failed to wait for command: {}", e))
        })?;

        tunnel_handle.abort();

        if !status.success() {
            let _: Result<serde_json::Value, _> = client.delete(&unlock_path).await;
            let code = status.code().unwrap_or(1);
            eprintln!("Migration failed (exit code {})", code);
            eprintln!(
                "Reset to last checkpoint: scry ci reset {} --to {}",
                shadow_ref, checkpoint_name
            );
            return Err(CliError::ExitCode(if code >= 10 { code } else { code + 10 }));
        }

        if !quiet {
            eprintln!("Migration succeeded");
        }

        // Step 6: Run replay (unless skipped)
        if skip_replay {
            if !quiet {
                eprintln!("Skipping replay (--skip-replay)");
            }
            let _: Result<serde_json::Value, _> = client.delete(&unlock_path).await;
            return Ok(());
        }

        if !quiet {
            eprintln!("Running replay (last {})...", replay_window);
        }

        let replay_result = Self::replay(
            client,
            &shadow_ref,
            true,  // wait
            true,  // fail_on_regression
            false, // fail_on_drift
            None,  // speed (default)
            None,  // max_drift (default)
            "30m", // timeout
            Some(replay_window.to_string()),
            None, // since_checkpoint
            None, // from
            None, // to
            quiet,
        )
        .await;

        // Step 7: Release lock
        let _: Result<serde_json::Value, _> = client.delete(&unlock_path).await;

        match replay_result {
            Ok(()) => {
                if !quiet {
                    eprintln!("Migration test passed");
                }
                Ok(())
            }
            Err(CliError::RegressionsDetected(count)) => {
                eprintln!("Regressions detected ({} queries regressed)", count);
                eprintln!(
                    "Reset to rollback: scry ci reset {} --to {}",
                    shadow_ref, checkpoint_name
                );
                Err(CliError::ExitCode(5))
            }
            Err(e) => Err(e),
        }
    }

    async fn wait_ready(
        client: &ApiClient,
        shadow: &str,
        timeout: &str,
        quiet: bool,
    ) -> Result<(), CliError> {
        let shadow_id = ShadowRef::parse(shadow)?.resolve(client).await?;
        let timeout_duration = parse_duration(timeout)?;
        let poll_interval = Duration::from_secs(2);
        let start = std::time::Instant::now();

        if !quiet {
            eprintln!("Waiting for shadow {} to be ready...", shadow);
        }

        loop {
            let path = format!("/api/v1/shadows/{}", shadow_id);
            let response: ShadowStatusResponse = client.get(&path).await?;

            match response.state.as_str() {
                "ready" | "replaying" => {
                    if !quiet {
                        eprintln!("Shadow is ready");
                    }
                    return Ok(());
                }
                "failed" => {
                    return Err(CliError::Other(format!(
                        "Shadow failed: {}",
                        response.error_message.unwrap_or_default()
                    )));
                }
                state => {
                    if start.elapsed() >= timeout_duration {
                        return Err(CliError::Timeout(format!(
                            "Shadow not ready after {} (state: {})",
                            timeout, state
                        )));
                    }
                    if !quiet {
                        eprintln!("  State: {} - waiting...", state);
                    }
                    tokio::time::sleep(poll_interval).await;
                }
            }
        }
    }

    async fn checkpoint(
        client: &ApiClient,
        shadow: &str,
        name: &str,
        quiet: bool,
    ) -> Result<(), CliError> {
        let shadow_id = ShadowRef::parse(shadow)?.resolve(client).await?;
        let path = format!("/api/v1/shadows/{}/checkpoints", shadow_id);

        #[derive(Serialize, Clone)]
        struct CreateCheckpointRequest {
            name: String,
        }

        let request = CreateCheckpointRequest {
            name: name.to_string(),
        };

        let (status, body) = client.post_with_status(&path, &request).await?;

        if status == reqwest::StatusCode::ACCEPTED {
            // Async provider path: poll the job, then read the checkpoint back.
            let job: CheckpointJobResponse = serde_json::from_value(body)
                .map_err(|e| CliError::Api(format!("bad 202 body: {e}")))?;
            Self::wait_for_job(client, &job.job_id, quiet).await?;
            let cp = Self::get_checkpoint_by_name(client, shadow_id, name).await?;
            if !quiet {
                eprintln!(
                    "Checkpoint '{}' created at journal position {}",
                    cp.name, cp.cdc_position
                );
            }
        } else {
            // Synchronous provider="none" path: body is a CheckpointInfo.
            let cp: CheckpointInfo = serde_json::from_value(body)
                .map_err(|e| CliError::Api(format!("bad checkpoint body: {e}")))?;
            if !quiet {
                eprintln!(
                    "Checkpoint '{}' created at journal position {}",
                    cp.name, cp.cdc_position
                );
            }
        }

        Ok(())
    }

    /// Poll a job to a terminal state (completed/failed), mirroring the
    /// `job.rs` polling shape (fixed interval, elapsed-based timeout).
    async fn wait_for_job(
        client: &ApiClient,
        job_id: &str,
        quiet: bool,
    ) -> Result<(), CliError> {
        use crate::commands::job::JobResponse;
        let timeout = Duration::from_secs(300);
        let poll = Duration::from_secs(2);
        let start = std::time::Instant::now();
        loop {
            let resp: JobResponse = client.get(&format!("/api/v1/jobs/{}", job_id)).await?;
            match resp.status.as_str() {
                "completed" => return Ok(()),
                "failed" => {
                    return Err(CliError::Api(format!(
                        "checkpoint job failed: {}",
                        resp.error.unwrap_or_default()
                    )))
                }
                s => {
                    if start.elapsed() > timeout {
                        return Err(CliError::Timeout(format!(
                            "checkpoint job {job_id} timed out"
                        )));
                    }
                    if !quiet {
                        eprintln!("checkpoint job status: {s}...");
                    }
                    tokio::time::sleep(poll).await;
                }
            }
        }
    }

    /// Read the named checkpoint back from the shadow's checkpoint list.
    async fn get_checkpoint_by_name(
        client: &ApiClient,
        shadow_id: Uuid,
        name: &str,
    ) -> Result<CheckpointInfo, CliError> {
        let list: CheckpointsResponse = client
            .get(&format!("/api/v1/shadows/{}/checkpoints", shadow_id))
            .await?;
        list.items
            .into_iter()
            .find(|c| c.name == name)
            .ok_or_else(|| {
                CliError::Api(format!("checkpoint '{name}' not found after job completion"))
            })
    }

    async fn connect(
        client: &ApiClient,
        shadow: &str,
        format: OutputFormat,
    ) -> Result<(), CliError> {
        let shadow_id = ShadowRef::parse(shadow)?.resolve(client).await?;
        let path = format!("/api/v1/shadows/{}/connection", shadow_id);

        let response: ConnectionResponse = client.get(&path).await?;

        match format {
            OutputFormat::Json => {
                println!("{}", serde_json::to_string_pretty(&response)
                    .map_err(|e| CliError::Other(format!("JSON serialization failed: {e}")))?
                );
            }
            OutputFormat::Table => {
                // For piping to other commands, just output the connection string
                println!("{}", response.connection_string());
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn replay(
        client: &ApiClient,
        shadow: &str,
        wait: bool,
        fail_on_regression: bool,
        fail_on_drift: bool,
        speed: Option<f64>,
        max_drift: Option<String>,
        timeout: &str,
        last: Option<String>,
        since_checkpoint: Option<String>,
        from: Option<String>,
        to: Option<String>,
        quiet: bool,
    ) -> Result<(), CliError> {
        let shadow_id = ShadowRef::parse(shadow)?.resolve(client).await?;

        // Parse max_drift to milliseconds
        let max_drift_ms = match &max_drift {
            Some(dur) => Some(parse_duration(dur)?.as_millis() as u64),
            None => None,
        };

        // Build replay request
        #[derive(Serialize, Clone)]
        struct StartReplayRequest {
            #[serde(skip_serializing_if = "Option::is_none")]
            start_time_ms: Option<u64>,
            #[serde(skip_serializing_if = "Option::is_none")]
            end_time_ms: Option<u64>,
            #[serde(skip_serializing_if = "Option::is_none")]
            speed: Option<f64>,
            #[serde(skip_serializing_if = "Option::is_none")]
            max_drift_ms: Option<u64>,
        }

        // Calculate absolute timestamps from relative options
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| CliError::Other(format!("Failed to get current time: {}", e)))?
            .as_millis() as u64;

        let (start_time_ms, end_time_ms) = match (&last, &since_checkpoint, &from) {
            // --last 30s → (now - 30s, now)
            (Some(dur), None, None) => {
                let duration_ms = parse_duration(dur)?.as_millis() as u64;
                (Some(now_ms.saturating_sub(duration_ms)), Some(now_ms))
            }
            // --since-checkpoint NAME → (checkpoint.created_at, now or --to)
            (None, Some(checkpoint_name), None) => {
                let checkpoint = Self::get_checkpoint(client, shadow_id, checkpoint_name).await?;
                let end = match &to {
                    Some(ts) => Some(Self::parse_rfc3339_to_ms(ts)?),
                    None => Some(now_ms),
                };
                (Some(checkpoint.created_at), end)
            }
            // --from TIMESTAMP [--to TIMESTAMP]
            (None, None, Some(from_ts)) => {
                let start = Self::parse_rfc3339_to_ms(from_ts)?;
                let end = match &to {
                    Some(ts) => Some(Self::parse_rfc3339_to_ms(ts)?),
                    None => Some(now_ms),
                };
                (Some(start), end)
            }
            // No time options → continuous from current position (no time filter)
            (None, None, None) => (None, None),
            // Invalid combinations should be prevented by clap conflicts_with
            _ => return Err(CliError::Other("Invalid time option combination".to_string())),
        };

        let request = StartReplayRequest {
            start_time_ms,
            end_time_ms,
            speed,
            max_drift_ms,
        };

        let path = format!("/api/v1/shadows/{}/replays", shadow_id);
        let response: ReplayResponse = client.post(&path, &request).await?;

        if !quiet {
            eprintln!("Replay started: {}", response.id);
        }

        if !wait {
            println!("{}", response.id);
            return Ok(());
        }

        // Poll until complete
        let timeout_duration = parse_duration(timeout)?;
        let poll_interval = Duration::from_secs(5);
        let start = std::time::Instant::now();

        loop {
            let status_path = format!("/api/v1/replays/{}", response.id);
            let status: ReplayResponse = client.get(&status_path).await?;

            match status.state.as_str() {
                "completed" => {
                    let drift_info = status.timing_summary.as_ref();
                    let max_drift = drift_info.map(|t| t.max_drift_ms).unwrap_or(0);
                    let degraded = drift_info.map(|t| t.degraded).unwrap_or(false);

                    if !quiet {
                        eprintln!(
                            "Replay completed: {} queries, {} succeeded, {} failed, {} regressions, max drift {}ms{}",
                            status.queries_processed,
                            status.queries_succeeded,
                            status.queries_failed,
                            status.regressions_detected,
                            max_drift,
                            if degraded { " (degraded)" } else { "" }
                        );
                    }

                    if fail_on_regression && status.regressions_detected > 0 {
                        return Err(CliError::RegressionsDetected(status.regressions_detected));
                    }

                    if fail_on_drift && degraded {
                        return Err(CliError::DriftExceeded(max_drift));
                    }

                    return Ok(());
                }
                "failed" => {
                    return Err(CliError::Other(format!(
                        "Replay failed: {}",
                        status.error_message.unwrap_or_default()
                    )));
                }
                state => {
                    if start.elapsed() >= timeout_duration {
                        return Err(CliError::Timeout(format!(
                            "Replay not completed after {} (state: {}, processed: {})",
                            timeout, state, status.queries_processed
                        )));
                    }
                    if !quiet {
                        eprintln!(
                            "  {} - {} queries processed, {} regressions...",
                            state, status.queries_processed, status.regressions_detected
                        );
                    }
                    tokio::time::sleep(poll_interval).await;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn reset(
        client: &ApiClient,
        shadow: &str,
        to: Option<String>,
        wait: bool,
        timeout: &str,
        max_restore_time: Option<String>,
        dry_run: bool,
        force: bool,
        quiet: bool,
    ) -> Result<(), CliError> {
        let shadow_id = ShadowRef::parse(shadow)?.resolve(client).await?;

        // Get checkpoint info for size estimate
        let checkpoints_path = format!("/api/v1/shadows/{}/checkpoints", shadow_id);
        let checkpoints: CheckpointsResponse = client.get(&checkpoints_path).await?;

        // If no checkpoint specified, use the latest (first in list)
        let checkpoint = match &to {
            Some(name) => checkpoints
                .items
                .iter()
                .find(|c| &c.name == name)
                .ok_or_else(|| CliError::CheckpointNotFound(name.clone()))?,
            None => checkpoints
                .items
                .first()
                .ok_or_else(|| CliError::Other("No checkpoints available".to_string()))?,
        };
        let checkpoint_name = &checkpoint.name;

        let size_bytes = checkpoint.size_bytes.unwrap_or(0);
        let est_restore_secs = size_bytes / (100 * 1024 * 1024); // ~100 MB/s
        let est_restore_secs = est_restore_secs.max(1);

        // Check max_restore_time limit
        if let Some(max_time) = &max_restore_time {
            let max_duration = parse_duration(max_time)?;
            if est_restore_secs > max_duration.as_secs() && !force {
                return Err(CliError::Other(format!(
                    "Estimated restore time (~{}s) exceeds limit ({}). Use --force to proceed.",
                    est_restore_secs, max_time
                )));
            }
        }

        // Dry run - just show estimate
        if dry_run {
            let size_str = format_size(size_bytes);
            eprintln!("Checkpoint: {}", checkpoint_name);
            eprintln!("Size: {}", size_str);
            eprintln!("Estimated restore time: ~{}s", est_restore_secs);
            return Ok(());
        }

        // Perform reset
        if !quiet {
            let size_str = format_size(size_bytes);
            eprintln!(
                "Restoring checkpoint '{}' ({}, ~{}s estimated)",
                checkpoint_name, size_str, est_restore_secs
            );
        }

        #[derive(Serialize, Clone)]
        struct ResetRequest {
            checkpoint_name: String,
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct ResetResponse {
            #[serde(default)]
            state: String,
        }

        let reset_path = format!("/api/v1/shadows/{}/reset", shadow_id);
        let request = ResetRequest {
            checkpoint_name: checkpoint_name.to_string(),
        };

        let _response: ResetResponse = client.post(&reset_path, &request).await?;

        if !wait {
            if !quiet {
                eprintln!("Reset started");
            }
            return Ok(());
        }

        // Poll until complete
        let timeout_duration = parse_duration(timeout)?;
        let poll_interval = Duration::from_secs(2);
        let start = std::time::Instant::now();

        loop {
            let status_path = format!("/api/v1/shadows/{}", shadow_id);
            let status: ShadowStatusResponse = client.get(&status_path).await?;

            match status.state.as_str() {
                "ready" => {
                    if !quiet {
                        let elapsed = start.elapsed().as_secs();
                        eprintln!("Checkpoint restored in {}s", elapsed);
                    }
                    return Ok(());
                }
                "failed" => {
                    return Err(CliError::Other(format!(
                        "Reset failed: {}",
                        status.error_message.unwrap_or_default()
                    )));
                }
                state => {
                    if start.elapsed() >= timeout_duration {
                        return Err(CliError::Timeout(format!(
                            "Reset not completed after {} (state: {})",
                            timeout, state
                        )));
                    }
                    if !quiet {
                        let elapsed = start.elapsed().as_secs();
                        eprintln!("  Restoring... {}s elapsed", elapsed);
                    }
                    tokio::time::sleep(poll_interval).await;
                }
            }
        }
    }

    /// Parse RFC3339 timestamp to Unix milliseconds.
    fn parse_rfc3339_to_ms(s: &str) -> Result<u64, CliError> {
        let dt = chrono::DateTime::parse_from_rfc3339(s)
            .map_err(|e| CliError::Other(format!("Invalid timestamp '{}': {}", s, e)))?;
        Ok(dt.timestamp_millis() as u64)
    }

    /// Get a checkpoint by name.
    async fn get_checkpoint(
        client: &ApiClient,
        shadow_id: uuid::Uuid,
        name: &str,
    ) -> Result<CheckpointInfo, CliError> {
        let path = format!("/api/v1/shadows/{}/checkpoints", shadow_id);
        let response: CheckpointsResponse = client.get(&path).await?;

        response
            .items
            .into_iter()
            .find(|c| c.name == name)
            .ok_or_else(|| CliError::CheckpointNotFound(name.to_string()))
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

/// Find an available port in the preferred range.
///
/// Tries ports in order: 15432, 15433, ..., 15532 (100 ports total).
/// Returns the first available port or an error if none are available.
pub fn find_available_port() -> Result<u16, CliError> {
    use std::net::TcpListener;

    const START_PORT: u16 = 15432;
    const END_PORT: u16 = 15532;

    for port in START_PORT..=END_PORT {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }

    Err(CliError::Other(format!(
        "No available ports in range {}-{}",
        START_PORT, END_PORT
    )))
}

/// Parse duration string like "5m", "1h", "30s"
pub fn parse_duration(s: &str) -> Result<Duration, CliError> {
    let s = s.trim();

    if let Some(num) = s.strip_suffix("ms") {
        let n: u64 = num.parse().map_err(|_| {
            CliError::Other(format!("Invalid duration '{}': not a number", s))
        })?;
        return Ok(Duration::from_millis(n));
    }

    if let Some(num) = s.strip_suffix('s') {
        let n: u64 = num.parse().map_err(|_| {
            CliError::Other(format!("Invalid duration '{}': not a number", s))
        })?;
        return Ok(Duration::from_secs(n));
    }

    if let Some(num) = s.strip_suffix('m') {
        let n: u64 = num.parse().map_err(|_| {
            CliError::Other(format!("Invalid duration '{}': not a number", s))
        })?;
        return Ok(Duration::from_secs(n * 60));
    }

    if let Some(num) = s.strip_suffix('h') {
        let n: u64 = num.parse().map_err(|_| {
            CliError::Other(format!("Invalid duration '{}': not a number", s))
        })?;
        return Ok(Duration::from_secs(n * 3600));
    }

    if let Some(num) = s.strip_suffix('d') {
        let n: u64 = num.parse().map_err(|_| {
            CliError::Other(format!("Invalid duration '{}': not a number", s))
        })?;
        return Ok(Duration::from_secs(n * 86400));
    }

    Err(CliError::Other(format!(
        "Invalid duration '{}': expected format like '5s', '10m', '1h'",
        s
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_job_response_decodes() {
        let json = r#"{"job_id":"550e8400-e29b-41d4-a716-446655440000",
            "status":"pending","checkpoint_id":"pre-migration"}"#;
        let resp: CheckpointJobResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.checkpoint_id, "pre-migration");
    }

    #[test]
    fn checkpoint_info_still_decodes_sync_path() {
        let json = r#"{"name":"pre-migration","cdc_position":42,"created_at":0}"#;
        let info: CheckpointInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.name, "pre-migration");
    }

    #[test]
    fn connection_response_tolerates_missing_password() {
        // No password field, but connection_url present → deserializes + usable.
        let json = r#"{"host":"h","port":5432,"database":"db","username":"u",
            "connection_url":"postgres://u:pw@h:5432/db"}"#;
        let resp: ConnectionResponse = serde_json::from_str(json).unwrap();
        assert!(resp.password.is_none());
        assert_eq!(resp.connection_string(), "postgres://u:pw@h:5432/db");
    }

    #[test]
    fn connection_response_uses_password_when_present() {
        let json = r#"{"host":"h","port":5432,"database":"db","username":"u",
            "password":"secret","connection_url":"postgres://ignored"}"#;
        let resp: ConnectionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.connection_string(), "postgres://u:secret@h:5432/db");
    }

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86400));
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert!(parse_duration("5").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("5x").is_err());
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(2_500_000), "2.4 MB");
        assert_eq!(format_size(2_500_000_000), "2.3 GB");
    }

    #[test]
    fn test_find_available_port_returns_port_in_range() {
        let port = find_available_port().unwrap();
        assert!(port >= 15432);
        assert!(port <= 15532);
    }

    #[test]
    fn test_find_available_port_skips_occupied() {
        use std::net::TcpListener;

        // Find an available port first
        let first_port = find_available_port().unwrap();

        // Occupy that port
        let _listener = TcpListener::bind(("127.0.0.1", first_port)).unwrap();

        // find_available_port should return a different port
        let second_port = find_available_port().unwrap();
        assert_ne!(
            first_port, second_port,
            "Should skip occupied port {}, got {}",
            first_port, second_port
        );
    }
}
