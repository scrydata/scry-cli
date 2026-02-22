//! Demo-specific error types for clear diagnostics.

use thiserror::Error;

/// Errors that can occur during the demo walkthrough.
#[derive(Debug, Error)]
pub enum DemoError {
    /// HTTP request failed to send (connection refused, timeout, etc.).
    #[error("API request failed: {method} {url}: {reason}")]
    ApiRequest {
        method: String,
        url: String,
        reason: String,
    },

    /// HTTP request returned a non-success status code.
    #[error("API error: {status} {method} {url}: {body}")]
    ApiResponse {
        method: String,
        url: String,
        status: u16,
        body: String,
    },

    /// A phase-gate precondition was not met.
    #[error("Precondition not met: {gate}. {detail}")]
    PreconditionFailed { gate: String, detail: String },

    /// Timed out waiting for a condition.
    #[error("Timeout after {elapsed_secs}s waiting for: {what}")]
    Timeout { what: String, elapsed_secs: u64 },

    /// A service is unhealthy or crashed.
    #[error("Service unhealthy: {service}. {detail}")]
    ServiceUnhealthy { service: String, detail: String },

    /// SQL execution failed across all available methods.
    #[error("SQL execution failed ({method}): {detail}")]
    SqlExecution { method: String, detail: String },

    /// Catch-all for other errors.
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}
