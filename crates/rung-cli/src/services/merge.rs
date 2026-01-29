//! Merge service for merging PRs and cleaning up the stack.
//!
//! This service encapsulates the business logic for the merge command,
//! accepting trait-based dependencies for testability.

#![allow(dead_code)] // Some fields not yet fully utilized

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use rung_core::stack::Stack;
use rung_core::{BranchName, StateStore};
use rung_git::{GitOps, Oid};
use rung_github::{GitHubApi, MergeMethod, MergePullRequest, UpdatePullRequest};
use serde::Serialize;

/// Configuration for a merge operation.
#[derive(Debug, Clone)]
pub struct MergeConfig {
    pub merge_method: MergeMethod,
    pub no_delete: bool,
}

/// Result of a successful merge operation.
#[derive(Debug, Clone, Serialize)]
pub struct MergeResult {
    pub merged_branch: String,
    pub pr_number: u64,
    pub merge_method: String,
    pub parent_branch: String,
    pub descendants_rebased: usize,
}

/// Information about a descendant branch that was processed.
#[derive(Debug, Clone)]
pub struct DescendantResult {
    pub branch: String,
    pub rebased: bool,
    pub pushed: bool,
    pub pr_updated: bool,
    pub error: Option<String>,
}

/// Service for merge operations with trait-based dependencies.
pub struct MergeService<'a, G: GitOps, H: GitHubApi> {
    repo: &'a G,
    client: &'a H,
    owner: String,
    repo_name: String,
}

#[allow(clippy::future_not_send)]
impl<'a, G: GitOps, H: GitHubApi> MergeService<'a, G, H> {
    /// Create a new merge service.
    #[must_use]
    pub const fn new(repo: &'a G, client: &'a H, owner: String, repo_name: String) -> Self {
        Self {
            repo,
            client,
            owner,
            repo_name,
        }
    }

    /// Validate that a PR is mergeable.
    pub async fn validate_mergeable(&self, pr_number: u64) -> Result<rung_github::PullRequest> {
        let pr = self
            .client
            .get_pr(&self.owner, &self.repo_name, pr_number)
            .await
            .context("Failed to fetch PR status")?;

        if pr.mergeable == Some(false) {
            bail!(
                "PR #{pr_number} is not mergeable. State: {}",
                pr.mergeable_state.as_deref().unwrap_or("unknown")
            );
        }

        Ok(pr)
    }

    /// Shift child PR bases to parent before merge.
    ///
    /// Returns the list of PRs that were shifted (for potential rollback).
    pub async fn shift_child_pr_bases(
        &self,
        stack: &Stack,
        current_branch: &str,
        parent_branch: &str,
        descendants: &[String],
    ) -> Result<Vec<(u64, String)>> {
        let mut shifted_prs = Vec::new();

        for branch_name in descendants {
            let branch_info = stack
                .find_branch(branch_name)
                .ok_or_else(|| anyhow::anyhow!("Branch '{branch_name}' not found in stack"))?;

            let stack_parent = branch_info
                .parent
                .as_ref()
                .map_or(parent_branch, |p| p.as_str());

            // Only shift direct children of the merging branch
            if stack_parent == current_branch {
                if let Some(child_pr_num) = branch_info.pr {
                    let update = UpdatePullRequest {
                        title: None,
                        body: None,
                        base: Some(parent_branch.to_string()),
                    };
                    self.client
                        .update_pr(&self.owner, &self.repo_name, child_pr_num, update)
                        .await
                        .with_context(|| format!("Failed to update PR #{child_pr_num} base"))?;

                    shifted_prs.push((child_pr_num, current_branch.to_string()));
                }
            }
        }

        Ok(shifted_prs)
    }

    /// Rollback PR base changes after a failed merge.
    pub async fn rollback_pr_bases(&self, shifted_prs: &[(u64, String)]) {
        for (child_pr_num, original_base) in shifted_prs {
            let rollback = UpdatePullRequest {
                title: None,
                body: None,
                base: Some(original_base.clone()),
            };
            let _ = self
                .client
                .update_pr(&self.owner, &self.repo_name, *child_pr_num, rollback)
                .await;
        }
    }

    /// Merge a PR on GitHub.
    pub async fn merge_pr(&self, pr_number: u64, merge_method: MergeMethod) -> Result<()> {
        let merge_request = MergePullRequest {
            commit_title: None,
            commit_message: None,
            merge_method,
        };

        self.client
            .merge_pr(&self.owner, &self.repo_name, pr_number, merge_request)
            .await
            .context("Failed to merge PR")?;

        Ok(())
    }

