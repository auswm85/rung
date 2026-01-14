//! Terminal output formatting utilities.

use colored::Colorize;
use rung_core::BranchState;

/// Print a success message.
pub fn success(msg: &str) {
    println!("{} {}", "✓".green(), msg);
}

/// Print an error message.
pub fn error(msg: &str) {
    eprintln!("{} {}", "✗".red(), msg);
}

/// Print a warning message.
pub fn warn(msg: &str) {
    println!("{} {}", "!".yellow(), msg);
}

/// Print an info message.
pub fn info(msg: &str) {
    println!("{} {}", "→".blue(), msg);
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
        format!("  {}", name)
    }
}

/// Format a PR reference.
#[must_use]
pub fn pr_ref(number: Option<u64>) -> String {
    match number {
        Some(n) => format!("#{}", n).dimmed().to_string(),
        None => "".to_string(),
    }
}

/// Print a horizontal line.
pub fn hr() {
    println!("{}", "─".repeat(50).dimmed());
}
