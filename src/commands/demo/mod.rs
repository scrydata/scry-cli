//! Demo subcommand — guided walkthrough of scry-platform capabilities.

pub mod assets;
pub mod client;
pub mod compose;
pub mod error;
pub mod explain;
pub mod gates;
pub mod status_bar;
pub mod terminal;
pub mod walkthrough;

use clap::Subcommand;
use std::path::PathBuf;

/// Default directory for extracted demo assets.
const DEFAULT_DIR: &str = "./scry-demo";

/// Demo subcommands.
#[derive(Subcommand, Clone, Debug)]
pub enum DemoCommand {
    /// Extract demo assets to disk and inspect them
    Init {
        /// Directory to extract assets into
        #[arg(long, default_value = DEFAULT_DIR)]
        dir: PathBuf,
    },

    /// Start the demo (auto-runs init if needed)
    Start {
        /// Scenario to run
        #[arg(long, default_value = "missing-index")]
        scenario: String,

        /// Show architectural explanations between steps
        #[arg(long)]
        explain: bool,

        /// Skip starting Docker/Podman Compose (use existing services)
        #[arg(long)]
        no_compose: bool,

        /// Directory containing demo assets
        #[arg(long, default_value = DEFAULT_DIR)]
        dir: PathBuf,

        /// Automatically clean up containers after successful completion
        #[arg(long)]
        cleanup_after: bool,

        /// Run scry-platform locally instead of in container (for development)
        #[arg(long)]
        dev: bool,

        /// Show detailed API request/response logging
        #[arg(long)]
        verbose: bool,

        /// Use hardcoded demo data (skip compose and API calls)
        #[arg(long)]
        offline: bool,
    },

    /// Stop demo services (preserves volumes)
    Stop {
        /// Directory containing demo assets
        #[arg(long, default_value = DEFAULT_DIR)]
        dir: PathBuf,
    },

    /// Stop services and remove volumes
    Clean {
        /// Directory containing demo assets
        #[arg(long, default_value = DEFAULT_DIR)]
        dir: PathBuf,

        /// Also remove the extracted demo directory
        #[arg(long)]
        remove_dir: bool,
    },

    /// Show running service health
    Status {
        /// Directory containing demo assets
        #[arg(long, default_value = DEFAULT_DIR)]
        dir: PathBuf,
    },

    /// Wait for demo services to become healthy
    WaitServices {
        /// Timeout in seconds
        #[arg(long, default_value = "120")]
        timeout: u64,
    },

    /// Build demo container images locally (instead of pulling from GHCR)
    Build,

    /// List available demo scenarios
    ListScenarios,
}

impl DemoCommand {
    pub async fn run(self, quiet: bool) -> Result<(), anyhow::Error> {
        match self {
            DemoCommand::Init { dir } => {
                assets::extract_to(&dir)?;
                compose::generate_override(&dir, false, None).await?;
                if !quiet {
                    terminal::print_success(&format!(
                        "Demo assets extracted to {}",
                        dir.display()
                    ));
                    terminal::print_info("Inspect the files, then run: scry demo start");
                }
                Ok(())
            }

            DemoCommand::Start {
                scenario,
                explain,
                no_compose,
                dir,
                cleanup_after,
                dev,
                verbose,
                offline,
            } => {
                // Validate scenario
                if !assets::scenario_exists(&scenario) {
                    let available = assets::list_scenarios()
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(anyhow::anyhow!(
                        "Unknown scenario: {scenario}. Available: {available}"
                    ));
                }

                // Auto-init if directory doesn't exist
                if !dir.exists() {
                    assets::extract_to(&dir)?;
                    compose::generate_override(&dir, dev, Some(&scenario)).await?;
                }

                walkthrough::run(&dir, &scenario, explain, no_compose, quiet, cleanup_after, dev, verbose, offline).await
            }

            DemoCommand::Stop { dir } => {
                compose::down(&dir, false).await?;
                if !quiet {
                    terminal::print_success("Demo services stopped (volumes preserved)");
                    terminal::print_info("Run 'scry demo start --no-compose' to restart");
                }
                Ok(())
            }

            DemoCommand::Clean { dir, remove_dir } => {
                compose::down(&dir, true).await?;
                if remove_dir && dir.exists() {
                    std::fs::remove_dir_all(&dir)?;
                    if !quiet {
                        terminal::print_success(&format!(
                            "Demo cleaned and {} removed",
                            dir.display()
                        ));
                    }
                } else if !quiet {
                    terminal::print_success("Demo cleaned (volumes removed)");
                }
                Ok(())
            }

            DemoCommand::Status { dir } => {
                compose::status(&dir).await
            }

            DemoCommand::WaitServices { timeout } => {
                compose::wait_for_services(timeout).await
            }

            DemoCommand::Build => {
                compose::build_images(quiet).await
            }

            DemoCommand::ListScenarios => {
                let scenarios = assets::list_scenarios();
                if quiet {
                    for s in &scenarios {
                        println!("{}", s.name);
                    }
                } else {
                    println!();
                    terminal::print_header("Available Scenarios");
                    println!();
                    for s in &scenarios {
                        println!(
                            "  {:<16} {}",
                            terminal::bold(&s.name),
                            s.description
                        );
                    }
                    println!();
                    println!(
                        "  Run: {}",
                        terminal::dim("scry demo start --scenario <name>")
                    );
                    println!();
                }
                Ok(())
            }
        }
    }
}
