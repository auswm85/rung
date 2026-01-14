//! `rung merge` command - Merge PR and clean up stack.

use anyhow::{Context, Result, bail};
use rung_core::State;
use rung_core::stack::Stack;
use rung_git::{Oid, Repository};
use rung_github::{Auth, GitHubClient, MergeMethod, MergePullRequest, UpdatePullRequest};

use crate::output;

/// Run the merge command.
#[allow(clippy::too_many_lines)]
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

    // Collect all descendants that need to be rebased
    let descendants = collect_descendants(&stack, &current_branch);

    // Capture old commits before any rebasing (needed for --onto)
    let mut old_commits: std::collections::HashMap<String, Oid> = std::collections::HashMap::new();
    old_commits.insert(current_branch.clone(), repo.branch_commit(&current_branch)?);
    for branch_name in &descendants {
        old_commits.insert(branch_name.clone(), repo.branch_commit(branch_name)?);
    }

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

        // Update stack immediately after merge succeeds
        // This ensures stack.json reflects reality even if rebases fail later
        {
            let mut stack = state.load_stack()?;

            // Count children before re-parenting
            let children_count = stack
                .branches
                .iter()
                .filter(|b| b.parent.as_ref() == Some(&current_branch))
                .count();

            // Re-parent any children to point to the merged branch's parent
            for branch in &mut stack.branches {
                if branch.parent.as_ref() == Some(&current_branch) {
                    branch.parent = Some(parent_branch.clone());
                }
            }

            // Remove the merged branch from stack
            stack.branches.retain(|b| b.name != current_branch);
            state.save_stack(&stack)?;

            if children_count > 0 {
                output::info(&format!(
                    "Re-parented {children_count} child branch(es) to '{parent_branch}'"
                ));
            }
        }

        // Fetch to get the merge commit on the parent branch
        repo.fetch(&parent_branch)
            .with_context(|| format!("Failed to fetch {parent_branch}"))?;

        // Process each descendant: update PR base, rebase, push
        for branch_name in &descendants {
            let branch_info = stack
                .find_branch(branch_name)
                .ok_or_else(|| anyhow::anyhow!("Branch '{branch_name}' not found in stack"))?;

            let stack_parent = branch_info.parent.as_deref().unwrap_or("main");

            // Determine the new base for this branch's PR
            // Direct children of merged branch → parent_branch (e.g., main)
            // Grandchildren → their parent branch (which we just rebased)
            let new_base = if stack_parent == current_branch {
                parent_branch.clone()
            } else {
                stack_parent.to_string()
            };

            // Update PR base on GitHub (before parent branch is deleted)
            if let Some(child_pr_num) = branch_info.pr {
                output::info(&format!(
                    "  Updating PR #{child_pr_num} base to '{new_base}'..."
                ));
                let update = UpdatePullRequest {
                    title: None,
                    body: None,
                    base: Some(new_base.clone()),
                };
                client
                    .update_pr(&owner, &repo_name, child_pr_num, update)
                    .await
                    .with_context(|| format!("Failed to update PR #{child_pr_num} base"))?;
            }

            // Rebase onto new parent's tip, using --onto to only bring unique commits
            output::info(&format!("  Rebasing {branch_name} onto '{new_base}'..."));
            repo.checkout(branch_name)?;

            // For direct children of merged branch, use remote ref (we just fetched)
            // For grandchildren, use local ref (we just rebased the parent locally)
            let new_base_commit = if new_base == parent_branch {
                repo.remote_branch_commit(&new_base)?
            } else {
                repo.branch_commit(&new_base)?
            };
            let old_base_commit = old_commits
                .get(stack_parent)
                .copied()
                .ok_or_else(|| anyhow::anyhow!("Could not find old commit for {stack_parent}"))?;

            if let Err(e) = repo.rebase_onto_from(new_base_commit, old_base_commit) {
                output::error(&format!("Rebase conflict in {branch_name}: {e}"));
                output::warn(
                    "Resolve conflicts, then run: git rebase --continue && git push --force",
                );
                bail!("Rebase failed - resolve conflicts before completing merge cleanup");
            }

            // Force push rebased branch
            repo.push(branch_name, true)
                .with_context(|| format!("Failed to push rebased {branch_name}"))?;
            output::info(&format!("  Rebased and pushed {branch_name}"));
        }

        // Delete remote branch AFTER descendants are safe
        if !no_delete {
            match client.delete_ref(&owner, &repo_name, &current_branch).await {
                Ok(()) => output::info(&format!("Deleted remote branch '{current_branch}'")),
                Err(e) => output::warn(&format!("Failed to delete remote branch: {e}")),
            }
        }

        Ok::<_, anyhow::Error>(())
    })?;

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

/// Collect all descendants of a branch in topological order (parents before children).
fn collect_descendants(stack: &Stack, root: &str) -> Vec<String> {
    let mut descendants = Vec::new();
    let mut queue = vec![root.to_string()];

    while let Some(parent) = queue.pop() {
        for branch in &stack.branches {
            if branch.parent.as_ref().is_some_and(|p| p == &parent) {
                descendants.push(branch.name.clone());
                queue.push(branch.name.clone());
            }
        }
    }
    descendants
}
