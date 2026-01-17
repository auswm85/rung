//! `rung move` command - Interactive branch navigation.

use anyhow::{Context, Result, bail};
use inquire::Select;
use rung_core::State;
use rung_git::Repository;

use crate::output;

/// Run the move command - interactive branch picker.
pub fn run() -> Result<()> {
    let (repo, state) = open_repo_and_state()?;
    let current = repo.current_branch()?;
    let stack = state.load_stack()?;

    if stack.is_empty() {
        bail!("No branches in stack. Use `rung create <name>` to add one.");
    }

    // Build display options with visual indicators
    let options: Vec<String> = stack
        .branches
        .iter()
        .map(|b| {
            let marker = if b.name == current { " ◀" } else { "" };
            let pr = b.pr.map(|n| format!(" #{n}")).unwrap_or_default();
            format!("{}{}{}", b.name, pr, marker)
        })
        .collect();

    // Find current branch index for pre-selection
    let start_idx = stack
        .branches
        .iter()
        .position(|b| b.name == current)
        .unwrap_or(0);

    let selection = Select::new("Jump to branch:", options)
        .with_starting_cursor(start_idx)
        .with_page_size(10)
        .prompt()
        .context("Selection cancelled")?;

    // Extract branch name (everything before first space)
    let branch_name = selection
        .split_whitespace()
        .next()
        .context("Invalid selection")?;

    if branch_name == current {
        output::info("Already on this branch");
    } else {
        repo.checkout(branch_name)?;
        output::success(&format!("Switched to '{branch_name}'"));
    }

    Ok(())
}

/// Helper to open repo and state.
fn open_repo_and_state() -> Result<(Repository, State)> {
    let repo = Repository::open_current().context("Not inside a git repository")?;
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    if !state.is_initialized() {
        bail!("Rung not initialized - run `rung init` first");
    }

    Ok((repo, state))
}
