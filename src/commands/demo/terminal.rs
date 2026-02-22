//! Terminal output helpers for the demo walkthrough.
//!
//! Respects `NO_COLOR` environment variable and non-TTY output.

use std::io::{self, IsTerminal, Read, Write};

/// Whether color output is enabled.
fn use_color() -> bool {
    std::env::var("NO_COLOR").is_err() && io::stdout().is_terminal()
}

/// Whether we're running in an interactive terminal.
pub fn is_interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// Clear the screen and move cursor to top-left.
#[allow(dead_code)]
pub fn clear_screen() {
    if io::stdout().is_terminal() {
        print!("\x1b[2J\x1b[H");
        let _ = io::stdout().flush();
    }
}

/// Enter alternate screen buffer (isolates from existing terminal content).
///
/// Uses the same technique as vim, less, and tmux to completely isolate
/// the demo from the terminal's existing content. The original terminal
/// state is saved and can be restored with `leave_alternate_screen()`.
pub fn enter_alternate_screen() {
    if io::stdout().is_terminal() {
        print!("\x1b[?1049h"); // Switch to alternate screen buffer
        print!("\x1b[2J\x1b[H"); // Clear and home cursor
        let _ = io::stdout().flush();
    }
}

/// Leave alternate screen buffer (restores original terminal content).
///
/// Switches back to the normal screen buffer, restoring whatever was
/// on the terminal before `enter_alternate_screen()` was called.
pub fn leave_alternate_screen() {
    if io::stdout().is_terminal() {
        print!("\x1b[?1049l");
        let _ = io::stdout().flush();
    }
}

// ANSI escape helpers

