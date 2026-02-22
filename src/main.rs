//! scry-cli: Command-line interface for scry-platform.

mod client;
mod commands;
mod config;
mod dvcs;
mod error;
mod migration_detector;
mod output;
mod repo_config;
mod resolve;
mod tunnel;

use clap::Parser;
use commands::analysis::AnalysisCommand;
use commands::checkpoints::CheckpointsCommand;
use commands::ci::CiCommand;
use commands::config_cmd::ConfigCommand;
use commands::demo::DemoCommand;
use commands::health::HealthCommand;
use commands::job::JobCommand;
use commands::replay::ReplayCommand;
use commands::shadow::ShadowCommand;
use commands::source::SourceCommand;
use config::Config;
use error::CliError;
use output::OutputFormat;

#[derive(Parser)]
#[command(name = "scry")]
#[command(about = "CLI for scry-platform API", version)]
struct Cli {
    /// API endpoint URL
    #[arg(long, env = "SCRY_API_URL", global = true)]
    api_url: Option<String>,

    /// API authentication token
    #[arg(long, env = "SCRY_API_TOKEN", global = true)]
    token: Option<String>,

    /// Profile to use from config file
    #[arg(long, global = true)]
    profile: Option<String>,

    /// Output as JSON instead of table
    #[arg(long, global = true)]
    json: bool,

    /// Suppress non-essential output
    #[arg(short, long, global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Manage CLI configuration
    #[command(subcommand)]
    Config(ConfigCommand),

    /// Check service health
    Health(HealthCommand),

    /// Manage sources
    #[command(subcommand)]
    Source(SourceCommand),

    /// Manage shadow instances
    #[command(subcommand)]
    Shadow(ShadowCommand),

    /// Manage shadow checkpoints
    #[command(subcommand)]
    Checkpoints(CheckpointsCommand),

    /// Run interactive demo walkthrough
    #[command(subcommand)]
    Demo(DemoCommand),

    /// CI/CD pipeline commands
    #[command(subcommand)]
    Ci(CiCommand),

    /// Manage replays
    #[command(subcommand)]
    Replay(ReplayCommand),

    /// Generate analysis reports
    #[command(subcommand)]
    Analysis(AnalysisCommand),

    /// Manage jobs
    #[command(subcommand)]
    Job(JobCommand),

    /// Show version information
    Version,
}

/// Resolve the API client from CLI args, env, and config.
fn resolve_client(cli: &Cli, config: &Config) -> Result<client::ApiClient, CliError> {
    let api_url = cli.api_url.clone()
        .or_else(|| config.effective_api_url(cli.profile.as_deref()))
        .ok_or_else(|| CliError::MissingConfig("API URL not configured. Set SCRY_API_URL or run 'scry config set'".to_string()))?;

    let token = cli.token.clone()
        .or_else(|| config.effective_token(cli.profile.as_deref()))
        .ok_or_else(|| CliError::MissingConfig("API token not configured. Set SCRY_API_TOKEN or run 'scry config set'".to_string()))?;

    client::ApiClient::new(api_url, token)
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let config = Config::load().unwrap_or_default();
    let format = if cli.json { OutputFormat::Json } else { OutputFormat::Table };

    let result = match &cli.command {
        Commands::Config(cmd) => {
            // Config commands don't need API client
            cmd.clone().run(format)
        }
        Commands::Demo(cmd) => {
            // Demo commands don't need API client — they use their own local client
            cmd.clone().run(cli.quiet).await
        }
        Commands::Health(cmd) => {
            // Health check can work without full auth for basic connectivity
            let client_result = resolve_client(&cli, &config).or_else(|_| {
                // Create minimal client for health check
                client::ApiClient::new(
                    cli.api_url.clone().unwrap_or_else(|| "http://localhost:8081".to_string()),
                    "".to_string(),
                )
            });
            match client_result {
                Ok(client) => cmd.clone().run(&client, format, cli.quiet).await.map_err(Into::into),
                Err(e) => Err(e.into()),
            }
        }
        Commands::Source(cmd) => {
            match resolve_client(&cli, &config) {
                Ok(client) => cmd.clone().run(&client, format, cli.quiet).await.map_err(Into::into),
                Err(e) => Err(e.into()),
            }
        }
        Commands::Shadow(cmd) => {
            match resolve_client(&cli, &config) {
                Ok(client) => cmd.clone().run(&client, format, cli.quiet).await.map_err(Into::into),
                Err(e) => Err(e.into()),
            }
        }
        Commands::Checkpoints(cmd) => {
            match resolve_client(&cli, &config) {
                Ok(client) => cmd.clone().run(&client, format, cli.quiet).await.map_err(Into::into),
                Err(e) => Err(e.into()),
            }
        }
        Commands::Ci(cmd) => {
            // DetectMigrations is a local git operation that doesn't need an API client
            if let commands::ci::CiCommand::DetectMigrations { .. } = cmd {
                cmd.clone().run_local(format).map_err(Into::into)
            } else {
                match resolve_client(&cli, &config) {
                    Ok(client) => cmd.clone().run(&client, format, cli.quiet).await.map_err(Into::into),
                    Err(e) => Err(e.into()),
                }
            }
        }
        Commands::Replay(cmd) => {
            match resolve_client(&cli, &config) {
                Ok(client) => cmd.clone().run(&client, format, cli.quiet).await.map_err(Into::into),
                Err(e) => Err(e.into()),
            }
        }
        Commands::Analysis(cmd) => {
            match resolve_client(&cli, &config) {
                Ok(client) => cmd.clone().run(&client, format, cli.quiet).await.map_err(Into::into),
                Err(e) => Err(e.into()),
            }
        }
        Commands::Job(cmd) => {
            match resolve_client(&cli, &config) {
                Ok(client) => cmd.clone().run(&client, format, cli.quiet).await.map_err(Into::into),
                Err(e) => Err(e.into()),
            }
        }
        Commands::Version => {
            println!("scry-cli v{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    };

    if let Err(e) = result {
        output::print_error(&e.to_string());
        let exit_code = if let Some(cli_err) = e.downcast_ref::<CliError>() {
            cli_err.exit_code()
        } else {
            1
        };
        std::process::exit(exit_code);
    }
}