    /// Update the stack after a successful merge.
    #[allow(clippy::unused_self)]
    pub fn update_stack_after_merge<S: StateStore>(
        &self,
        state: &S,
        current_branch: &str,
        parent_branch: &str,
    ) -> Result<usize> {
        let mut stack = state.load_stack()?;

        // Count children before re-parenting
        let children_count = stack
            .branches
            .iter()
            .filter(|b| b.parent.as_ref().is_some_and(|p| p == current_branch))
            .count();

        // Re-parent any children to point to the merged branch's parent
        let new_parent = BranchName::new(parent_branch).context("Invalid parent branch name")?;
        for branch in &mut stack.branches {
            if branch.parent.as_ref().is_some_and(|p| p == current_branch) {
                branch.parent = Some(new_parent.clone());
            }
        }

        // Mark the branch as merged
        stack
            .mark_merged(current_branch)
            .ok_or_else(|| anyhow::anyhow!("Branch '{current_branch}' missing from stack"))?;

        // Clear merged history when entire stack is done
        stack.clear_merged_if_empty();

        state.save_stack(&stack)?;

        Ok(children_count)
    }

    /// Rebase descendant branches onto the new parent.
    pub async fn rebase_descendants<S: StateStore>(
        &self,
        _state: &S,
        stack: &Stack,
        current_branch: &str,
        parent_branch: &str,
        descendants: &[String],
        old_commits: &HashMap<String, Oid>,
    ) -> Result<Vec<DescendantResult>> {
        let mut results = Vec::new();

        // Fetch to get the merge commit on the parent branch
        self.repo
            .fetch(parent_branch)
            .with_context(|| format!("Failed to fetch {parent_branch}"))?;

        for branch_name in descendants {
            let branch_info = stack
                .find_branch(branch_name)
                .ok_or_else(|| anyhow::anyhow!("Branch '{branch_name}' not found in stack"))?;

            let stack_parent = branch_info
                .parent
                .as_ref()
                .map_or(parent_branch, |p| p.as_str());

            // Determine the new base for this branch
            let new_base = if stack_parent == current_branch {
                parent_branch.to_string()
            } else {
                stack_parent.to_string()
            };

            // Checkout and rebase
            self.repo.checkout(branch_name)?;

            // Get new base commit
            let new_base_commit = if new_base == parent_branch {
                self.repo.remote_branch_commit(&new_base)?
            } else {
                self.repo.branch_commit(&new_base)?
            };

            let old_base_commit = old_commits
                .get(stack_parent)
                .copied()
                .ok_or_else(|| anyhow::anyhow!("Could not find old commit for {stack_parent}"))?;

            // Attempt rebase
            if let Err(e) = self.repo.rebase_onto_from(new_base_commit, old_base_commit) {
                results.push(DescendantResult {
                    branch: branch_name.clone(),
                    rebased: false,
                    pushed: false,
                    pr_updated: false,
                    error: Some(e.to_string()),
                });
                bail!("Rebase conflict in '{branch_name}' - manual intervention required");
            }

            // Force push rebased branch
            self.repo
                .push(branch_name, true)
                .with_context(|| format!("Failed to push rebased {branch_name}"))?;

            // Update PR base for grandchildren (direct children were already shifted)
            let mut pr_updated = false;
            if stack_parent != current_branch {
                if let Some(child_pr_num) = branch_info.pr {
                    let update = UpdatePullRequest {
                        title: None,
                        body: None,
                        base: Some(new_base.clone()),
                    };
                    if self
                        .client
                        .update_pr(&self.owner, &self.repo_name, child_pr_num, update)
                        .await
                        .is_ok()
                    {
                        pr_updated = true;
                    }
                }
            }

            results.push(DescendantResult {
                branch: branch_name.clone(),
                rebased: true,
                pushed: true,
                pr_updated,
                error: None,
            });
        }

        Ok(results)
    }

    /// Delete the remote branch after merge.
    pub async fn delete_remote_branch(&self, branch: &str) -> Result<()> {
        self.client
            .delete_ref(&self.owner, &self.repo_name, branch)
            .await
            .context("Failed to delete remote branch")
    }

    /// Collect all descendants of a branch in topological order.
    #[must_use]
    pub fn collect_descendants(stack: &Stack, root: &str) -> Vec<String> {
        let mut descendants = Vec::new();
        let mut queue = vec![root.to_string()];

        while let Some(parent) = queue.pop() {
            for branch in &stack.branches {
                if branch.parent.as_ref().is_some_and(|p| p == &parent) {
                    descendants.push(branch.name.to_string());
                    queue.push(branch.name.to_string());
                }
            }
        }
        descendants
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_config() {
        let config = MergeConfig {
            merge_method: MergeMethod::Squash,
            no_delete: false,
        };
        assert!(!config.no_delete);
    }
}
