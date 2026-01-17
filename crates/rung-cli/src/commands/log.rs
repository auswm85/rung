//! `rung log` command - show commits between the base branch and HEAD.

use super::utils::open_repo_and_state;
use crate::output;
use anyhow::{Result, bail};

// Run the log command.
pub fn run() -> Result<()> {
    let (repo, state) = open_repo_and_state()?;
    let current = repo.current_branch()?;
    let stack = state.load_stack()?;

    if stack.is_empty() {
        bail!("No branches in stack. Use `rung create <name>` to add one.");
    }

    // Get branches
    let Some(head) = stack.find_branch(&current) else {
        bail!("Current branch '{current}' is not in stack")
    };

    let Some(base) = &head.parent else {
        bail!("Current branch '{current}' has no parent branch")
    };

    // Get commits
    let head_oid = repo.branch_commit(head.name.as_str())?;
    let base_oid = repo.branch_commit(base.as_str())?;
    let commits = repo.commits_between(base_oid, head_oid)?;

    if commits.is_empty() {
        output::warn("Current branch has no commits");
        return Ok(());
    }

    // Print commits
    for commit in commits {
        let commit = repo.find_commit(commit)?;

        let short_id = &commit.id().to_string()[..7];
        let msg = commit.message().unwrap_or("").trim();
        let sig = commit.author();
        let author = sig.name().unwrap_or("unknown");

        let msg = format!("{short_id:<10} {msg}     {author}");
        output::info(&msg);
    }

    Ok(())
}
