//! `rung adopt` command - Bring an existing branch into the stack.

use anyhow::{Context, Result, bail};
use inquire::Select;
use rung_core::{BranchName, State};
use rung_git::Repository;

use crate::commands::utils;
use crate::output;
use crate::services::AdoptService;

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

    // Create service
    let service = AdoptService::new(&repo, &state);

    // Determine which branch to adopt
    let current = service.current_branch()?;
    let branch_name = branch.unwrap_or(&current);

    // Validate branch name
    let branch_name_validated = BranchName::new(branch_name).context("Invalid branch name")?;

    // Verify the branch exists
    if !service.branch_exists(branch_name) {
        bail!("Branch '{branch_name}' does not exist");
    }

    // Check if branch is already in the stack
    if service.is_in_stack(branch_name)? {
        bail!("Branch '{branch_name}' is already in the stack");
    }

    // Get the base branch for display
    let base_branch = service.default_branch()?;

    // Determine parent branch
    let parent_name = if let Some(p) = parent {
        p.to_string()
    } else {
        // Interactive selection
        let choices = service.get_parent_choices()?;

        if choices.len() == 1 {
            // Only base branch available
            base_branch
        } else {
            Select::new("Select parent branch:", choices)
                .with_help_message("The branch that this branch should be based on")
                .prompt()
                .context("Failed to get parent selection")?
        }
    };

    // Validate parent
    service.validate_parent(&parent_name)?;

    if dry_run {
        output::info(&format!(
            "Would adopt branch '{branch_name}' with parent '{parent_name}'"
        ));
        return Ok(());
    }

    // Adopt the branch
    let result = service.adopt_branch(&branch_name_validated, &parent_name)?;

    output::success(&format!(
        "Adopted branch '{}' with parent '{}'",
        result.branch_name, result.parent_name
    ));

    // Show position in stack
    if result.stack_depth > 1 {
        output::info(&format!("Stack depth: {}", result.stack_depth));
    }

    Ok(())
}
