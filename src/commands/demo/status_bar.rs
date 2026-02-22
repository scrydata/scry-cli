//! Persistent status bar using ANSI scrolling regions.
//!
//! Displays a fixed status line at the bottom of the terminal while
//! content scrolls above it.

use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Minimum terminal dimensions for status bar.
const MIN_COLS: u16 = 60;
const MIN_ROWS: u16 = 15;

/// ANSI escape sequences.
mod ansi {
    pub const SAVE_CURSOR: &str = "\x1b[s";
    pub const RESTORE_CURSOR: &str = "\x1b[u";
    pub const CLEAR_LINE: &str = "\x1b[2K";
    pub const RESET_SCROLL_REGION: &str = "\x1b[r";
    pub const INVERSE_ON: &str = "\x1b[7m";
    #[allow(dead_code)]
    pub const INVERSE_OFF: &str = "\x1b[27m";
    pub const RESET: &str = "\x1b[0m";

    pub fn move_to(row: u16, col: u16) -> String {
        format!("\x1b[{};{}H", row, col)
    }

    pub fn set_scroll_region(top: u16, bottom: u16) -> String {
        format!("\x1b[{};{}r", top, bottom)
    }
}

/// Check if color output is enabled (respects NO_COLOR).
fn use_color() -> bool {
    std::env::var("NO_COLOR").is_err() && io::stdout().is_terminal()
}

/// Persistent status bar for the demo walkthrough.
pub struct StatusBar {
    enabled: bool,
    rows: u16,
    cols: u16,
    current_step: usize,
    total_steps: usize,
    step_title: String,
    start_time: Instant,
    activated: Arc<AtomicBool>,
}

impl StatusBar {
    /// Create a new status bar for `total_steps` steps.
    pub fn new(total_steps: usize) -> Self {
        let (cols, rows) = terminal_size::terminal_size()
            .map(|(w, h)| (w.0, h.0))
            .unwrap_or((80, 24));

        // Determine if status bar should be enabled
        let enabled = use_color() && io::stdout().is_terminal() && cols >= MIN_COLS && rows >= MIN_ROWS;

        if !enabled && io::stdout().is_terminal() && use_color() {
            // Terminal is too small - print a warning
            if cols < MIN_COLS || rows < MIN_ROWS {
                eprintln!(
                    "\x1b[2m  Terminal too small for status bar ({cols}x{rows}, need {MIN_COLS}x{MIN_ROWS})\x1b[0m"
                );
            }
        }

        Self {
            enabled,
            rows,
            cols,
            current_step: 0,
            total_steps,
            step_title: String::new(),
            start_time: Instant::now(),
            activated: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns whether the status bar is enabled.
    #[allow(dead_code)]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Returns an Arc<AtomicBool> that can be used to check if the status bar
    /// is currently active (for signal handlers).
    #[allow(dead_code)]
    pub fn activated_flag(&self) -> Arc<AtomicBool> {
        self.activated.clone()
    }

    /// Activate the status bar by setting up the scrolling region.
    pub fn activate(&mut self) -> io::Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let mut stdout = io::stdout();

        // Set scrolling region to exclude the bottom line
        // Rows are 1-indexed in ANSI
        let scroll_bottom = self.rows.saturating_sub(1);
        if scroll_bottom < 2 {
            return Ok(());
        }

        write!(
            stdout,
            "{}{}{}",
            ansi::set_scroll_region(1, scroll_bottom),
            ansi::move_to(1, 1),
            ansi::SAVE_CURSOR
        )?;
        stdout.flush()?;

        self.activated.store(true, Ordering::SeqCst);
        self.redraw()?;

        Ok(())
    }

    /// Deactivate the status bar and restore normal terminal behavior.
    pub fn deactivate(&mut self) -> io::Result<()> {
        if !self.activated.load(Ordering::SeqCst) {
            return Ok(());
        }

        self.activated.store(false, Ordering::SeqCst);

        let mut stdout = io::stdout();

        // Reset scrolling region
        write!(stdout, "{}", ansi::RESET_SCROLL_REGION)?;

        // Move to the status bar line and clear it
        write!(
            stdout,
            "{}{}",
            ansi::move_to(self.rows, 1),
            ansi::CLEAR_LINE
        )?;

        // Move cursor to a reasonable position
        write!(stdout, "{}", ansi::move_to(self.rows.saturating_sub(1), 1))?;

        stdout.flush()?;
        Ok(())
    }

    /// Set the current step and title.
    pub fn set_step(&mut self, step: usize, title: &str) -> io::Result<()> {
        self.current_step = step;
        self.step_title = title.to_string();

        if self.activated.load(Ordering::SeqCst) {
            self.redraw()?;
        }

        Ok(())
    }

    /// Redraw the status bar.
    fn redraw(&self) -> io::Result<()> {
        if !self.activated.load(Ordering::SeqCst) {
            return Ok(());
        }

        let mut stdout = io::stdout();

        // Save cursor, move to status line
        write!(
            stdout,
            "{}{}{}",
            ansi::SAVE_CURSOR,
            ansi::move_to(self.rows, 1),
            ansi::CLEAR_LINE
        )?;

        // Build status line content
        let step_indicator = format!("Step {}/{}", self.current_step, self.total_steps);

        // Progress bar: filled stars for completed/current, empty for remaining
        let mut progress = String::new();
        progress.push('[');
        for i in 1..=self.total_steps {
            if i <= self.current_step {
                progress.push('*');
            } else {
                progress.push('-');
            }
        }
        progress.push(']');

        // Elapsed time
        let elapsed = self.start_time.elapsed();
        let mins = elapsed.as_secs() / 60;
        let secs = elapsed.as_secs() % 60;
        let elapsed_str = format!("{mins}m {secs:02}s");

        // Truncate title if needed
        let max_title_len = self.cols.saturating_sub(45) as usize;
        let title = if self.step_title.len() > max_title_len {
            format!("{}...", &self.step_title[..max_title_len.saturating_sub(3)])
        } else {
            self.step_title.clone()
        };

        // Build the full status line
        let status = format!(
            " > {}   {}   {}   Elapsed: {}   ^C=quit",
            step_indicator, progress, title, elapsed_str
        );

        // Pad to full width
        let padded = format!("{:<width$}", status, width = self.cols as usize);

        // Write with inverse video
        write!(
            stdout,
            "{}{}{}",
            ansi::INVERSE_ON,
            &padded[..padded.len().min(self.cols as usize)],
            ansi::RESET
        )?;

        // Restore cursor
        write!(stdout, "{}", ansi::RESTORE_CURSOR)?;
        stdout.flush()?;

        Ok(())
    }

    /// Refresh the status bar (for elapsed time updates).
    #[allow(dead_code)]
    pub fn refresh(&self) -> io::Result<()> {
        self.redraw()
    }
}

impl Drop for StatusBar {
    fn drop(&mut self) {
        let _ = self.deactivate();
    }
}

/// Restore terminal state (for use in signal handlers).
#[allow(dead_code)]
pub fn restore_terminal() {
    let mut stdout = io::stdout();
    let _ = write!(stdout, "{}", ansi::RESET_SCROLL_REGION);
    let _ = write!(stdout, "{}", ansi::RESET);
    let _ = stdout.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_bar_creation() {
        let bar = StatusBar::new(6);
        assert_eq!(bar.total_steps, 6);
        assert_eq!(bar.current_step, 0);
    }

    #[test]
    fn test_ansi_sequences() {
        assert_eq!(ansi::move_to(5, 10), "\x1b[5;10H");
        assert_eq!(ansi::set_scroll_region(1, 23), "\x1b[1;23r");
    }
}
