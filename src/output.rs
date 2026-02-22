//! Output formatting for CLI commands.

use serde::Serialize;
use tabled::{Table, Tabled};

/// Output format.
#[derive(Debug, Clone, Copy, Default)]
pub enum OutputFormat {
    #[default]
    Table,
    Json,
}

/// Print a single item.
pub fn print_item<T: Serialize>(item: &T, format: OutputFormat) -> anyhow::Result<()> {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(item)?);
        }
        OutputFormat::Table => {
            println!("{}", serde_json::to_string_pretty(item)?);
        }
    }
    Ok(())
}

/// Print a list as a table or JSON.
pub fn print_table<T: Serialize + Tabled>(items: &[T], format: OutputFormat) -> anyhow::Result<()> {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(items)?);
        }
        OutputFormat::Table => {
            if items.is_empty() {
                println!("No items found.");
            } else {
                let table = Table::new(items).to_string();
                println!("{}", table);
            }
        }
    }
    Ok(())
}

/// Print a success message.
pub fn print_success(message: &str, quiet: bool) {
    if !quiet {
        println!("{}", message);
    }
}

/// Print an error message to stderr.
pub fn print_error(message: &str) {
    eprintln!("Error: {}", message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_format_default() {
        let format = OutputFormat::default();
        assert!(matches!(format, OutputFormat::Table));
    }
}
