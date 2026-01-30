//! Merge service for merging PRs and cleaning up the stack.
//!
//! This service encapsulates the business logic for the merge command,
//! accepting trait-based dependencies for testability.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use rung_core::stack::Stack;
use rung_core::{BranchName, StateStore};
use rung_git::{GitOps, Oid};
use rung_github::{GitHubApi, MergeMethod, MergePullRequest, UpdatePullRequest};

/// Information about a descendant branch that was processed.
#[derive(Debug, Clone)]
pub struct DescendantResult {
    pub branch: String,
    pub rebased: bool,
    pub pr_updated: bool,
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
                bail!("Rebase conflict in '{branch_name}': {e}");
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
                pr_updated,
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
    use rung_core::stack::StackBranch;

    #[test]
    fn test_collect_descendants_empty() {
        let stack = Stack::default();
        let descendants =
            MergeService::<rung_git::Repository, rung_github::GitHubClient>::collect_descendants(
                &stack, "main",
            );
        assert!(descendants.is_empty());
    }

    #[test]
    fn test_descendant_result_creation() {
        let result = DescendantResult {
            branch: "feature/child".to_string(),
            rebased: true,
            pr_updated: false,
        };
        assert_eq!(result.branch, "feature/child");
        assert!(result.rebased);
        assert!(!result.pr_updated);
    }

    #[test]
    fn test_descendant_result_clone() {
        let result = DescendantResult {
            branch: "test".to_string(),
            rebased: true,
            pr_updated: true,
        };
        let cloned = result.clone();
        assert_eq!(result.branch, cloned.branch);
        assert_eq!(result.rebased, cloned.rebased);
        assert_eq!(result.pr_updated, cloned.pr_updated);
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_collect_descendants_single_child() {
        let mut stack = Stack::default();
        let parent = BranchName::new("feature/parent").expect("valid");
        let child = BranchName::new("feature/child").expect("valid");
        let main = BranchName::new("main").expect("valid");

        stack.add_branch(StackBranch::new(parent, Some(main)));
        stack.add_branch(StackBranch::new(
            child,
            Some(BranchName::new("feature/parent").expect("valid")),
        ));

        let descendants =
            MergeService::<rung_git::Repository, rung_github::GitHubClient>::collect_descendants(
                &stack,
                "feature/parent",
            );
        assert_eq!(descendants.len(), 1);
        assert_eq!(descendants[0], "feature/child");
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_collect_descendants_multiple_children() {
        let mut stack = Stack::default();
        let parent = BranchName::new("feature/parent").expect("valid");
        let child1 = BranchName::new("feature/child1").expect("valid");
        let child2 = BranchName::new("feature/child2").expect("valid");
        let main = BranchName::new("main").expect("valid");

        stack.add_branch(StackBranch::new(parent, Some(main)));
        stack.add_branch(StackBranch::new(
            child1,
            Some(BranchName::new("feature/parent").expect("valid")),
        ));
        stack.add_branch(StackBranch::new(
            child2,
            Some(BranchName::new("feature/parent").expect("valid")),
        ));

        let descendants =
            MergeService::<rung_git::Repository, rung_github::GitHubClient>::collect_descendants(
                &stack,
                "feature/parent",
            );
        assert_eq!(descendants.len(), 2);
        assert!(descendants.contains(&"feature/child1".to_string()));
        assert!(descendants.contains(&"feature/child2".to_string()));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_collect_descendants_nested_children() {
        let mut stack = Stack::default();
        let main = BranchName::new("main").expect("valid");
        let parent = BranchName::new("feature/parent").expect("valid");
        let child = BranchName::new("feature/child").expect("valid");
        let grandchild = BranchName::new("feature/grandchild").expect("valid");

        stack.add_branch(StackBranch::new(parent, Some(main)));
        stack.add_branch(StackBranch::new(
            child,
            Some(BranchName::new("feature/parent").expect("valid")),
        ));
        stack.add_branch(StackBranch::new(
            grandchild,
            Some(BranchName::new("feature/child").expect("valid")),
        ));

        let descendants =
            MergeService::<rung_git::Repository, rung_github::GitHubClient>::collect_descendants(
                &stack,
                "feature/parent",
            );
        assert_eq!(descendants.len(), 2);
        // Should contain both child and grandchild (in topological order from BFS)
        assert!(descendants.contains(&"feature/child".to_string()));
        assert!(descendants.contains(&"feature/grandchild".to_string()));
    }

    #[test]
    fn test_collect_descendants_no_match() {
        let stack = Stack::default();
        let descendants =
            MergeService::<rung_git::Repository, rung_github::GitHubClient>::collect_descendants(
                &stack,
                "nonexistent",
            );
        assert!(descendants.is_empty());
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_collect_descendants_from_middle_of_chain() {
        let mut stack = Stack::default();
        let main = BranchName::new("main").expect("valid");
        let level1 = BranchName::new("level1").expect("valid");
        let level2 = BranchName::new("level2").expect("valid");
        let level3 = BranchName::new("level3").expect("valid");

        stack.add_branch(StackBranch::new(level1, Some(main)));
        stack.add_branch(StackBranch::new(
            level2,
            Some(BranchName::new("level1").expect("valid")),
        ));
        stack.add_branch(StackBranch::new(
            level3,
            Some(BranchName::new("level2").expect("valid")),
        ));

        // Get descendants from level2 - should only get level3
        let descendants =
            MergeService::<rung_git::Repository, rung_github::GitHubClient>::collect_descendants(
                &stack, "level2",
            );
        assert_eq!(descendants.len(), 1);
        assert_eq!(descendants[0], "level3");
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_collect_descendants_diamond_topology() {
        // main -> A -> B
        //      -> A -> C
        //      -> B and C both point to D (diamond merge)
        let mut stack = Stack::default();
        let main = BranchName::new("main").expect("valid");
        let a = BranchName::new("branch-a").expect("valid");
        let b = BranchName::new("branch-b").expect("valid");
        let c = BranchName::new("branch-c").expect("valid");

        stack.add_branch(StackBranch::new(a, Some(main)));
        stack.add_branch(StackBranch::new(
            b,
            Some(BranchName::new("branch-a").expect("valid")),
        ));
        stack.add_branch(StackBranch::new(
            c,
            Some(BranchName::new("branch-a").expect("valid")),
        ));

        let descendants =
            MergeService::<rung_git::Repository, rung_github::GitHubClient>::collect_descendants(
                &stack, "branch-a",
            );
        assert_eq!(descendants.len(), 2);
        assert!(descendants.contains(&"branch-b".to_string()));
        assert!(descendants.contains(&"branch-c".to_string()));
    }

    #[test]
    fn test_descendant_result_all_fields_true() {
        let result = DescendantResult {
            branch: "all-true".to_string(),
            rebased: true,
            pr_updated: true,
        };
        assert!(result.rebased);
        assert!(result.pr_updated);
    }

    #[test]
    fn test_descendant_result_all_fields_false() {
        let result = DescendantResult {
            branch: "all-false".to_string(),
            rebased: false,
            pr_updated: false,
        };
        assert!(!result.rebased);
        assert!(!result.pr_updated);
    }
}
