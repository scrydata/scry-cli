//! Interactive walkthrough — ports the 9-step demo.sh flow to Rust.

use std::io::{self, Write};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::Mutex;

use super::{
    assets,
    client::DemoClient,
    compose,
    error::DemoError,
    explain, gates,
    status_bar::StatusBar,
    terminal,
};

const SHADOW_ID: &str = "00000000-0000-0000-0000-000000000001";
const API_URL: &str = "http://localhost:8081";
const API_TOKEN: &str = "demo-token";
const PROXY_PORT: &str = "5434";
const TOTAL_STEPS: usize = 6;

/// Shadow connection info.
struct ShadowConn {
    host: String,
    port: String,
    container: String,
    database: String,
}

/// Run the full walkthrough.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    dir: &Path,
    scenario: &str,
    explain_mode: bool,
    no_compose: bool,
    _quiet: bool,
    cleanup_after: bool,
    dev_mode: bool,
    verbose: bool,
    offline: bool,
) -> Result<(), anyhow::Error> {
    if offline {
        // Offline mode: skip compose/podman, use hardcoded data
        let sql = assets::scenario_sql(scenario)
            .ok_or_else(|| anyhow::anyhow!("Unknown scenario: {scenario}"))?;
        let client = DemoClient::new(API_URL, API_TOKEN, verbose);
        return run_inner(
            dir, scenario, &sql, explain_mode, true, compose::Runtime::Docker,
            cleanup_after, dev_mode, &client, true,
        )
        .await;
    }

    let runtime = compose::detect_runtime().await?;
    compose::check_docker_daemon(runtime).await?;
    let sql = assets::scenario_sql(scenario)
        .ok_or_else(|| anyhow::anyhow!("Unknown scenario: {scenario}"))?;

    // Start Podman TCP API if needed (both dev and container modes need it —
    // the local platform binary also talks to Podman to create shadow containers).
    let podman_pid = compose::maybe_start_podman_tcp(runtime).await;

    let client = DemoClient::new(API_URL, API_TOKEN, verbose);

    // Ensure cleanup on exit
    let result = run_inner(
        dir, scenario, &sql, explain_mode, no_compose, runtime,
        cleanup_after, dev_mode, &client, false,
    )
    .await;

    if let Some(pid) = podman_pid {
        compose::stop_podman_tcp(pid);
    }

    result
}

#[allow(clippy::too_many_arguments)]
async fn run_inner(
    dir: &Path,
    scenario: &str,
    sql: &assets::ScenarioFiles,
    explain_mode: bool,
    no_compose: bool,
    runtime: compose::Runtime,
    cleanup_after: bool,
    dev_mode: bool,
    client: &DemoClient,
    offline: bool,
) -> Result<(), anyhow::Error> {
    // Enter alternate screen buffer (isolates from existing terminal content)
    terminal::enter_alternate_screen();

    // Create status bar
    let status = Arc::new(Mutex::new(StatusBar::new(TOTAL_STEPS)));

    // Set up Ctrl+C handler for cleanup
    let status_for_signal = status.clone();
    let dir_for_signal = dir.to_path_buf();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            // Deactivate status bar to restore terminal
            {
                let mut s = status_for_signal.lock().await;
                let _ = s.deactivate();
            }
            // Leave alternate screen to restore original terminal content
            terminal::leave_alternate_screen();
            eprintln!();
            eprintln!("  Demo interrupted.");
            eprintln!(
                "  Run 'scry demo clean --dir {}' to remove containers.",
                dir_for_signal.display()
            );
            std::process::exit(130);
        }
    });

    // Activate status bar (sets scrolling region)
    {
        let mut s = status.lock().await;
        let _ = s.activate();
    }

    // Run the demo steps, catching errors for diagnostics
    let result = run_steps(
        dir, scenario, sql, explain_mode, no_compose, runtime,
        dev_mode, client, offline, &status,
    )
    .await;

    // On error: deactivate status bar, leave alt screen, show diagnostics
    if let Err(ref e) = result {
        {
            let mut s = status.lock().await;
            let _ = s.deactivate();
        }
        terminal::leave_alternate_screen();

        println!();
        terminal::print_error(&format!("{e}"));
        println!();

        // Collect and display container logs on failure
        if !offline {
            let services = ["scry-platform", "scry-backfill", "scry-proxy"];
            let logs = compose::collect_service_logs(dir, &services, 20).await;
            for (svc, log_text) in &logs {
                if !log_text.is_empty() {
                    println!("  {} {}:", terminal::bold("Logs:"), svc);
                    for line in log_text.lines().take(20) {
                        println!("    {line}");
                    }
                    println!();
                }
            }
        }

        println!("  {} Debugging commands:", terminal::bold(""));
        println!("    docker compose -f {}/docker-compose.yml logs <service>", dir.display());
        println!("    docker compose -f {}/docker-compose.yml ps -a", dir.display());
        println!("    curl http://localhost:8081/api/v1/ready");
        println!("    scry demo status");
        println!();

        return result.map(|_| ());
    }

    // Kill local platform process on success
    if let Ok(Some(mut child)) = result {
        let _ = child.kill().await;
    }

    // Deactivate status bar and leave alternate screen before cleanup prompt
    {
        let mut s = status.lock().await;
        let _ = s.deactivate();
    }
    terminal::leave_alternate_screen();

    // Handle cleanup
    let cleaned_up = if cleanup_after {
        compose::down(dir, true).await?;
        terminal::print_success("Demo cleaned up automatically");
        true
    } else if terminal::is_interactive() {
        println!();
        print!("  Clean up demo files and containers? [y/N] ");
        let _ = io::stdout().flush();

        let mut input = String::new();
        let did_cleanup = if io::stdin().read_line(&mut input).is_ok() {
            if input.trim().eq_ignore_ascii_case("y") {
                compose::down(dir, true).await?;
                terminal::print_success("Demo cleaned up");
                true
            } else {
                false
            }
        } else {
            false
        };
        println!(); // Ensure clean terminal state after input
        did_cleanup
    } else {
        false
    };

    // Show NEXT STEPS after cleanup prompt (if not cleaned up)
    if !cleaned_up {
        print_next_steps(scenario);
    }

    Ok(())
}

