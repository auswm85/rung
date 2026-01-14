//! `rung submit` command - Push branches and create/update PRs.

use anyhow::{Context, Result, bail};
use rung_core::State;
use rung_git::Repository;

use crate::output;

/// Run the submit command.
pub fn run(draft: bool, _force: bool) -> Result<()> {
    // Open repository
    let repo = Repository::open_current().context("Not inside a git repository")?;

    // Get state manager
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    // Ensure initialized
    if !state.is_initialized() {
        bail!("Rung not initialized - run `rung init` first");
    }

    // Load stack
    let stack = state.load_stack()?;

    if stack.is_empty() {
        output::info("No branches in stack - nothing to submit");
        return Ok(());
    }

    // TODO: Implement actual submit
    output::warn("Submit not yet fully implemented");
    output::info("Would submit the following branches:");

    for branch in &stack.branches {
        let pr_status = branch.pr.map_or_else(
            || {
                if draft {
                    "create draft PR".to_string()
                } else {
                    "create PR".to_string()
                }
            },
            |n| format!("update PR #{n}"),
        );
        println!("  → {} ({pr_status})", branch.name);
    }

    Ok(())
}
