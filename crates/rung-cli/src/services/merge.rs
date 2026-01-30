//! Merge service for merging PRs and cleaning up the stack.
//!
//! This service encapsulates the business logic for the merge command,
//! accepting trait-based dependencies for testability.

use std::collections::{HashMap, VecDeque};

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
    /// Error message if rebase or push failed for this branch.
    #[allow(dead_code)]
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
    ///
    /// GitHub may return `mergeable: None` while computing merge status.
    /// This method polls until `mergeable` becomes `Some(true)` or retries exhaust.
    pub async fn validate_mergeable(&self, pr_number: u64) -> Result<rung_github::PullRequest> {
        const MAX_RETRIES: u32 = 5;
        const RETRY_DELAY_MS: u64 = 1000;

        let mut attempts = 0;
        loop {
            let pr = self
                .client
                .get_pr(&self.owner, &self.repo_name, pr_number)
                .await
                .context("Failed to fetch PR status")?;

            match pr.mergeable {
                Some(true) => return Ok(pr),
                Some(false) => {
                    bail!(
                        "PR #{pr_number} is not mergeable. State: {}",
                        pr.mergeable_state.as_deref().unwrap_or("unknown")
                    );
                }
                None => {
                    attempts += 1;
                    if attempts >= MAX_RETRIES {
                        bail!(
                            "PR #{pr_number} mergeable status unknown after {MAX_RETRIES} attempts. \
                             State: {}",
                            pr.mergeable_state.as_deref().unwrap_or("unknown")
                        );
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS)).await;
                }
            }
        }
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
            if stack_parent == current_branch
                && let Some(child_pr_num) = branch_info.pr
            {
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
        use std::collections::HashSet;

        let mut results = Vec::new();
        let mut failed_branches: HashSet<String> = HashSet::new();

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

            // Skip if an ancestor failed - propagate failure down the chain
            if failed_branches.contains(stack_parent) {
                failed_branches.insert(branch_name.clone());
                results.push(DescendantResult {
                    branch: branch_name.clone(),
                    rebased: false,
                    pr_updated: false,
                    error: Some(format!(
                        "Skipped: ancestor '{stack_parent}' failed to rebase"
                    )),
                });
                continue;
            }

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
                failed_branches.insert(branch_name.clone());
                results.push(DescendantResult {
                    branch: branch_name.clone(),
                    rebased: false,
                    pr_updated: false,
                    error: Some(format!("Rebase conflict: {e}")),
                });
                continue;
            }

            // Force push rebased branch
            if let Err(e) = self.repo.push(branch_name, true) {
                failed_branches.insert(branch_name.clone());
                results.push(DescendantResult {
                    branch: branch_name.clone(),
                    rebased: true,
                    pr_updated: false,
                    error: Some(format!("Push failed: {e}")),
                });
                continue;
            }

            // Update PR base for grandchildren (direct children were already shifted)
            // Only update if this branch and all its ancestors succeeded
            let mut pr_updated = false;
            if stack_parent != current_branch
                && !failed_branches.contains(branch_name)
                && let Some(child_pr_num) = branch_info.pr
            {
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

            results.push(DescendantResult {
                branch: branch_name.clone(),
                rebased: true,
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

    /// Collect all descendants of a branch in topological order (BFS).
    #[must_use]
    pub fn collect_descendants(stack: &Stack, root: &str) -> Vec<String> {
        let mut descendants = Vec::new();
        let mut queue = VecDeque::from([root.to_string()]);

        while let Some(parent) = queue.pop_front() {
            for branch in &stack.branches {
                if branch.parent.as_ref().is_some_and(|p| p == &parent) {
                    descendants.push(branch.name.to_string());
                    queue.push_back(branch.name.to_string());
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
            error: None,
        };
        assert_eq!(result.branch, "feature/child");
        assert!(result.rebased);
        assert!(!result.pr_updated);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_descendant_result_clone() {
        let result = DescendantResult {
            branch: "test".to_string(),
            rebased: true,
            pr_updated: true,
            error: None,
        };
        let cloned = result.clone();
        assert_eq!(result.branch, cloned.branch);
        assert_eq!(result.rebased, cloned.rebased);
        assert_eq!(result.pr_updated, cloned.pr_updated);
        assert_eq!(result.error, cloned.error);
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
            error: None,
        };
        assert!(result.rebased);
        assert!(result.pr_updated);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_descendant_result_all_fields_false() {
        let result = DescendantResult {
            branch: "all-false".to_string(),
            rebased: false,
            pr_updated: false,
            error: None,
        };
        assert!(!result.rebased);
        assert!(!result.pr_updated);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_descendant_result_with_error() {
        let result = DescendantResult {
            branch: "failed".to_string(),
            rebased: false,
            pr_updated: false,
            error: Some("Rebase conflict: merge conflict".to_string()),
        };
        assert!(!result.rebased);
        assert!(!result.pr_updated);
        assert!(result.error.is_some());
        assert!(
            result
                .error
                .as_ref()
                .is_some_and(|e| e.contains("conflict"))
        );
    }

    // Mock-based tests for MergeService methods
    #[allow(clippy::manual_async_fn, clippy::unwrap_used, clippy::expect_used)]
    mod mock_tests {
        use super::*;
        use crate::services::test_mocks::{MockGitOps, MockStateStore};
        use rung_core::stack::StackBranch;
        use rung_git::Oid;
        use std::sync::atomic::{AtomicBool, Ordering};

        // Mock GitHubApi for merge testing
        struct MockGitHubClient {
            pr_mergeable: Option<bool>,
            merge_should_fail: bool,
            delete_should_fail: bool,
            update_pr_called: AtomicBool,
        }

        impl MockGitHubClient {
            fn new() -> Self {
                Self {
                    pr_mergeable: Some(true),
                    merge_should_fail: false,
                    delete_should_fail: false,
                    update_pr_called: AtomicBool::new(false),
                }
            }

            fn with_unmergeable_pr(mut self) -> Self {
                self.pr_mergeable = Some(false);
                self
            }

            fn with_unknown_mergeable(mut self) -> Self {
                self.pr_mergeable = None;
                self
            }

            fn with_merge_failure(mut self) -> Self {
                self.merge_should_fail = true;
                self
            }

            fn with_delete_failure(mut self) -> Self {
                self.delete_should_fail = true;
                self
            }
        }

        impl rung_github::GitHubApi for MockGitHubClient {
            fn get_pr(
                &self,
                _owner: &str,
                _repo: &str,
                number: u64,
            ) -> impl std::future::Future<Output = rung_github::Result<rung_github::PullRequest>> + Send
            {
                let mergeable = self.pr_mergeable;
                async move {
                    Ok(rung_github::PullRequest {
                        number,
                        title: "Test PR".to_string(),
                        body: None,
                        state: rung_github::PullRequestState::Open,
                        base_branch: "main".to_string(),
                        head_branch: "feature".to_string(),
                        html_url: format!("https://github.com/test/repo/pull/{number}"),
                        mergeable,
                        mergeable_state: Some(match mergeable {
                            Some(true) => "clean".to_string(),
                            Some(false) => "blocked".to_string(),
                            None => "unknown".to_string(),
                        }),
                        draft: false,
                    })
                }
            }

            fn get_prs_batch(
                &self,
                _owner: &str,
                _repo: &str,
                _numbers: &[u64],
            ) -> impl std::future::Future<
                Output = rung_github::Result<
                    std::collections::HashMap<u64, rung_github::PullRequest>,
                >,
            > + Send {
                async { Ok(std::collections::HashMap::new()) }
            }

            fn find_pr_for_branch(
                &self,
                _owner: &str,
                _repo: &str,
                _branch: &str,
            ) -> impl std::future::Future<
                Output = rung_github::Result<Option<rung_github::PullRequest>>,
            > + Send {
                async { Ok(None) }
            }

            fn create_pr(
                &self,
                _owner: &str,
                _repo: &str,
                _params: rung_github::CreatePullRequest,
            ) -> impl std::future::Future<Output = rung_github::Result<rung_github::PullRequest>> + Send
            {
                async { Err(rung_github::Error::PrNotFound(0)) }
            }

            fn update_pr(
                &self,
                _owner: &str,
                _repo: &str,
                number: u64,
                _params: rung_github::UpdatePullRequest,
            ) -> impl std::future::Future<Output = rung_github::Result<rung_github::PullRequest>> + Send
            {
                self.update_pr_called.store(true, Ordering::SeqCst);
                async move {
                    Ok(rung_github::PullRequest {
                        number,
                        title: "Updated".to_string(),
                        body: None,
                        state: rung_github::PullRequestState::Open,
                        base_branch: "main".to_string(),
                        head_branch: "feature".to_string(),
                        html_url: format!("https://github.com/test/repo/pull/{number}"),
                        mergeable: None,
                        mergeable_state: None,
                        draft: false,
                    })
                }
            }

            fn get_check_runs(
                &self,
                _owner: &str,
                _repo: &str,
                _commit_sha: &str,
            ) -> impl std::future::Future<Output = rung_github::Result<Vec<rung_github::CheckRun>>> + Send
            {
                async { Ok(vec![]) }
            }

            fn merge_pr(
                &self,
                _owner: &str,
                _repo: &str,
                _number: u64,
                _params: rung_github::MergePullRequest,
            ) -> impl std::future::Future<Output = rung_github::Result<rung_github::MergeResult>> + Send
            {
                let should_fail = self.merge_should_fail;
                async move {
                    if should_fail {
                        Err(rung_github::Error::ApiError {
                            status: 405,
                            message: "mock merge failed".to_string(),
                        })
                    } else {
                        Ok(rung_github::MergeResult {
                            sha: "abc123".to_string(),
                            merged: true,
                            message: "Pull Request successfully merged".to_string(),
                        })
                    }
                }
            }

            fn delete_ref(
                &self,
                _owner: &str,
                _repo: &str,
                _branch: &str,
            ) -> impl std::future::Future<Output = rung_github::Result<()>> + Send {
                let should_fail = self.delete_should_fail;
                async move {
                    if should_fail {
                        Err(rung_github::Error::ApiError {
                            status: 404,
                            message: "ref not found".to_string(),
                        })
                    } else {
                        Ok(())
                    }
                }
            }

            fn get_default_branch(
                &self,
                _owner: &str,
                _repo: &str,
            ) -> impl std::future::Future<Output = rung_github::Result<String>> + Send {
                async { Ok("main".to_string()) }
            }

            fn list_pr_comments(
                &self,
                _owner: &str,
                _repo: &str,
                _pr_number: u64,
            ) -> impl std::future::Future<
                Output = rung_github::Result<Vec<rung_github::IssueComment>>,
            > + Send {
                async { Ok(vec![]) }
            }

            fn create_pr_comment(
                &self,
                _owner: &str,
                _repo: &str,
                _pr_number: u64,
                _comment: rung_github::CreateComment,
            ) -> impl std::future::Future<Output = rung_github::Result<rung_github::IssueComment>> + Send
            {
                async {
                    Ok(rung_github::IssueComment {
                        id: 1,
                        body: Some(String::new()),
                    })
                }
            }

            fn update_pr_comment(
                &self,
                _owner: &str,
                _repo: &str,
                _comment_id: u64,
                _comment: rung_github::UpdateComment,
            ) -> impl std::future::Future<Output = rung_github::Result<rung_github::IssueComment>> + Send
            {
                async {
                    Ok(rung_github::IssueComment {
                        id: 1,
                        body: Some(String::new()),
                    })
                }
            }
        }

        #[test]
        fn test_merge_service_creation() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("main", oid);
            let github = MockGitHubClient::new();

            let service = MergeService::new(&git, &github, "owner".to_string(), "repo".to_string());

            // Service should be created successfully
            assert_eq!(service.owner, "owner");
            assert_eq!(service.repo_name, "repo");
        }

        #[tokio::test]
        async fn test_validate_mergeable_success() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("main", oid);
            let github = MockGitHubClient::new();

            let service = MergeService::new(&git, &github, "owner".to_string(), "repo".to_string());

            let result = service.validate_mergeable(123).await;
            assert!(result.is_ok());
            let pr = result.unwrap();
            assert_eq!(pr.number, 123);
            assert_eq!(pr.mergeable, Some(true));
        }

        #[tokio::test]
        async fn test_validate_mergeable_not_mergeable() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("main", oid);
            let github = MockGitHubClient::new().with_unmergeable_pr();

            let service = MergeService::new(&git, &github, "owner".to_string(), "repo".to_string());

            let result = service.validate_mergeable(123).await;
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("not mergeable"));
        }

        #[tokio::test]
        #[ignore = "slow test - retries 5 times with 1s delay"]
        async fn test_validate_mergeable_unknown_status_retries() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("main", oid);
            let github = MockGitHubClient::new().with_unknown_mergeable();

            let service = MergeService::new(&git, &github, "owner".to_string(), "repo".to_string());

            let result = service.validate_mergeable(123).await;
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("unknown after"));
            assert!(err.contains("State: unknown"));
        }

        #[tokio::test]
        async fn test_merge_pr_success() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("main", oid);
            let github = MockGitHubClient::new();

            let service = MergeService::new(&git, &github, "owner".to_string(), "repo".to_string());

            let result = service.merge_pr(123, MergeMethod::Squash).await;
            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn test_merge_pr_failure() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("main", oid);
            let github = MockGitHubClient::new().with_merge_failure();

            let service = MergeService::new(&git, &github, "owner".to_string(), "repo".to_string());

            let result = service.merge_pr(123, MergeMethod::Squash).await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_delete_remote_branch_success() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("main", oid);
            let github = MockGitHubClient::new();

            let service = MergeService::new(&git, &github, "owner".to_string(), "repo".to_string());

            let result = service.delete_remote_branch("feature").await;
            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn test_delete_remote_branch_failure() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("main", oid);
            let github = MockGitHubClient::new().with_delete_failure();

            let service = MergeService::new(&git, &github, "owner".to_string(), "repo".to_string());

            let result = service.delete_remote_branch("feature").await;
            assert!(result.is_err());
        }

        #[test]
        #[allow(clippy::expect_used)]
        fn test_update_stack_after_merge() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("main", oid);
            let github = MockGitHubClient::new();

            let mut stack = Stack::default();
            // Parent branch must have a PR number to be added to merged list
            let mut parent_branch = StackBranch::try_new("feature/parent", None::<&str>).unwrap();
            parent_branch.pr = Some(10);
            stack.add_branch(parent_branch);
            stack
                .add_branch(StackBranch::try_new("feature/child", Some("feature/parent")).unwrap());

            let state = MockStateStore::new().with_stack(stack);

            let service = MergeService::new(&git, &github, "owner".to_string(), "repo".to_string());

            let children_count = service
                .update_stack_after_merge(&state, "feature/parent", "main")
                .expect("update should succeed");

            assert_eq!(children_count, 1);

            // Verify stack was updated
            let updated_stack = state.load_stack().unwrap();
            // The parent branch should be in merged list (only if it had a PR)
            assert!(
                updated_stack
                    .merged
                    .iter()
                    .any(|m| m.name.as_str() == "feature/parent")
            );
            // The child should now point to main
            let child = updated_stack.find_branch("feature/child").unwrap();
            assert_eq!(child.parent.as_ref().unwrap().as_str(), "main");
        }

        #[test]
        fn test_update_stack_after_merge_no_children() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("main", oid);
            let github = MockGitHubClient::new();

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/only", None::<&str>).unwrap());

            let state = MockStateStore::new().with_stack(stack);

            let service = MergeService::new(&git, &github, "owner".to_string(), "repo".to_string());

            let children_count = service
                .update_stack_after_merge(&state, "feature/only", "main")
                .expect("update should succeed");

            assert_eq!(children_count, 0);
        }

        #[tokio::test]
        #[allow(clippy::expect_used)]
        async fn test_shift_child_pr_bases() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("main", oid)
                .with_branch("feature/parent", oid)
                .with_branch("feature/child", oid);
            let github = MockGitHubClient::new();

            let mut stack = Stack::default();
            let mut parent_branch = StackBranch::try_new("feature/parent", None::<&str>).unwrap();
            parent_branch.pr = Some(10);
            stack.add_branch(parent_branch);

            let mut child_branch =
                StackBranch::try_new("feature/child", Some("feature/parent")).unwrap();
            child_branch.pr = Some(20);
            stack.add_branch(child_branch);

            let service = MergeService::new(&git, &github, "owner".to_string(), "repo".to_string());

            let descendants = vec!["feature/child".to_string()];
            let result = service
                .shift_child_pr_bases(&stack, "feature/parent", "main", &descendants)
                .await;

            assert!(result.is_ok());
            let shifted = result.unwrap();
            assert_eq!(shifted.len(), 1);
            assert_eq!(shifted[0].0, 20); // PR number
            assert_eq!(shifted[0].1, "feature/parent"); // Original base
        }

        #[tokio::test]
        async fn test_rollback_pr_bases() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("main", oid);
            let github = MockGitHubClient::new();

            let service = MergeService::new(&git, &github, "owner".to_string(), "repo".to_string());

            let shifted_prs = vec![(20, "feature/parent".to_string())];

            // This should not panic
            service.rollback_pr_bases(&shifted_prs).await;

            // Verify update_pr was called
            assert!(github.update_pr_called.load(Ordering::SeqCst));
        }
    }
}