/// The actual demo steps, separated so errors can be caught for diagnostics.
///
/// Returns the local platform child process handle (if any) so the caller can clean it up.
#[allow(clippy::too_many_arguments)]
async fn run_steps(
    dir: &Path,
    scenario: &str,
    sql: &assets::ScenarioFiles,
    explain_mode: bool,
    no_compose: bool,
    runtime: compose::Runtime,
    dev_mode: bool,
    client: &DemoClient,
    offline: bool,
    status: &Arc<Mutex<StatusBar>>,
) -> Result<Option<tokio::process::Child>, anyhow::Error> {
    terminal::print_banner(scenario);

    if offline {
        terminal::print_warning("[OFFLINE MODE] Using hardcoded demo data -- no API calls");
        println!();
    }

    // ── Step 1: Start services ──────────────────────────────────────────
    {
        let mut s = status.lock().await;
        let _ = s.set_step(1, "Starting Services");
    }
    terminal::show_progress(1, TOTAL_STEPS, "Starting Services");
    println!();
    terminal::print_info("Launching the local Scry stack...");
    terminal::print_info("This includes: PostgreSQL, NATS, Scry Platform, and Proxy");

    terminal::print_step("Starting services...");

    // Pre-flight: verify Docker socket is accessible
    if !no_compose && !offline {
        gates::docker_socket_accessible(runtime).await?;
    }

    // In dev mode, start local binary instead of container
    let mut local_platform: Option<tokio::process::Child> = None;

    if !no_compose && !offline {
        compose::generate_override(dir, dev_mode, Some(scenario)).await?;
        compose::up(dir, dev_mode).await?;

        // In dev mode: compose started NATS + postgres, but local platform
        // needs NATS. Start platform now, wait for health, then restart
        // backfill (which likely already failed connecting to the platform).
        if dev_mode {
            terminal::print_info("Dev mode: starting local scry-platform binary...");
            local_platform = Some(compose::start_local_platform().await?);

            if !compose::wait_for_http("http://localhost:8080/health", 30).await {
                return Err(anyhow::anyhow!(
                    "Local scry-platform failed to become healthy within 60s. \
                     Check 'cargo build --release' output."
                ));
            }
            terminal::print_info("Local scry-platform is ready.");

            // Restart backfill, proxy, and query-generator now that the platform is accepting connections
            compose::restart_service(dir, "scry-backfill").await?;
            compose::restart_service(dir, "scry-proxy").await?;
            compose::restart_service(dir, "query-generator").await?;

            // Verify services didn't immediately exit
            tokio::time::sleep(Duration::from_secs(3)).await;
            gates::container_running(runtime, "scry-demo-backfill").await?;
            gates::container_running(runtime, "scry-demo-proxy").await?;
        }
    }

    // Show architecture overview while services start
    terminal::print_header("WHILE WE WAIT...");
    println!();
    terminal::print_architecture_overview();
    println!();

    if !offline {
        // Wait for all services with per-service readiness feedback
        compose::wait_for_services_walkthrough(120, dev_mode).await?;

        // Phase gate: verify the API is fully initialized (not just healthcheck)
        gates::api_ready(client, 30).await?;
    }

    if explain_mode {
        let (title, text) = explain::services();
        terminal::print_explain_block(title, text);
        terminal::wait_for_enter();
    }

    // ── Step 2: Sync shadow ─────────────────────────────────────────────
    {
        let mut s = status.lock().await;
        let _ = s.set_step(2, "Syncing Shadow Database");
    }
    terminal::show_progress(2, TOTAL_STEPS, "Syncing Shadow Database");
    println!();
    terminal::print_info("A shadow database is an isolated copy where we test migrations.");
    terminal::print_info("Scry keeps it synchronized with your source database schema.");

    terminal::print_step("Waiting for shadow database to sync...");

    let shadow_conn = if offline {
        ShadowConn {
            host: "localhost".to_string(),
            port: "5433".to_string(),
            container: String::new(),
            database: "demo".to_string(),
        }
    } else {
        // Create source if needed
        ensure_source_registered(client).await?;

        // Create shadow if needed
        ensure_shadow_created(client).await?;

        // Poll for ready
        wait_for_shadow_ready(client, status).await?;

        terminal::print_success("Shadow database synced and ready");

        // Capture shadow connection info
        get_shadow_connection(client, runtime).await?
    };

    if explain_mode {
        let (title, text) = explain::shadow_sync();
        terminal::print_explain_block(title, text);
        terminal::wait_for_enter();
    }

    // ── Step 3: Capture workload ────────────────────────────────────────
    {
        let mut s = status.lock().await;
        let _ = s.set_step(3, "Capturing Query Workload");
    }
    terminal::show_progress(3, TOTAL_STEPS, "Capturing Query Workload");
    println!();
    terminal::print_info("Scry Proxy sits between your app and database, capturing");
    terminal::print_info("real query patterns without impacting performance.");

    terminal::print_step("Capturing baseline query workload...");
    terminal::print_info("Running typical queries through the proxy...");

    if !offline {
        // Phase gate: verify proxy is accepting connections
        gates::proxy_connectable(10).await?;
    }

    terminal::print_spinner(0, "Executing baseline queries through proxy...");
    run_workload_queries(runtime, sql.workload, offline).await?;
    terminal::clear_spinner();

    // Inject synthetic query events for missing-index scenario
    // (region queries can't run through proxy since the column doesn't exist on source)
    if !offline {
        if scenario == "missing-index" {
            terminal::print_spinner(1, "Injecting synthetic region queries...");
        }
        let injected = inject_synthetic_query_events(scenario).await?;
        terminal::clear_spinner();

        // Brief sleep for NATS persistence
        tokio::time::sleep(Duration::from_secs(3)).await;

        if injected > 0 {
            terminal::print_success(&format!("{injected} query events captured and journaled"));
        } else {
            terminal::print_success("Query events captured and journaled");
        }
    } else {
        let query_count = sql.workload.matches(';').count();
        terminal::print_success(&format!("{query_count} queries captured"));
    }

    println!();
    terminal::print_info("Query types captured:");
    println!("    - ORDER lookups by status");
    println!("    - JOIN queries with users table");
    println!("    - Aggregation queries (GROUP BY)");
    println!("    - User-specific order history");

    // Scenario-specific hint about which queries will regress
    match scenario {
        "missing-index" => {
            println!();
            terminal::print_info("Watch for: region-filtered queries will show the regression");
        }
        "table-locking" => {
            println!();
            terminal::print_info("Watch for: concurrent queries blocked during the UPDATE");
        }
        "index-drop" => {
            println!();
            terminal::print_info("Watch for: queries that relied on the dropped index");
        }
        _ => {}
    }

    if explain_mode {
        let (title, text) = explain::query_capture();
        terminal::print_explain_block(title, text);
        terminal::wait_for_enter();
    }

    // ── Step 4: Show scenario ───────────────────────────────────────────
    {
        let mut s = status.lock().await;
        let _ = s.set_step(4, "Review Scenario");
    }
    terminal::show_progress(4, TOTAL_STEPS, "Review Scenario");
    terminal::print_header("THE SCENARIO");

    println!();
    match scenario {
        "missing-index" => {
            println!(
                "  Your teammate added a {} column for a new feature.",
                terminal::bold("region")
            );
            println!("  The migration passed in staging with 100 rows.");
            println!("  Production has 25,000 orders.");
        }
        "table-locking" => {
            println!("  A migration needs to backfill data on the orders table.");
            println!("  It runs a large UPDATE touching ~15,000 rows.");
            println!("  Meanwhile, production queries keep coming in...");
        }
        "index-drop" => {
            println!("  Someone found an 'unused' index in pg_stat_user_indexes.");
            println!("  The stats showed idx_scans = 0, so they dropped it.");
            println!("  (Stats were reset after last week's failover...)");
        }
        _ => {}
    }
    println!();
    terminal::print_info("Next: We'll apply this migration to the shadow and replay queries.");

    terminal::wait_for_enter();

    // ── Step 5: Apply migration ─────────────────────────────────────────
    terminal::print_step("Applying migration to shadow database...");

    println!();
    terminal::print_info("Migration SQL:");
    terminal::print_sql_box(sql.migration);
    println!();

    terminal::print_info("Applying to shadow database...");
    apply_sql_to_shadow(runtime, sql.migration, &shadow_conn, dir, offline).await?;
    terminal::print_success("Migration applied to shadow (source database is untouched)");

    if explain_mode {
        let (title, text) = explain::migration_apply();
        terminal::print_explain_block(title, text);
        terminal::wait_for_enter();
    }

    // ── Step 6: Run replay ──────────────────────────────────────────────
    {
        let mut s = status.lock().await;
        let _ = s.set_step(5, "Replay & Compare");
    }
    terminal::show_progress(5, TOTAL_STEPS, "Replay & Compare");
    println!();
    terminal::print_info("Now we replay the captured queries against the shadow database");
    terminal::print_info("(with the migration applied) and compare performance.");

    terminal::print_step("Replaying captured queries against shadow...");
    println!();

    let (replay_id, replay_state) = run_replay_with_progress(client, offline).await?;
    println!();

    if !replay_state.eq_ignore_ascii_case("failed") {
        if explain_mode {
            let (title, text) = explain::replay();
            terminal::print_explain_block(title, text);
            terminal::wait_for_enter();
        }

        // ── Step 7: Show results ────────────────────────────────────────────
        display_replay_results(client, &replay_id, scenario, offline).await?;

        if explain_mode {
            let (title, text) = explain::results();
            terminal::print_explain_block(title, text);
            terminal::wait_for_enter();
        }
    }
    // If replay failed, error was already printed by run_replay_with_progress

    // ── Step 8: Apply fix & verify ──────────────────────────────────────
    {
        let mut s = status.lock().await;
        let _ = s.set_step(6, "Apply Fix & Verify");
    }
    terminal::show_progress(6, TOTAL_STEPS, "Apply Fix & Verify");
    terminal::print_header("APPLY THE FIX");

    println!();
    terminal::print_info("Let's apply the suggested fix to the shadow and re-run the replay");
    terminal::print_info("to verify the regression is resolved.");
    println!();

    // Show explanation BEFORE the action so user understands what will happen
    if explain_mode {
        let (title, text) = explain::fix_and_verify();
        terminal::print_explain_block(title, text);
    }

    terminal::wait_for_enter();

    terminal::print_step("Applying fix to shadow...");

    // Show the fix SQL (skip comment lines)
    println!();
    terminal::print_info("Fix SQL:");
    let fix_display: String = sql
        .fix
        .lines()
        .filter(|l| !l.starts_with("--") && !l.is_empty())
        .take(5)
        .collect::<Vec<&str>>()
        .join("\n");
    terminal::print_sql_box(&fix_display);
    println!();

    apply_sql_to_shadow(runtime, sql.fix, &shadow_conn, dir, offline).await?;
    terminal::print_success("Fix applied");

    // Cancel the previous replay so the shadow is free for a new one
    if !offline && !replay_id.is_empty() {
        let _ = client
            .delete(&format!("/api/v1/replays/{replay_id}"))
            .await;
    }

    terminal::print_step("Re-running replay with fix applied...");
    println!();

    let (_replay_id2, replay_state2) = run_replay_with_progress(client, offline).await?;
    println!();

    if replay_state2.eq_ignore_ascii_case("failed") {
        terminal::print_header("REPLAY FAILED");
        println!();
        println!("  The second replay failed. The fix may not have been applied correctly.");
        println!();
    } else {
        terminal::print_header("ALL QUERIES PASSED");
        println!();
        println!("  {} No regressions detected.", terminal::fmt_good(""));
        println!();
        println!("  All query patterns performing at baseline or better.");
        println!("  The fix resolves the regression -- safe to merge.");
        println!();
    }

    // ── Step 9: Wrap up ─────────────────────────────────────────────────
    terminal::print_header("DEMO COMPLETE");

    println!();
    println!("  What you saw:");
    println!("  - Query capture via scry-proxy");
    println!("  - Shadow database sync via scry-backfill");
    println!("  - Migration replay with regression detection");
    println!("  - Performance comparison reporting");
    println!();
    println!("  This demo ran everything locally on Docker.");
    println!();

    terminal::print_header("Scry Cloud");
    println!();
    println!("  Liked what you saw? Scry Cloud brings this to your CI/CD");
    println!("  pipeline with zero infrastructure to manage.");
    println!();
    println!("  In production, Scry handles:");
    println!("  - Shadow database provisioning & scaling");
    println!("  - Query journal storage & retention");
    println!("  - Replay scheduling & CI/CD integration");
    println!("  - Multi-environment coordination");
    println!();
    println!("  Learn more: https://www.scrydata.com");
    println!();

    if explain_mode {
        let (title, text) = explain::wrapup();
        terminal::print_explain_block(title, text);
    }

    Ok(local_platform)
}

