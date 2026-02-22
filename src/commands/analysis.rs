//! Analysis command implementation.

use crate::client::ApiClient;
use crate::error::CliError;
use crate::output::OutputFormat;
use clap::{Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Subcommand, Clone)]
pub enum AnalysisCommand {
    /// Generate analysis report
    Report {
        /// Replay ID
        replay_id: String,

        /// Output format
        #[arg(long, value_enum, default_value = "text")]
        format: ReportFormat,

        /// Exit with error if any regressions or failed queries
        #[arg(long)]
        strict: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ReportFormat {
    Text,
    Json,
    Junit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisReport {
    pub replay_id: Uuid,
    pub summary: ReplaySummary,
    pub regressions: Vec<Regression>,
    pub generated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaySummary {
    pub total_queries: u64,
    pub successful_queries: u64,
    pub failed_queries: u64,
    pub avg_latency_ms: f64,
    pub p50_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub p99_latency_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Regression {
    pub query_hash: String,
    pub severity: String,
    pub description: String,
    pub prod_latency_ms: f64,
    pub shadow_latency_ms: f64,
}

impl AnalysisCommand {
    pub async fn run(
        self,
        client: &ApiClient,
        _format: OutputFormat,
        _quiet: bool,
    ) -> Result<(), CliError> {
        match self {
            AnalysisCommand::Report { replay_id, format, strict } => {
                let uuid = replay_id.parse::<Uuid>()
                    .map_err(|_| CliError::Other(format!("Invalid replay ID: {}", replay_id)))?;

                let path = format!("/api/v1/replays/{}/report", uuid);
                let report: AnalysisReport = client.get(&path).await?;

                match format {
                    ReportFormat::Text => {
                        print_text_report(&report);
                    }
                    ReportFormat::Json => {
                        println!("{}", serde_json::to_string_pretty(&report)
                            .map_err(|e| CliError::Other(e.to_string()))?);
                    }
                    ReportFormat::Junit => {
                        print_junit_report(&report);
                    }
                }

                // Check strict mode
                if strict {
                    let has_regressions = !report.regressions.is_empty();
                    let has_failures = report.summary.failed_queries > 0;

                    if has_regressions || has_failures {
                        return Err(CliError::Other(format!(
                            "Strict check failed: {} regressions, {} failed queries",
                            report.regressions.len(),
                            report.summary.failed_queries
                        )));
                    }
                }

                Ok(())
            }
        }
    }
}

fn print_text_report(report: &AnalysisReport) {
    println!("=== Replay Analysis Report ===");
    println!("Replay ID: {}", report.replay_id);
    println!();
    println!("Summary:");
    println!("  Total queries:    {}", report.summary.total_queries);
    println!("  Successful:       {}", report.summary.successful_queries);
    println!("  Failed:           {}", report.summary.failed_queries);
    println!();
    println!("Latency:");
    println!("  Average:  {:.2}ms", report.summary.avg_latency_ms);
    println!("  P50:      {:.2}ms", report.summary.p50_latency_ms);
    println!("  P95:      {:.2}ms", report.summary.p95_latency_ms);
    println!("  P99:      {:.2}ms", report.summary.p99_latency_ms);
    println!();

    if report.regressions.is_empty() {
        println!("No regressions detected.");
    } else {
        println!("Regressions ({}):", report.regressions.len());
        for reg in &report.regressions {
            println!(
                "  [{}] {} - prod: {:.2}ms, shadow: {:.2}ms",
                reg.severity, reg.description, reg.prod_latency_ms, reg.shadow_latency_ms
            );
        }
    }
}

fn print_junit_report(report: &AnalysisReport) {
    println!(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    println!(
        r#"<testsuite name="scry-replay" tests="{}" failures="{}" errors="0">"#,
        report.summary.total_queries,
        report.regressions.len()
    );

    // Successful test case
    println!(
        r#"  <testcase name="replay-{}" classname="scry.replay">"#,
        report.replay_id
    );

    if !report.regressions.is_empty() {
        for reg in &report.regressions {
            println!(
                r#"    <failure message="{}" type="regression">{}</failure>"#,
                xml_escape(&reg.description),
                xml_escape(&format!(
                    "Severity: {}, Prod: {:.2}ms, Shadow: {:.2}ms",
                    reg.severity, reg.prod_latency_ms, reg.shadow_latency_ms
                ))
            );
        }
    }

    println!("  </testcase>");
    println!("</testsuite>");
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