pub fn bold(text: &str) -> String {
    if use_color() {
        format!("\x1b[1m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

pub fn dim(text: &str) -> String {
    if use_color() {
        format!("\x1b[2m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

pub fn green(text: &str) -> String {
    if use_color() {
        format!("\x1b[0;32m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

pub fn red(text: &str) -> String {
    if use_color() {
        format!("\x1b[0;31m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

pub fn yellow(text: &str) -> String {
    if use_color() {
        format!("\x1b[1;33m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn cyan(text: &str) -> String {
    if use_color() {
        format!("\x1b[0;36m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn blue(text: &str) -> String {
    if use_color() {
        format!("\x1b[0;34m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Print the demo banner with ASCII owl logo.
pub fn print_banner(scenario: &str) {
    println!();
    let border = blue("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  {border}");
    println!("     {}    {}", dim(",_,"), bold("Scry Demo"));
    println!(
        "    {}   {}",
        dim("(O,O)"),
        dim("Catch Migration Regressions Before Production")
    );
    println!("    {}", dim("(   )"));
    println!("    {}", dim("-\"-\"-"));
    println!("  {border}");
    println!();
    println!("  Scenario: {}", bold(scenario));
    println!();
}

/// Print a section header.
pub fn print_header(title: &str) {
    let border = cyan("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();
    println!("{border}");
    println!("{}", bold(title));
    println!("{border}");
}

/// Print a step indicator.
pub fn print_step(text: &str) {
    println!();
    println!("{}", yellow(&format!(">> {text}")));
}

/// Print a success message.
pub fn print_success(text: &str) {
    println!("  {} {text}", green("ok"));
}

/// Print an info/dim message.
pub fn print_info(text: &str) {
    println!("  {}", dim(text));
}

/// Print an error message.
pub fn print_error(text: &str) {
    println!("  {} {text}", red("error:"));
}

/// Print a warning/note message.
pub fn print_warning(text: &str) {
    println!("  {} {text}", yellow("note:"));
}

/// Print a green-highlighted value.
pub fn fmt_good(text: &str) -> String {
    green(text)
}

/// Print a red-highlighted value.
pub fn fmt_bad(text: &str) -> String {
    red(text)
}

/// Print a cyan-highlighted value.
pub fn fmt_query(text: &str) -> String {
    cyan(text)
}

/// Show a progress indicator for steps.
///
/// Renders: `Step 2 of 6: Syncing Shadow Database`
/// With: `●─●─○─○─○─○`
pub fn show_progress(step: usize, total: usize, title: &str) {
    let divider = dim("────────────────────────────────────────────────────────────");
    println!();
    println!("{divider}");

    let mut dots = String::new();
    for i in 1..=total {
        if i < step {
            dots.push('*');
        } else if i == step {
            dots.push_str(&green("*"));
        } else {
            dots.push('o');
        }
        if i < total {
            dots.push('-');
        }
    }

    println!("  Step {step} of {total}: {}", bold(title));
    println!("  {dots}");
    println!("{divider}");
}

/// Wait for the user to press Enter.
pub fn wait_for_enter() {
    if !is_interactive() {
        return;
    }
    println!();
    println!("{}", dim("Press ENTER to continue..."));
    let _ = io::stdout().flush();
    // Read one byte at a time until newline
    let mut buf = [0u8; 1];
    loop {
        match io::stdin().read(&mut buf) {
            Ok(0) => break,
            Ok(_) => {
                if buf[0] == b'\n' {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

/// Print a service health line.
pub fn print_service_status(name: &str, healthy: bool) {
    let status = if healthy {
        green("healthy")
    } else {
        red("not running")
    };
    println!("  {:<30} {status}", name);
}

/// Print a progress bar with percentage.
pub fn print_progress_bar(pct: u32, suffix: &str) {
    let clamped = pct.min(100);
    let filled = (clamped / 5) as usize;
    let empty = 20 - filled;
    let bar_filled: String = "#".repeat(filled);
    let bar_empty: String = "-".repeat(empty);
    print!("\r  [{bar_filled}{bar_empty}] {clamped}% {suffix}");
    let _ = io::stdout().flush();
}

/// Print a final (100%) progress bar and newline.
pub fn finish_progress_bar(suffix: &str) {
    let bar: String = "#".repeat(20);
    print!("\r  [{bar}] 100% {suffix}          ");
    println!();
}

/// Display SQL in a bordered box (like demo.sh).
pub fn print_sql_box(sql: &str) {
    let top = cyan("  ┌──────────────────────────────────────────────────────┐");
    let bottom = cyan("  └──────────────────────────────────────────────────────┘");
    println!("{top}");
    let line_count = sql.lines().count();
    for line in sql.lines().take(20) {
        println!("  │ {line}");
    }
    if line_count > 20 {
        println!("  │ {}", dim(&format!("... ({} more lines)", line_count - 20)));
    }
    println!("{bottom}");
}

/// Print an explanation block (for --explain mode).
pub fn print_explain_block(title: &str, text: &str) {
    let border = blue("┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈");
    println!();
    println!("  {border}");
    println!("  {}", bold(&format!("How it works: {title}")));
    println!();
    for line in text.lines() {
        println!("  {line}");
    }
    println!("  {border}");
}

/// Spinner characters for polling loops.
const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Print a spinner frame.
pub fn print_spinner(frame: usize, message: &str) {
    let ch = SPINNER[frame % SPINNER.len()];
    print!("\r  {ch} {}", dim(message));
    let _ = io::stdout().flush();
}

/// Clear the current spinner line.
pub fn clear_spinner() {
    print!("\r{:60}\r", "");
    let _ = io::stdout().flush();
}

/// Waiting animation for service startup.
#[allow(dead_code)]
pub fn print_service_waiting(name: &str) {
    print!("  Starting {:<30}", format!("{name}..."));
    let _ = io::stdout().flush();
}

/// Mark a waiting service as ready.
#[allow(dead_code)]
pub fn print_service_ready() {
    println!(" {}", green("ready"));
}

/// Mark a waiting service as timed out.
#[allow(dead_code)]
pub fn print_service_timeout() {
    println!(" {}", red("timeout"));
}

/// Print architecture overview while services start.
pub fn print_architecture_overview() {
    println!("  The demo runs these services locally:");
    println!();
    println!("  {} PostgreSQL source database (simulates production)", dim("•"));
    println!("    └─ Pre-loaded with 100K orders for realistic testing");
    println!();
    println!("  {} NATS JetStream (event journals)", dim("•"));
    println!("    └─ Retains DDL, CDC, and query events by age/size");
    println!();
    println!("  {} Scry Platform (replay engine)", dim("•"));
    println!("    └─ Manages shadows, runs replays, detects regressions");
    println!();
    println!("  {} Scry Backfill (CDC replication)", dim("•"));
    println!("    └─ Syncs source schema/data to shadow via logical replication");
    println!();
    println!("  {} Scry Proxy (query capture)", dim("•"));
    println!("    └─ Transparent proxy capturing SQL + bind params");
    println!();
    println!(
        "  {}",
        dim("In production, the proxy sits between your app and database.")
    );
    println!(
        "  {}",
        dim("Captured queries are replayed against shadows with migrations applied.")
    );
}