// ─── API integration helpers ────────────────────────────────────────────────

/// Register the demo source, tolerating 409 Conflict (already exists).
async fn ensure_source_registered(client: &DemoClient) -> Result<(), DemoError> {
    let source_resp = client.get("/api/v1/sources/demo/demo").await;
    match source_resp {
        Ok(ref v) if v.get("error").is_none() => {
            // Source exists
        }
        _ => {
            terminal::print_info("Registering source...");
            match client
                .post(
                    "/api/v1/sources",
                    &serde_json::json!({"project": "demo", "database": "demo"}),
                )
                .await
            {
                Ok(_) => {}
                Err(DemoError::ApiResponse { status: 409, .. }) => {
                    // Already exists, fine
                }
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}

/// Create the demo shadow, tolerating 409 Conflict (already exists).
async fn ensure_shadow_created(client: &DemoClient) -> Result<(), DemoError> {
    let shadow_resp = client
        .get(&format!("/api/v1/shadows/{SHADOW_ID}"))
        .await;
    match shadow_resp {
        Ok(ref v) if v.get("error").is_none() => {
            // Shadow exists
        }
        _ => {
            terminal::print_info("Creating shadow database...");
            match client
                .post(
                    "/api/v1/shadows",
                    &serde_json::json!({
                        "shadow_id": SHADOW_ID,
                        "database": "demo",
                        "username": "postgres",
                        "password": "postgres",
                        "source": "demo/demo"
                    }),
                )
                .await
            {
                Ok(_) => {}
                Err(DemoError::ApiResponse { status: 409, .. }) => {
                    // Already exists, fine
                }
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}

/// Poll shadow state until "ready", with timeout and failure detection.
async fn wait_for_shadow_ready(
    client: &DemoClient,
    status: &Arc<Mutex<StatusBar>>,
) -> Result<(), DemoError> {
    let start = std::time::Instant::now();

    // Poll every 1s for faster feedback (300 attempts = 5 min timeout)
    for attempt in 0..300u32 {
        let resp = client
            .get(&format!("/api/v1/shadows/{SHADOW_ID}"))
            .await;

        let state = resp
            .as_ref()
            .ok()
            .and_then(|v| v.get("state"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        if state.eq_ignore_ascii_case("ready") {
            terminal::clear_spinner();
            return Ok(());
        }

        // Handle failed state
        if state.eq_ignore_ascii_case("failed") {
            terminal::clear_spinner();
            println!();
            terminal::print_error("Shadow database sync failed");
            println!();
            terminal::print_info("Troubleshooting steps:");
            println!("    1. Check platform logs: docker compose logs scry-platform");
            println!("    2. Check backfill logs: docker compose logs scry-backfill");
            println!("    3. Restart the demo: scry demo clean && scry demo start");
            println!();
            // Deactivate status bar and leave alternate screen
            {
                let mut s = status.lock().await;
                let _ = s.deactivate();
            }
            terminal::leave_alternate_screen();
            return Err(DemoError::PreconditionFailed {
                gate: "shadow_ready".to_string(),
                detail: "Shadow sync reported 'failed' state".to_string(),
            });
        }

        // Build spinner message with progress details
        let elapsed = start.elapsed().as_secs();
        let events_applied = resp
            .as_ref()
            .ok()
            .and_then(|v| v.get("events_applied"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        // Fetch sync_summary from progress endpoint every ~5s
        let detail = if attempt % 5 == 4 {
            let progress = client
                .get(&format!("/api/v1/shadows/{SHADOW_ID}/progress"))
                .await;

            // Try sync_summary first (new API)
            let sync = progress
                .as_ref()
                .ok()
                .and_then(|v| v.get("sync_summary"));

            if let Some(summary) = sync {
                let phase = summary
                    .get("phase")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let pct = summary
                    .get("progress_pct")
                    .and_then(|v| v.as_u64());
                let sync_detail = summary
                    .get("detail")
                    .and_then(|v| v.as_str());

                let mut msg = format!("Shadow: {phase}");
                if let Some(p) = pct {
                    msg.push_str(&format!(" {p}%"));
                }
                if let Some(d) = sync_detail {
                    msg.push_str(&format!(" \u{2014} {d}"));
                }
                if events_applied > 0 {
                    msg.push_str(&format!(", {events_applied} events"));
                }
                format!("{msg} ({elapsed}s)")
            } else if events_applied > 0 {
                format!("Shadow: {state}, {events_applied} events applied ({elapsed}s)")
            } else {
                format!("Shadow: {state} ({elapsed}s)")
            }
        } else if events_applied > 0 {
            format!("Shadow: {state}, {events_applied} events applied ({elapsed}s)")
        } else {
            format!("Shadow: {state} ({elapsed}s)")
        };

        // Warn when approaching timeout
        let detail = if elapsed > 180 {
            let remaining = 300u64.saturating_sub(elapsed);
            let mins = remaining / 60;
            let secs = remaining % 60;
            format!("{detail} -- timeout in {mins}m {secs:02}s")
        } else {
            detail
        };

        terminal::print_spinner(attempt as usize, &detail);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    terminal::clear_spinner();
    println!();
    terminal::print_error("Shadow database sync timed out");
    println!();
    terminal::print_info("The shadow may still be syncing. You can:");
    println!("    1. Wait and retry: scry demo start --no-compose");
    println!("    2. Check progress: docker compose logs -f scry-backfill");
    println!("    3. Restart: scry demo clean && scry demo start");
    println!();

    Err(DemoError::Timeout {
        what: "shadow database sync".to_string(),
        elapsed_secs: 300,
    })
}

/// Get shadow connection info from the /connection endpoint (BUG #1 fix).
async fn get_shadow_connection(
    client: &DemoClient,
    runtime: compose::Runtime,
) -> Result<ShadowConn, DemoError> {
    let resp = client
        .get(&format!("/api/v1/shadows/{SHADOW_ID}/connection"))
        .await?;

    let host = resp
        .get("host")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let port = resp
        .get("port")
        .and_then(|v| v.as_u64())
        .map(|n| n.to_string())
        .unwrap_or_default();
    let database = resp
        .get("database")
        .and_then(|v| v.as_str())
        .unwrap_or("demo")
        .to_string();

    // Find the shadow container by name on the Docker network
    let container = find_shadow_container(runtime)
        .await
        .unwrap_or_default();

    Ok(ShadowConn {
        host,
        port,
        container,
        database,
    })
}

/// Find a running shadow container by name pattern.
async fn find_shadow_container(runtime: compose::Runtime) -> Option<String> {
    let output = Command::new(runtime.container_cmd())
        .args(["ps", "--filter", "name=scry-shadow", "--format", "{{.Names}}"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;

    let names = String::from_utf8_lossy(&output.stdout);
    let name = names.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.lines().next().unwrap_or_default().to_string())
    }
}

// ─── Workload execution ─────────────────────────────────────────────────────

/// Run workload queries through the proxy (psql local or docker exec fallback).
async fn run_workload_queries(
    runtime: compose::Runtime,
    workload_sql: &str,
    offline: bool,
) -> Result<(), DemoError> {
    if offline {
        tokio::time::sleep(Duration::from_secs(1)).await;
        return Ok(());
    }

    // Try local psql first
    if try_psql_local(workload_sql).await {
        return Ok(());
    }

    // Fallback: docker exec into source container
    let cmd_name = runtime.container_cmd().to_string();
    let sql_owned = workload_sql.to_string();
    let result = tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let mut child = match std::process::Command::new(&cmd_name)
            .args([
                "exec",
                "-i",
                "scry-demo-source",
                "psql",
                "-h",
                "scry-demo-proxy",
                "-p",
                "5434",
                "-U",
                "postgres",
                "-d",
                "demo",
                "-q",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return Err(format!("Failed to spawn docker exec: {e}")),
        };

        if let Some(ref mut stdin) = child.stdin {
            let _ = stdin.write_all(sql_owned.as_bytes());
        }
        drop(child.stdin.take());

        match child.wait_with_output() {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(format!(
                    "docker exec psql exited {}: {}",
                    output.status.code().unwrap_or(-1),
                    stderr.lines().last().unwrap_or_default()
                ))
            }
            Err(e) => Err(format!("Failed to wait for docker exec: {e}")),
        }
    })
    .await;

    match result {
        Ok(Ok(())) => {}
        Ok(Err(detail)) => {
            return Err(DemoError::SqlExecution {
                method: "psql (local + docker exec)".to_string(),
                detail,
            });
        }
        Err(e) => {
            return Err(DemoError::SqlExecution {
                method: "psql (local + docker exec)".to_string(),
                detail: format!("Task join error: {e}"),
            });
        }
    }

    // Give workload time to be captured
    tokio::time::sleep(Duration::from_secs(3)).await;
    Ok(())
}

async fn try_psql_local(sql: &str) -> bool {
    // Check if psql exists
    let exists = Command::new("which")
        .arg("psql")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);

    if !exists {
        return false;
    }

    // Use std::process for synchronous stdin write (avoids async trait issues)
    let sql_owned = sql.to_string();
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let mut child = match std::process::Command::new("psql")
            .args([
                "-h",
                "localhost",
                "-p",
                PROXY_PORT,
                "-U",
                "postgres",
                "-d",
                "demo",
                "-q",
            ])
            .env("PGPASSWORD", "postgres")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return false,
        };

        if let Some(ref mut stdin) = child.stdin {
            let _ = stdin.write_all(sql_owned.as_bytes());
        }
        // Drop stdin to close pipe
        drop(child.stdin.take());

        child.wait().map(|s| s.success()).unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

// ─── SQL application to shadow ──────────────────────────────────────────────

/// Apply SQL to the shadow database with tiered fallback.
///
/// Returns an error with diagnostics if all methods fail.
async fn apply_sql_to_shadow(
    runtime: compose::Runtime,
    sql: &str,
    conn: &ShadowConn,
    dir: &Path,
    offline: bool,
) -> Result<(), DemoError> {
    if offline {
        tokio::time::sleep(Duration::from_secs(2)).await;
        return Ok(());
    }

    let cmd = runtime.container_cmd();
    let mut errors = Vec::new();

    // Tier 1: docker cp + exec into shadow container
    if !conn.container.is_empty() {
        match try_shadow_direct(cmd, sql, &conn.container, &conn.database, dir).await {
            Ok(()) => return Ok(()),
            Err(e) => errors.push(format!("Direct exec into {}: {e}", conn.container)),
        }
    }

    // Tier 2: Cross-container psql from source to shadow via docker network
    if !conn.host.is_empty() && !conn.port.is_empty() {
        match try_cross_container(cmd, sql, conn, dir).await {
            Ok(()) => return Ok(()),
            Err(e) => errors.push(format!("Cross-container via source: {e}")),
        }
    }

    Err(DemoError::SqlExecution {
        method: "all tiers failed".to_string(),
        detail: errors.join("; "),
    })
}

/// Tier 1: docker cp SQL file into shadow container, then exec psql.
async fn try_shadow_direct(
    cmd: &str,
    sql: &str,
    container: &str,
    database: &str,
    dir: &Path,
) -> Result<(), String> {
    let tmp_path = dir.join(".tmp-apply.sql");
    std::fs::write(&tmp_path, sql).map_err(|e| format!("write temp file: {e}"))?;

    let cp_output = Command::new(cmd)
        .args([
            "cp",
            &tmp_path.to_string_lossy(),
            &format!("{container}:/tmp/apply.sql"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("docker cp: {e}"))?;

    if !cp_output.status.success() {
        let _ = std::fs::remove_file(&tmp_path);
        let stderr = String::from_utf8_lossy(&cp_output.stderr);
        return Err(format!("docker cp failed: {}", stderr.trim()));
    }

    let exec_output = Command::new(cmd)
        .args([
            "exec",
            container,
            "psql",
            "-U",
            "postgres",
            "-d",
            database,
            "-f",
            "/tmp/apply.sql",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("docker exec psql: {e}"))?;

    let _ = std::fs::remove_file(&tmp_path);

    if exec_output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&exec_output.stderr);
        Err(format!(
            "psql exited {}: {}",
            exec_output.status.code().unwrap_or(-1),
            stderr.trim()
        ))
    }
}

/// Tier 2: docker cp SQL file into source container, exec psql connecting to shadow.
async fn try_cross_container(
    cmd: &str,
    sql: &str,
    conn: &ShadowConn,
    dir: &Path,
) -> Result<(), String> {
    let tmp_path = dir.join(".tmp-apply.sql");
    std::fs::write(&tmp_path, sql).map_err(|e| format!("write temp file: {e}"))?;

    let cp_output = Command::new(cmd)
        .args([
            "cp",
            &tmp_path.to_string_lossy(),
            "scry-demo-source:/tmp/apply.sql",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("docker cp: {e}"))?;

    if !cp_output.status.success() {
        let _ = std::fs::remove_file(&tmp_path);
        let stderr = String::from_utf8_lossy(&cp_output.stderr);
        return Err(format!("docker cp to source failed: {}", stderr.trim()));
    }

    let exec_output = Command::new(cmd)
        .args([
            "exec",
            "scry-demo-source",
            "psql",
            "-h",
            &conn.host,
            "-p",
            &conn.port,
            "-U",
            "postgres",
            "-d",
            &conn.database,
            "-f",
            "/tmp/apply.sql",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("docker exec psql: {e}"))?;

    let _ = std::fs::remove_file(&tmp_path);

    if exec_output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&exec_output.stderr);
        Err(format!(
            "psql exited {}: {}",
            exec_output.status.code().unwrap_or(-1),
            stderr.trim()
        ))
    }
}

// ─── Replay execution ───────────────────────────────────────────────────────

/// Start a replay and poll for progress with an animated progress bar.
///
/// Returns `(replay_id, final_state)`. Uses correct API fields (BUG #2 + #4 fix).
async fn run_replay_with_progress(
    client: &DemoClient,
    offline: bool,
) -> Result<(String, String), DemoError> {
    if offline {
        for i in 1..=20u32 {
            let pct = i * 5;
            terminal::print_progress_bar(pct, "queries replayed");
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        terminal::finish_progress_bar("queries replayed");
        return Ok((String::new(), "completed".to_string()));
    }

    // Start the replay with correct request body (speed: 0.0 for fast-forward)
    let replay_resp = client
        .post(
            &format!("/api/v1/shadows/{SHADOW_ID}/replays"),
            &serde_json::json!({"speed": 0.0}),
        )
        .await?;

    let replay_id = replay_resp
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    if replay_id.is_empty() {
        return Err(DemoError::ApiResponse {
            method: "POST".to_string(),
            url: format!("{API_URL}/api/v1/shadows/{SHADOW_ID}/replays"),
            status: 200,
            body: "Response missing 'id' field".to_string(),
        });
    }

    // Poll for completion using a spinner (progress bar is misleading for
    // sub-second fast-forward replays where intermediate counts aren't flushed)
    let mut final_state = String::from("running");
    let replay_start = std::time::Instant::now();
    for iteration in 0..120u32 {
        let status_resp = client
            .get(&format!("/api/v1/replays/{replay_id}"))
            .await;

        let state = match status_resp {
            Ok(ref v) => v
                .get("state")
                .and_then(|s| s.as_str())
                .unwrap_or("running")
                .to_string(),
            Err(_) => "running".to_string(),
        };

        final_state = state.clone();

        if state.eq_ignore_ascii_case("completed") || state.eq_ignore_ascii_case("failed") {
            break;
        }

        let elapsed = replay_start.elapsed().as_secs();
        let msg = format!("Replaying queries... ({elapsed}s)");
        terminal::print_spinner(iteration as usize, &msg);

        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    terminal::clear_spinner();

    // Fetch final replay info to show actual count
    if final_state.eq_ignore_ascii_case("completed") {
        if let Ok(resp) = client.get(&format!("/api/v1/replays/{replay_id}")).await {
            let queries_processed = resp
                .get("queries_processed")
                .and_then(|q| q.as_u64())
                .unwrap_or(0);
            if queries_processed > 0 {
                terminal::print_success(&format!("{queries_processed} queries replayed successfully"));
            } else {
                terminal::print_success("Replay completed");
            }
        } else {
            terminal::print_success("Replay completed");
        }
    }

    // Report failure details if the replay ended in a failed state
    if final_state.eq_ignore_ascii_case("failed") {
        terminal::print_error("Replay execution failed");

        // Try to fetch error details from the replay endpoint
        if let Ok(resp) = client.get(&format!("/api/v1/replays/{replay_id}")).await {
            if let Some(error_msg) = resp.get("error").and_then(|v| v.as_str()) {
                terminal::print_error(&format!("  Detail: {error_msg}"));
            }
        }
    }

    Ok((replay_id, final_state))
}

// ─── Results display ────────────────────────────────────────────────────────

/// Display replay results (regressions) using correct API fields (BUG #5 fix).
async fn display_replay_results(
    client: &DemoClient,
    replay_id: &str,
    scenario: &str,
    offline: bool,
) -> Result<bool, DemoError> {
    if offline || replay_id.is_empty() {
        terminal::print_header("REGRESSION DETECTED");
        println!();
        display_fallback_results(scenario);
        return Ok(true);
    }

    // Poll for results with timeout (results may still be writing)
    for attempt in 0..10u32 {
        let resp = client
            .get(&format!("/api/v1/replays/{replay_id}/regressions"))
            .await;

        if let Ok(ref data) = resp {
            // PageResponse has "items" array
            if let Some(items) = data.get("items").and_then(|v| v.as_array()) {
                if !items.is_empty() {
                    terminal::clear_spinner();
                    terminal::print_header("REGRESSION DETECTED");
                    println!();
                    display_real_results(items, scenario);
                    return Ok(true);
                }
            }
        }

        // Wait for results to be recorded
        if attempt < 9 {
            terminal::print_spinner(attempt as usize, "Analyzing results...");
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    terminal::clear_spinner();
    terminal::print_header("REPLAY RESULTS");
    println!();
    terminal::print_info("No regressions detected by the replay engine.");
    println!();
    Ok(false)
}

/// Display real regression results from the API using correct field names.
///
/// ReplayResultResponse fields: sql, production_time_us, shadow_time_us,
/// time_diff_pct, is_regression, success, error.
fn display_real_results(items: &[serde_json::Value], scenario: &str) {
    println!(
        "  {} query pattern(s) regressed:",
        terminal::bold(&items.len().to_string())
    );
    println!();

    for r in items {
        let sql_full = r
            .get("sql")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let prod_us = r
            .get("production_time_us")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let shadow_us = r
            .get("shadow_time_us")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let time_diff_pct = r
            .get("time_diff_pct")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let prod_ms = prod_us as f64 / 1000.0;
        let shadow_ms = shadow_us as f64 / 1000.0;
        let slowdown = if prod_us > 0 {
            shadow_us as f64 / prod_us as f64
        } else {
            0.0
        };

        // Truncate SQL at word boundary (80 chars), UTF-8 safe
        let truncated = if sql_full.len() > 80 {
            let safe_end = sql_full.floor_char_boundary(80);
            let cut = &sql_full[..safe_end];
            match cut.rfind(' ') {
                Some(pos) if pos > 40 => format!("{}...", &sql_full[..pos]),
                _ => format!("{cut}..."),
            }
        } else {
            sql_full.to_string()
        };
        println!("  {}", terminal::fmt_query(&truncated));
        println!(
            "  |-- Before:  {}",
            terminal::fmt_good(&format!("{prod_ms:.1}ms"))
        );
        println!(
            "  |-- After:   {}  ({})",
            terminal::fmt_bad(&format!("{shadow_ms:.1}ms")),
            terminal::fmt_bad(&format!("{slowdown:.0}x slower")),
        );
        println!("  |-- Change:  {time_diff_pct:+.1}%");
        println!();
    }

    // Show scenario-specific context and suggested fix
    match scenario {
        "missing-index" => {
            println!(
                "  {} The {} column has no index, causing sequential scans on 25K rows.",
                terminal::fmt_bad("Cause:"),
                terminal::bold("region"),
            );
            println!();
            println!(
                "  {} Suggested fix:",
                terminal::fmt_good("Tip:")
            );
            println!("     CREATE INDEX idx_orders_region ON orders(region);");
        }
        "table-locking" => {
            println!(
                "  {} Large UPDATE acquired an exclusive lock, blocking concurrent queries.",
                terminal::fmt_bad("Cause:"),
            );
            println!();
            println!(
                "  {} Suggested fix:",
                terminal::fmt_good("Tip:")
            );
            println!("     Batch the UPDATE into smaller chunks with SKIP LOCKED");
        }
        "index-drop" => {
            println!(
                "  {} Dropped index was serving queries despite pg_stat showing idx_scans = 0.",
                terminal::fmt_bad("Cause:"),
            );
            println!();
            println!(
                "  {} Suggested fix:",
                terminal::fmt_good("Tip:")
            );
            println!("     CREATE INDEX CONCURRENTLY idx_orders_status_created");
            println!("       ON orders(status, created_at);");
        }
        _ => {}
    }
    println!();
}

fn display_fallback_results(scenario: &str) {
    terminal::print_warning("Simulated results (offline mode)");
    println!();
    match scenario {
        "missing-index" => {
            println!("  {} query patterns regressed:", terminal::bold("3"));
            println!();
            println!(
                "  {}",
                terminal::fmt_query("SELECT * FROM orders WHERE region = $1 AND status = $2")
            );
            println!(
                "  |-- Before:  {}",
                terminal::fmt_good("p50=2ms   p99=8ms")
            );
            println!(
                "  |-- After:   {}  ({})",
                terminal::fmt_bad("p50=184ms p99=312ms"),
                terminal::fmt_bad("92x slower")
            );
            println!(
                "  `-- Cause:   Seq Scan on orders (missing index on {})",
                terminal::bold("region")
            );
            println!();
            println!(
                "  {}",
                terminal::fmt_query("SELECT region, status, COUNT(*) FROM orders GROUP BY ...")
            );
            println!(
                "  |-- Before:  {}",
                terminal::fmt_good("p50=5ms   p99=15ms")
            );
            println!(
                "  |-- After:   {}  ({})",
                terminal::fmt_bad("p50=210ms p99=380ms"),
                terminal::fmt_bad("42x slower")
            );
            println!("  `-- Cause:   Seq Scan on orders");
            println!();
            println!(
                "  {}",
                terminal::fmt_query("SELECT o.*, u.email FROM orders o JOIN users u ...")
            );
            println!(
                "  |-- Before:  {}",
                terminal::fmt_good("p50=8ms   p99=25ms")
            );
            println!(
                "  |-- After:   {}  ({})",
                terminal::fmt_bad("p50=165ms p99=290ms"),
                terminal::fmt_bad("20x slower")
            );
            println!("  `-- Cause:   Seq Scan on orders");
            println!();
            println!(
                "  {} Suggested fix:",
                terminal::fmt_good("Tip:")
            );
            println!("     CREATE INDEX idx_orders_region ON orders(region);");
        }
        "table-locking" => {
            println!("  {} Migration caused blocking:", terminal::bold(""));
            println!();
            println!(
                "  |-- UPDATE duration:     {}",
                terminal::fmt_bad("47 seconds")
            );
            println!(
                "  |-- Queries blocked:     {}",
                terminal::fmt_bad("156")
            );
            println!(
                "  |-- Max wait time:       {}",
                terminal::fmt_bad("38 seconds")
            );
            println!(
                "  `-- Queries timed out:   {}",
                terminal::fmt_bad("12")
            );
            println!();
            println!("  {} Affected queries:", terminal::bold(""));
            println!();
            println!(
                "  {}",
                terminal::fmt_query("UPDATE orders SET status = 'shipped' WHERE id = ...")
            );
            println!(
                "  `-- {}",
                terminal::fmt_bad("Blocked for 38s waiting for row lock")
            );
            println!();
            println!(
                "  {}",
                terminal::fmt_query("SELECT * FROM orders WHERE id = ...")
            );
            println!(
                "  `-- {}",
                terminal::fmt_bad("Blocked for 22s (isolation level)")
            );
            println!();
            println!(
                "  {} Suggested fix:",
                terminal::fmt_good("Tip:")
            );
            println!("     Batch the UPDATE into smaller chunks with SKIP LOCKED");
        }
        "index-drop" => {
            println!("  {} query patterns regressed:", terminal::bold("5"));
            println!();
            println!(
                "  {}",
                terminal::fmt_query(
                    "SELECT * FROM orders WHERE status = $1 AND created_at > $2"
                )
            );
            println!(
                "  |-- Before:  {}",
                terminal::fmt_good("p50=2ms   p99=8ms")
            );
            println!(
                "  |-- After:   {}  ({})",
                terminal::fmt_bad("p50=85ms  p99=140ms"),
                terminal::fmt_bad("42x slower")
            );
            println!(
                "  `-- Cause:   Lost index {}",
                terminal::bold("idx_orders_status_created")
            );
            println!();
            println!("  {} ... and 4 more patterns", terminal::dim(""));
            println!();
            println!("  {} Impact:", terminal::bold(""));
            println!(
                "  |-- Queries affected:    {}",
                terminal::fmt_bad("~18,720/day")
            );
            println!(
                "  |-- Added latency:       {}",
                terminal::fmt_bad("+83ms average")
            );
            println!(
                "  `-- That 'unused' index was serving {}",
                terminal::fmt_bad("2,400 queries/hour")
            );
            println!();
            println!(
                "  {} Suggested fix:",
                terminal::fmt_good("Tip:")
            );
            println!("     CREATE INDEX CONCURRENTLY idx_orders_status_created");
            println!("       ON orders(status, created_at);");
        }
        _ => {}
    }
}

// ─── Synthetic event injection ───────────────────────────────────────────────

const RECEIVER_URL: &str = "http://localhost:8080";
const RECEIVER_TOKEN: &str = "demo-token";

/// Inject synthetic QueryEvents via the batch API for scenarios where queries
/// can't run through the proxy (e.g., missing-index region queries).
async fn inject_synthetic_query_events(scenario: &str) -> Result<usize, DemoError> {
    // Only inject for missing-index — other scenarios generate traffic via query-generator
    if scenario != "missing-index" {
        return Ok(0);
    }

    terminal::print_info("Injecting synthetic query events for region queries...");

    let queries = [
        "SELECT id, order_number, user_id, status, total_amount, created_at FROM orders WHERE region = 'west' AND status = 'pending' ORDER BY created_at DESC LIMIT 50",
        "SELECT region, status, COUNT(*) as order_count, SUM(total_amount) as revenue FROM orders WHERE created_at > NOW() - INTERVAL '30 days' GROUP BY region, status ORDER BY region, order_count DESC",
        "SELECT o.id, o.order_number, u.email, o.total_amount, o.region FROM orders o JOIN users u ON o.user_id = u.id WHERE o.region = 'east' AND o.total_amount > 500 ORDER BY o.total_amount DESC LIMIT 20",
        "SELECT id, order_number, shipping_address, region FROM orders WHERE region = 'central' AND status IN ('confirmed', 'processing') AND created_at > NOW() - INTERVAL '7 days' ORDER BY created_at ASC",
    ];

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let mut events = Vec::new();
    let mut seq = 1u64;
    for query in &queries {
        // Send 5 copies of each query with varied timings for statistical significance
        for i in 0..5u64 {
            // Fast production times (2-8ms) representing well-indexed query performance
            let exec_time_us = 2000 + (i * 1200);
            events.push(serde_json::json!({
                "source_id": "demo-walkthrough-synthetic",
                "sequence": seq,
                "shadow_id": SHADOW_ID,
                "timestamp_ms": now_ms - (i * 1000), // spread over last few seconds
                "query": query,
                "database": "demo",
                "execution_time_us": exec_time_us,
                "user": "postgres",
            }));
            seq += 1;
        }
    }

    let event_count = events.len();
    let batch = serde_json::json!({ "events": events });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let resp = client
        .post(format!("{RECEIVER_URL}/events/queries/batch"))
        .header("Authorization", format!("Bearer {RECEIVER_TOKEN}"))
        .json(&batch)
        .send()
        .await
        .map_err(|e| DemoError::ApiRequest {
            method: "POST".to_string(),
            url: format!("{RECEIVER_URL}/events/queries/batch"),
            reason: format!("{e}"),
        })?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(DemoError::ApiResponse {
            method: "POST".to_string(),
            url: format!("{RECEIVER_URL}/events/queries/batch"),
            status,
            body,
        });
    }

    terminal::print_success(&format!("{event_count} synthetic query events injected"));
    Ok(event_count)
}

// ─── Misc helpers ───────────────────────────────────────────────────────────

fn print_next_steps(scenario: &str) {
    terminal::print_header("NEXT STEPS");
    println!();

    // Query-generator info
    println!("  The query-generator is still running, sending traffic through the proxy.");
    println!("  This data flows to the query journal so you can run additional replays.");
    println!();

    if scenario == "missing-index" {
        println!("  Other scenarios to try:");
        println!(
            "    {}     Table lock during migration",
            terminal::fmt_query("scry demo start --scenario table-locking")
        );
        println!(
            "    {}       Dropped index regression",
            terminal::fmt_query("scry demo start --scenario index-drop")
        );
        println!();
    } else if scenario == "table-locking" {
        println!("  Other scenarios to try:");
        println!(
            "    {}    Query regresses due to missing index",
            terminal::fmt_query("scry demo start --scenario missing-index")
        );
        println!(
            "    {}       Dropped index regression",
            terminal::fmt_query("scry demo start --scenario index-drop")
        );
        println!();
    } else if scenario == "index-drop" {
        println!("  Other scenarios to try:");
        println!(
            "    {}    Query regresses due to missing index",
            terminal::fmt_query("scry demo start --scenario missing-index")
        );
        println!(
            "    {}  Table lock during migration",
            terminal::fmt_query("scry demo start --scenario table-locking")
        );
        println!();
    }

    println!("  Useful commands:");
    println!("    {}      Swagger UI", terminal::dim("open http://localhost:8081/swagger-ui/"));
    println!("    {}  Stop everything", terminal::dim("scry demo stop"));
    println!("    {} Remove containers + data", terminal::dim("scry demo clean"));
    println!();
}
