//! Terminal output formatting utilities.

use std::sync::atomic::{AtomicBool, Ordering};

use colored::Colorize;
use rung_core::BranchState;

static QUIET_MODE: AtomicBool = AtomicBool::new(false);

/// Set quiet mode globally. Call once at startup.
pub fn set_quiet(quiet: bool) {
    QUIET_MODE.store(quiet, Ordering::Relaxed);
}

fn is_quiet() -> bool {
    QUIET_MODE.load(Ordering::Relaxed)
}

/// Print a success message (suppressed in quiet mode).
pub fn success(msg: &str) {
    if !is_quiet() {
        println!("{} {}", "✓".green(), msg);
    }
}

/// Print an error message (always prints to stderr).
pub fn error(msg: &str) {
    eprintln!("{} {}", "✗".red(), msg);
}

/// Print a warning message (always prints to stderr).
pub fn warn(msg: &str) {
    eprintln!("{} {}", "!".yellow(), msg);
}

/// Print an info message (suppressed in quiet mode).
pub fn info(msg: &str) {
    if !is_quiet() {
        println!("{} {}", "→".blue(), msg);
    }
}

/// Print essential machine-readable output (always prints).
///
/// Use for results that should be available for piping, like PR URLs.
pub fn essential(msg: &str) {
    println!("{msg}");
}

/// Get the status indicator for a branch state.
#[must_use]
pub fn state_indicator(state: &BranchState) -> String {
    match state {
        BranchState::Synced => "●".green().to_string(),
        BranchState::Diverged { commits_behind } => {
            format!("{} ({}↓)", "●".yellow(), commits_behind)
        }
        BranchState::Conflict { .. } => "●".red().to_string(),
        BranchState::Detached => "○".dimmed().to_string(),
    }
}

/// Get a colored branch name with current indicator.
#[must_use]
pub fn branch_name(name: &str, is_current: bool) -> String {
    if is_current {
        format!("{} {}", "▶".cyan(), name.cyan().bold())
    } else {
        format!("  {name}")
    }
}

/// Format a PR reference.
#[must_use]
pub fn pr_ref(number: Option<u64>) -> String {
    number.map_or_else(String::new, |n| format!("#{n}").dimmed().to_string())
}

/// Print a horizontal line (suppressed in quiet mode).
pub fn hr() {
    if !is_quiet() {
        println!("{}", "─".repeat(50).dimmed());
    }
}
