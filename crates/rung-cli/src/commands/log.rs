//! `rung log` command - show commits between the base branch and HEAD.

use anyhow::{Result, bail};

use super::utils::open_repo_and_state;
use crate::output;
use crate::services::{CommitInfo, LogResult, LogService};

/// Run the log command.
pub fn run(json: bool) -> Result<()> {
    let (repo, state) = open_repo_and_state()?;

    // Create service
    let service = LogService::new(&repo, &state);

    let stack = service.load_stack()?;
    if stack.is_empty() {
        bail!("No branches in stack. Use `rung create <name>` to add one.");
    }

    let current = service.current_branch()?;
    let log_result = service.get_branch_log(&current)?;

    if log_result.commits.is_empty() && !json {
        output::warn("Current branch has no commits");
        return Ok(());
    }

    if json {
        print_json(&log_result)?;
    } else {
        print_commits(&log_result.commits);
    }

    Ok(())
}

/// Print commits in human-readable format.
fn print_commits(commits: &[CommitInfo]) {
    for commit in commits {
        let msg = format!(
            "{:<10} {:<25}     {}",
            commit.hash, commit.message, commit.author
        );
        output::info(&msg);
    }
}

/// Print log result as JSON.
fn print_json(log_result: &LogResult) -> Result<()> {
    let json_output = serde_json::to_string_pretty(log_result)?;
    println!("{json_output}");
    Ok(())
}
