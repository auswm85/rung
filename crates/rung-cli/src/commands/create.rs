//! `rung create` command - Create a new branch in the stack.

use anyhow::{Context, Result, bail};
use rung_core::{BranchName, State, slugify, stack::StackBranch};
use rung_git::Repository;

use crate::output;

/// Run the create command.
pub fn run(name: Option<&str>, message: Option<&str>) -> Result<()> {
    // Determine the branch name: explicit > derived from message > error
    let name = match (name, message) {
        (Some(n), _) => n.to_string(),
        (None, Some(msg)) => slugify(msg),
        (None, None) => bail!("Either a branch name or --message must be provided"),
    };

    // Validate branch name
    let branch_name = BranchName::new(&name).context("Invalid branch name")?;

    // Validate message content (even when name is provided explicitly)
    if let Some(msg) = message {
        if slugify(msg).is_empty() {
            bail!("Commit message must contain at least one alphanumeric character");
        }
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

    // Get current branch (will be parent)
    let parent_str = repo.current_branch()?;
    let parent = BranchName::new(&parent_str).context("Invalid parent branch name")?;

    // Check if branch already exists
    if repo.branch_exists(&name) {
        bail!("Branch '{name}' already exists");
    }

    // Create the branch at current HEAD (parent's tip)
    repo.create_branch(&name)?;

    // Add to stack
    let mut stack = state.load_stack()?;
    let branch = StackBranch::new(branch_name, Some(parent.clone()));
    stack.add_branch(branch);
    state.save_stack(&stack)?;

    // Checkout the new branch
    repo.checkout(&name)?;

    // If message is provided, stage all changes and create a commit on the NEW branch
    if let Some(msg) = message {
        // Check for changes before staging to provide clearer feedback
        if repo.is_clean()? {
            output::warn("Working directory is clean - branch created without commit");
        } else {
            repo.stage_all().context("Failed to stage changes")?;

            if repo.has_staged_changes()? {
                repo.create_commit(msg).context("Failed to create commit")?;
                output::info(&format!("Created commit: {msg}"));
            } else {
                output::warn("No staged changes to commit (untracked files may exist)");
            }
        }
    }

    output::success(&format!("Created branch '{name}' with parent '{parent}'"));

    // Show position in stack
    let ancestry = stack.ancestry(&name);
    if ancestry.len() > 1 {
        output::info(&format!("Stack depth: {}", ancestry.len()));
    }

    Ok(())
}
