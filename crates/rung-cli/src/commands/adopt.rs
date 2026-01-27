//! `rung adopt` command - Bring an existing branch into the stack.

use anyhow::{Context, Result, bail};
use inquire::Select;
use rung_core::{BranchName, State, stack::StackBranch};
use rung_git::Repository;

use crate::commands::utils;
use crate::output;

/// Run the adopt command.
pub fn run(branch: Option<&str>, parent: Option<&str>, dry_run: bool) -> Result<()> {
    // Open repository
    let repo = Repository::open_current().context("Not inside a git repository")?;

    // Get state manager
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    // Ensure initialized
    if !state.is_initialized() {
        bail!("Rung not initialized - run `rung init` first");
    }

    // Ensure on branch (not detached HEAD)
    utils::ensure_on_branch(&repo)?;

    // Determine which branch to adopt
    let current = repo.current_branch()?;
    let branch_name = branch.unwrap_or(&current);

    // Validate branch name
    let branch_name_validated = BranchName::new(branch_name).context("Invalid branch name")?;

    // Verify the branch exists
    if !repo.branch_exists(branch_name) {
        bail!("Branch '{branch_name}' does not exist");
    }

    // Load stack
    let mut stack = state.load_stack()?;

    // Check if branch is already in the stack
    if stack.find_branch(branch_name).is_some() {
        bail!("Branch '{branch_name}' is already in the stack");
    }

    // Get the base branch for validation
    let base_branch = state.default_branch()?;

    // Determine parent branch
    let parent_name = if let Some(p) = parent {
        p.to_string()
    } else {
        // Interactive selection
        let mut choices: Vec<String> = vec![base_branch.clone()];
        for b in &stack.branches {
            choices.push(b.name.to_string());
        }

        if choices.len() == 1 {
            // Only base branch available
            base_branch.clone()
        } else {
            Select::new("Select parent branch:", choices)
                .with_help_message("The branch that this branch should be based on")
                .prompt()
                .context("Failed to get parent selection")?
        }
    };

    // Validate parent exists (either base branch or in stack)
    let parent_is_base = parent_name == base_branch;
    let parent_in_stack = stack.find_branch(&parent_name).is_some();

    if !parent_is_base && !parent_in_stack {
        // Check if parent exists as a git branch at all
        if !repo.branch_exists(&parent_name) {
            bail!("Parent branch '{parent_name}' does not exist");
        }
        bail!(
            "Parent branch '{parent_name}' is not in the stack. \
             Add it first with `rung adopt {parent_name}` or use the base branch '{base_branch}'"
        );
    }

    // Check for cycles (branch can't be its own ancestor)
    // This is only relevant if the branch already exists in git history
    // For adopt, we're adding a new entry, so cycles aren't possible
    // unless we're trying to adopt a branch as a child of itself

    let parent_branch = if parent_is_base {
        None
    } else {
        Some(BranchName::new(&parent_name).context("Invalid parent branch name")?)
    };

    if dry_run {
        output::info(&format!(
            "Would adopt branch '{branch_name}' with parent '{parent_name}'"
        ));
        return Ok(());
    }

    // Add to stack
    let branch = StackBranch::new(branch_name_validated, parent_branch);
    stack.add_branch(branch);
    state.save_stack(&stack)?;

    output::success(&format!(
        "Adopted branch '{branch_name}' with parent '{parent_name}'"
    ));

    // Show position in stack
    let ancestry = stack.ancestry(branch_name);
    if ancestry.len() > 1 {
        output::info(&format!("Stack depth: {}", ancestry.len()));
    }

    Ok(())
}
