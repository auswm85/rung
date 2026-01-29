//! `rung merge` command - Merge PR and clean up stack.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use rung_core::State;
use rung_git::{Oid, Repository};
use rung_github::{Auth, GitHubClient, MergeMethod};
use serde::Serialize;

use crate::commands::utils;
use crate::output;
use crate::services::MergeService;

/// JSON output for merge command.
#[derive(Debug, Serialize)]
struct MergeOutput {
    merged_branch: String,
    pr_number: u64,
    merge_method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    checked_out: Option<String>,
    descendants_rebased: usize,
}

/// Run the merge command.
#[allow(clippy::too_many_lines)]
pub fn run(json: bool, method: &str, no_delete: bool) -> Result<()> {
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

    // Ensure on branch
    utils::ensure_on_branch(&repo)?;

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

    // Get parent branch from stack (may be None for root branches)
    let stack_parent_branch = branch.parent.as_ref().map(ToString::to_string);

    // Get remote info
    let origin_url = repo.origin_url()?;
    let (owner, repo_name) = Repository::parse_github_remote(&origin_url)?;

    if !json {
        output::info(&format!("Merging PR #{pr_number} for {current_branch}..."));
    }

    // Collect all descendants that need to be rebased
    let descendants =
        MergeService::<Repository, GitHubClient>::collect_descendants(&stack, &current_branch);

    // Capture old commits before any rebasing (needed for --onto)
    let mut old_commits: HashMap<String, Oid> = HashMap::new();
    old_commits.insert(current_branch.clone(), repo.branch_commit(&current_branch)?);
    for branch_name in &descendants {
        old_commits.insert(branch_name.clone(), repo.branch_commit(branch_name)?);
    }

    // Create GitHub client and merge
    let rt = tokio::runtime::Runtime::new()?;
    let parent_branch = rt.block_on(async {
        let auth = Auth::auto();
        let client = GitHubClient::new(&auth)?;
        let service = MergeService::new(&repo, &client, owner.clone(), repo_name.clone());

        // Step 1: Validate PR is mergeable before making any changes
        let pr = service.validate_mergeable(pr_number).await?;

        // Determine parent branch: use stack parent if available, otherwise use PR's base
        let parent_branch = stack_parent_branch
            .clone()
            .unwrap_or_else(|| pr.base_branch.clone());

        // Step 2: Shift child PR bases to parent BEFORE merge (proactive approach)
        if !json && !descendants.is_empty() {
            for branch_name in &descendants {
                if let Some(branch_info) = stack.find_branch(branch_name) {
                    let stack_parent = branch_info
                        .parent
                        .as_ref()
                        .map_or(parent_branch.as_str(), |p| p.as_str());
                    if stack_parent == current_branch {
                        if let Some(child_pr_num) = branch_info.pr {
                            output::info(&format!(
                                "Relinking PR #{child_pr_num} to '{parent_branch}' before merge..."
                            ));
                        }
                    }
                }
            }
        }

        let shifted_prs = service
            .shift_child_pr_bases(&stack, &current_branch, &parent_branch, &descendants)
            .await?;

        // Step 3: Merge the PR
        let merge_result = service.merge_pr(pr_number, merge_method).await;

        // Step 4: If merge fails, rollback the PR base changes
        if let Err(merge_err) = merge_result {
            if !shifted_prs.is_empty() {
                if !json {
                    output::warn("Merge failed, rolling back PR base changes...");
                }
                service.rollback_pr_bases(&shifted_prs).await;
                if !json {
                    for (child_pr_num, original_base) in &shifted_prs {
                        output::info(&format!(
                            "  Restored PR #{child_pr_num} base to '{original_base}'"
                        ));
                    }
                }
            }
            return Err(merge_err);
        }

        if !json {
            output::success(&format!("Merged PR #{pr_number}"));
        }

        // Update stack immediately after merge succeeds
        let children_count =
            service.update_stack_after_merge(&state, &current_branch, &parent_branch)?;

        if !json && children_count > 0 {
            output::info(&format!(
                "Re-parented {children_count} child branch(es) to '{parent_branch}'"
            ));
        }

        // Process each descendant: rebase and push
        if !descendants.is_empty() {
            let results = service
                .rebase_descendants(
                    &state,
                    &stack,
                    &current_branch,
                    &parent_branch,
                    &descendants,
                    &old_commits,
                )
                .await;

            match results {
                Ok(results) => {
                    if !json {
                        for result in &results {
                            if result.rebased {
                                output::info(&format!("  Rebased and pushed {}", result.branch));
                            }
                            if result.pr_updated {
                                output::info(&format!("  Updated PR base for {}", result.branch));
                            }
                        }
                    }
                }
                Err(e) => {
                    if !json {
                        output::error(&format!("Merged parent, but descendant has conflicts: {e}"));
                        output::warn("Manual intervention required. After resolving conflicts:");
                        output::info("  1. git rebase --continue");
                        output::info("  2. git push --force-with-lease");
                        output::info("  3. rung sync");
                        output::info("");
                        output::info(
                            "Note: 'rung sync' will rebase any remaining descendant branches.",
                        );
                    }
                    return Err(e);
                }
            }
        }

        // Delete remote branch AFTER descendants are safe
        if !no_delete {
            match service.delete_remote_branch(&current_branch).await {
                Ok(()) => {
                    if !json {
                        output::info(&format!("Deleted remote branch '{current_branch}'"));
                    }
                }
                Err(e) => {
                    if !json {
                        output::warn(&format!("Failed to delete remote branch: {e}"));
                    }
                }
            }
        }

        Ok::<_, anyhow::Error>(parent_branch)
    })?;

    // Delete local branch and checkout parent
    repo.checkout(&parent_branch)?;

    // Try to delete local branch (may fail if we're on it, but we just checked out parent)
    if let Err(e) = repo.delete_branch(&current_branch) {
        if !json {
            output::warn(&format!("Could not delete local branch: {e}"));
        }
    } else if !json {
        output::info(&format!("Deleted local branch '{current_branch}'"));
    }

    // Pull latest from parent to get the merge commit
    if let Err(e) = repo.pull_ff() {
        if !json {
            output::warn(&format!("Could not pull latest {parent_branch}: {e}"));
        }
    }

    if json {
        return output_json(&MergeOutput {
            merged_branch: current_branch,
            pr_number,
            merge_method: method.to_string(),
            checked_out: Some(parent_branch),
            descendants_rebased: descendants.len(),
        });
    }

    output::info(&format!("Checked out '{parent_branch}'"));
    output::success("Merge complete!");

    Ok(())
}

/// Output merge result as JSON.
fn output_json(output: &MergeOutput) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(output)?);
    Ok(())
}
