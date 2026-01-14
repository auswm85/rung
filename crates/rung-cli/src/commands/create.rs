//! `rung create` command - Create a new branch in the stack.

use anyhow::{Context, Result, bail};
use rung_core::{State, stack::StackBranch};
use rung_git::Repository;

use crate::output;

/// Run the create command.
pub fn run(name: &str) -> Result<()> {
    // Open repository
    let repo = Repository::open_current().context("Not inside a git repository")?;

    // Get state manager
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    // Ensure initialized
    if !state.is_initialized() {
        bail!("Rung not initialized - run `rung init` first");
    }

    // Get current branch (will be parent)
    let parent = repo.current_branch()?;

    // Check if branch already exists
    if repo.branch_exists(name) {
        bail!("Branch '{name}' already exists");
    }

    // Create the branch
    repo.create_branch(name)?;

    // Add to stack
    let mut stack = state.load_stack()?;
    let branch = StackBranch::new(name, Some(parent.clone()));
    stack.add_branch(branch);
    state.save_stack(&stack)?;

    // Checkout the new branch
    repo.checkout(name)?;

    output::success(&format!("Created branch '{name}' with parent '{parent}'"));

    // Show position in stack
    let ancestry = stack.ancestry(name);
    if ancestry.len() > 1 {
        output::info(&format!("Stack depth: {}", ancestry.len()));
    }

    Ok(())
}
