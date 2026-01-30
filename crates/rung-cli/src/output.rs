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

/// Print the detached HEAD error message with guidance (always to stderr).
pub fn error_detached_head() {
    error("Cannot run this command in detached HEAD state.");
    eprintln!();
    eprintln!("You are not on any branch. To fix this:");
    eprintln!("  1. Create a new branch: git checkout -b <branch-name>");
    eprintln!("  2. Or return to an existing branch: git checkout <branch-name>");
    eprintln!();
    eprintln!("Run `rung status` after switching to a branch.");
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

/// Print a detail line without prefix (suppressed in quiet mode).
///
/// Use for indented detail lines that accompany info or warn messages.
pub fn detail(msg: &str) {
    if !is_quiet() {
        println!("{msg}");
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

/// Status of a pull request for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrStatus {
    /// PR is open and ready for review.
    Open,
    /// PR is a draft.
    Draft,
    /// PR has been merged.
    Merged,
    /// PR was closed without merging.
    Closed,
}

/// Format a PR reference.
#[must_use]
pub fn pr_ref(number: Option<u64>, status: Option<PrStatus>) -> String {
    let Some(n) = number else {
        return String::new();
    };

    let text = format!("#{n}");

    match status {
        Some(PrStatus::Open) => text,                       // Default/White
        Some(PrStatus::Draft) => text.yellow().to_string(), // Yellow
        Some(PrStatus::Merged) => text.green().to_string(), // Green
        Some(PrStatus::Closed) => text.red().to_string(),   // Red
        None => text.dimmed().to_string(),                  // Dimmed (Unknown state)
    }
}

/// Print a horizontal line (suppressed in quiet mode).
pub fn hr() {
    if !is_quiet() {
        println!("{}", "─".repeat(50).dimmed());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colored::Colorize;
    use rung_core::BranchState;

    #[test]
    fn pr_ref_colors_match_status() {
        colored::control::set_override(true);

        let text = "#42";
        assert_eq!(pr_ref(None, Some(PrStatus::Open)), "");
        assert_eq!(pr_ref(Some(42), Some(PrStatus::Open)), text);
        assert_eq!(
            pr_ref(Some(42), Some(PrStatus::Draft)),
            text.yellow().to_string()
        );
        assert_eq!(
            pr_ref(Some(42), Some(PrStatus::Merged)),
            text.green().to_string()
        );
        assert_eq!(
            pr_ref(Some(42), Some(PrStatus::Closed)),
            text.red().to_string()
        );
        assert_eq!(pr_ref(Some(42), None), text.dimmed().to_string());

        colored::control::set_override(false);
    }

    #[test]
    fn test_state_indicator_synced() {
        let indicator = state_indicator(&BranchState::Synced);
        // Contains a bullet point character
        assert!(!indicator.is_empty());
    }

    #[test]
    fn test_state_indicator_diverged() {
        let indicator = state_indicator(&BranchState::Diverged { commits_behind: 3 });
        assert!(indicator.contains('3'));
        assert!(indicator.contains('↓'));
    }

    #[test]
    fn test_state_indicator_conflict() {
        let indicator = state_indicator(&BranchState::Conflict {
            files: vec!["test.rs".to_string()],
        });
        assert!(!indicator.is_empty());
    }

    #[test]
    fn test_state_indicator_detached() {
        let indicator = state_indicator(&BranchState::Detached);
        assert!(!indicator.is_empty());
    }

    #[test]
    fn test_branch_name_current() {
        let name = branch_name("feature/test", true);
        assert!(name.contains("feature/test"));
        assert!(name.contains('▶'));
    }

    #[test]
    fn test_branch_name_not_current() {
        let name = branch_name("feature/test", false);
        assert!(name.contains("feature/test"));
        assert!(!name.contains('▶'));
    }

    #[test]
    fn test_pr_ref_some() {
        let pr = pr_ref(Some(123), None);
        assert!(pr.contains("123"));
        assert!(pr.contains('#'));
    }

    #[test]
    fn test_pr_ref_none() {
        let pr = pr_ref(None, None);
        assert!(pr.is_empty());
    }

    #[test]
    fn test_quiet_mode_default() {
        // Reset to default state
        set_quiet(false);
        assert!(!is_quiet());
    }

    #[test]
    fn test_quiet_mode_enabled() {
        set_quiet(true);
        assert!(is_quiet());
        // Reset
        set_quiet(false);
    }
}
