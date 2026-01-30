//! `rung create` command - Create a new branch in the stack.

use anyhow::{Context, Result, bail};
use rung_core::{BranchName, State, slugify};
use rung_git::Repository;

use crate::commands::utils;
use crate::output;
use crate::services::CreateService;

/// Run the create command.
pub fn run(name: Option<&str>, message: Option<&str>, dry_run: bool) -> Result<()> {
    // Determine the branch name: explicit > derived from message > error
    let name = match (name, message) {
        (Some(n), _) => n.to_string(),
        (None, Some(msg)) => slugify(msg),
        (None, None) => bail!("Either a branch name or --message must be provided"),
    };

    // Validate branch name
    let branch_name = BranchName::new(&name).context("Invalid branch name")?;

    // Validate message content (even when name is provided explicitly)
    if let Some(msg) = message
        && slugify(msg).is_empty()
    {
        bail!("Commit message must contain at least one alphanumeric character");
    }

    // Open repository
    let repo = Repository::open_current().context("Not inside a git repository")?;

    // Get state manager
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    // Ensure initialized
    if !state.is_initialized() {
        bail!("Rung not initialized - run `rung init` first");
    }

    // Ensure on branch
    utils::ensure_on_branch(&repo)?;

    // Create service
    let service = CreateService::new(&repo, &state);

    // Get current branch (will be parent)
    let parent_str = service.current_branch()?;
    let parent = BranchName::new(&parent_str).context("Invalid parent branch name")?;

    // Check if branch already exists
    if service.branch_exists(&name) {
        bail!("Branch '{name}' already exists");
    }

    if dry_run {
        output::info(&format!(
            "Would create branch '{name}' with parent '{parent}'"
        ));

        if let Some(msg) = message {
            if service.is_clean()? {
                output::warn("Working directory is clean - branch would be created without commit");
            } else if service.has_staged_changes()? {
                output::info(&format!("Would create commit with message: {msg}"));
            } else {
                output::warn(
                    "No staged changes - branch would be created without commit (unstaged/untracked files exist)",
                );
            }
        }
    } else {
        // Create the branch
        let result = service.create_branch(&branch_name, &parent, message)?;

        // Report commit status
        if message.is_some() {
            if result.commit_created {
                if let Some(msg) = &result.commit_message {
                    output::info(&format!("Created commit: {msg}"));
                }
            } else if service.is_clean()? {
                output::warn("Working directory is clean - branch created without commit");
            } else {
                output::warn("No staged changes to commit (untracked files may exist)");
            }
        }

        output::success(&format!(
            "Created branch '{}' with parent '{}'",
            result.branch_name, result.parent_name
        ));

        // Show position in stack
        if result.stack_depth > 1 {
            output::info(&format!("Stack depth: {}", result.stack_depth));
        }
    }

    Ok(())
}
