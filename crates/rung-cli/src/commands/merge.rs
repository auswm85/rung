//! `rung merge` command - Merge PR and clean up stack.

use anyhow::{Context, Result, bail};
use rung_core::State;
use rung_git::Repository;
use rung_github::{Auth, GitHubClient, MergeMethod, MergePullRequest};

use crate::output;

/// Run the merge command.
pub fn run(method: &str, no_delete: bool) -> Result<()> {
    // Parse merge method
    let merge_method = match method.to_lowercase().as_str() {
        "squash" => MergeMethod::Squash,
        "merge" => MergeMethod::Merge,
        "rebase" => MergeMethod::Rebase,
        _ => bail!("Invalid merge method: {method}. Use squash, merge, or rebase."),
    };

    // Open repository
    let repo = Repository::open_current().context("Not inside a git repository")?;
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    // Ensure initialized
    if !state.is_initialized() {
        bail!("Rung not initialized - run `rung init` first");
    }

    // Get current branch
    let current_branch = repo.current_branch()?;

    // Load stack and find the branch
    let stack = state.load_stack()?;
    let branch = stack
        .find_branch(&current_branch)
        .ok_or_else(|| anyhow::anyhow!("Branch '{current_branch}' not in stack"))?;

    // Get PR number
    let pr_number = branch.pr.ok_or_else(|| {
        anyhow::anyhow!("No PR associated with branch '{current_branch}'. Run `rung submit` first.")
    })?;

    // Get parent branch for later checkout
    let parent_branch = branch.parent.clone().unwrap_or_else(|| "main".to_string());

    // Get remote info
    let origin_url = repo.origin_url()?;
    let (owner, repo_name) = Repository::parse_github_remote(&origin_url)?;

    output::info(&format!("Merging PR #{pr_number} for {current_branch}..."));

    // Create GitHub client and merge
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let auth = Auth::auto();
        let client = GitHubClient::new(&auth)?;

        // Merge the PR
        let merge_request = MergePullRequest {
            commit_title: None, // Use GitHub's default
            commit_message: None,
            merge_method,
        };

        client
            .merge_pr(&owner, &repo_name, pr_number, merge_request)
            .await
            .context("Failed to merge PR")?;

        output::success(&format!("Merged PR #{pr_number}"));

        // Delete remote branch if requested
        if !no_delete {
            match client.delete_ref(&owner, &repo_name, &current_branch).await {
                Ok(()) => output::info(&format!("Deleted remote branch '{current_branch}'")),
                Err(e) => output::warn(&format!("Failed to delete remote branch: {e}")),
            }
        }

        Ok::<_, anyhow::Error>(())
    })?;

    // Remove branch from stack
    let mut stack = state.load_stack()?;

    // Re-parent any children to point to the merged branch's parent
    let children: Vec<_> = stack
        .branches
        .iter()
        .filter(|b| b.parent.as_ref() == Some(&current_branch))
        .map(|b| b.name.clone())
        .collect();

    for child_name in &children {
        if let Some(child) = stack.find_branch_mut(child_name) {
            child.parent = Some(parent_branch.clone());
        }
    }

    // Remove the merged branch from stack
    stack.branches.retain(|b| b.name != current_branch);
    state.save_stack(&stack)?;

    if !children.is_empty() {
        output::info(&format!(
            "Re-parented {} child branch(es) to '{}'",
            children.len(),
            parent_branch
        ));
    }

    // Delete local branch and checkout parent
    repo.checkout(&parent_branch)?;

    // Try to delete local branch (may fail if we're on it, but we just checked out parent)
    if let Err(e) = repo.delete_branch(&current_branch) {
        output::warn(&format!("Could not delete local branch: {e}"));
    } else {
        output::info(&format!("Deleted local branch '{current_branch}'"));
    }

    // Pull latest from parent
    output::info(&format!("Checked out '{parent_branch}'"));

    output::success("Merge complete!");

    Ok(())
}
