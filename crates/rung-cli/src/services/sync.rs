//! Sync service for rebasing the stack when base moves.
//!
//! This service encapsulates the business logic for the sync command,
//! accepting trait-based dependencies for testability.

use std::collections::HashMap;

use anyhow::Result;
use rung_core::StateStore;
use rung_core::stack::Stack;
use rung_core::sync::{
    self, ExternalMergeInfo, ReconcileResult, ReparentedBranch, StaleBranches, SyncPlan, SyncResult,
};
use rung_git::GitOps;
use rung_github::{GitHubApi, PullRequestState, UpdatePullRequest};

/// Threshold for switching from individual REST calls to batched GraphQL query.
const BATCH_THRESHOLD: usize = 5;

/// Service for sync operations with trait-based dependencies.
pub struct SyncService<'a, G: GitOps, H: GitHubApi> {
    repo: &'a G,
    client: &'a H,
    owner: String,
    repo_name: String,
}

#[allow(clippy::future_not_send)]
impl<'a, G: GitOps, H: GitHubApi> SyncService<'a, G, H> {
    /// Create a new sync service.
    #[must_use]
    pub const fn new(repo: &'a G, client: &'a H, owner: String, repo_name: String) -> Self {
        Self {
            repo,
            client,
            owner,
            repo_name,
        }
    }

    /// Fetch the base branch from remote.
    ///
    /// Note: Currently unused in CLI (fetch happens before GitHub client is available).
    /// Kept for API completeness and testability.
    #[allow(dead_code)]
    pub fn fetch_base(&self, base_branch: &str) -> Result<()> {
        self.repo
            .fetch(base_branch)
            .map_err(|e| anyhow::anyhow!("Failed to fetch {base_branch}: {e}"))
    }

    /// Detect merged PRs and validate PR bases.
    ///
    /// This performs two key operations:
    /// 1. Detects PRs that were merged externally (via GitHub UI)
    /// 2. Validates that each PR's base branch matches what stack.json expects
    pub async fn detect_and_reconcile_merged<S: StateStore>(
        &self,
        state: &S,
        base_branch: &str,
    ) -> Result<ReconcileResult> {
        let stack = state.load_stack()?;

        // Collect branches with PRs to check
        let branches_with_prs: Vec<_> = stack
            .branches
            .iter()
            .filter_map(|b| b.pr.map(|pr| (b.name.to_string(), b.parent.clone(), pr)))
            .collect();

        if branches_with_prs.is_empty() {
            return Ok(ReconcileResult::default());
        }

        let mut merged_prs = Vec::new();
        let mut ghost_parents = Vec::new();

        // Use batch fetch for larger stacks to reduce API calls
        if branches_with_prs.len() > BATCH_THRESHOLD {
            let pr_numbers: Vec<u64> = branches_with_prs.iter().map(|(_, _, pr)| *pr).collect();

            match self
                .client
                .get_prs_batch(&self.owner, &self.repo_name, &pr_numbers)
                .await
            {
                Ok(pr_map) => {
                    for (branch_name, stack_parent, pr_number) in &branches_with_prs {
                        if let Some(pr) = pr_map.get(pr_number) {
                            Self::process_pr_result(
                                pr,
                                branch_name,
                                stack_parent.as_ref(),
                                *pr_number,
                                base_branch,
                                &mut merged_prs,
                                &mut ghost_parents,
                            );
                        }
                    }
                }
                Err(_) => {
                    // Fall back to individual fetches
                    self.fetch_prs_individually(
                        &branches_with_prs,
                        base_branch,
                        &mut merged_prs,
                        &mut ghost_parents,
                    )
                    .await;
                }
            }
        } else {
            self.fetch_prs_individually(
                &branches_with_prs,
                base_branch,
                &mut merged_prs,
                &mut ghost_parents,
            )
            .await;
        }

        // If no merged PRs, just return with ghost parent repairs
        if merged_prs.is_empty() {
            return Ok(ReconcileResult {
                merged: vec![],
                reparented: vec![],
                repaired: ghost_parents,
            });
        }

        // Reconcile the stack for merged PRs
        let mut result = sync::reconcile_merged(state, &merged_prs)?;
        result.repaired = ghost_parents;

        Ok(result)
    }

    /// Fetch PRs individually using REST API.
    async fn fetch_prs_individually(
        &self,
        branches_with_prs: &[(String, Option<rung_core::BranchName>, u64)],
        base_branch: &str,
        merged_prs: &mut Vec<ExternalMergeInfo>,
        ghost_parents: &mut Vec<ReparentedBranch>,
    ) {
        for (branch_name, stack_parent, pr_number) in branches_with_prs {
            if let Ok(pr) = self
                .client
                .get_pr(&self.owner, &self.repo_name, *pr_number)
                .await
            {
                Self::process_pr_result(
                    &pr,
                    branch_name,
                    stack_parent.as_ref(),
                    *pr_number,
                    base_branch,
                    merged_prs,
                    ghost_parents,
                );
            }
        }
    }

    /// Process a fetched PR: detect merges and ghost parents.
    fn process_pr_result(
        pr: &rung_github::PullRequest,
        branch_name: &str,
        stack_parent: Option<&rung_core::BranchName>,
        pr_number: u64,
        base_branch: &str,
        merged_prs: &mut Vec<ExternalMergeInfo>,
        ghost_parents: &mut Vec<ReparentedBranch>,
    ) {
        if pr.state == PullRequestState::Merged {
            merged_prs.push(ExternalMergeInfo {
                branch_name: branch_name.to_string(),
                pr_number,
                merged_into: pr.base_branch.clone(),
            });
        } else {
            // PR is still open - validate its base matches our expectation
            let expected_base = stack_parent.map_or(base_branch, |p| p.as_str());

            if pr.base_branch != expected_base {
                ghost_parents.push(ReparentedBranch {
                    name: branch_name.to_string(),
                    old_parent: pr.base_branch.clone(),
                    new_parent: expected_base.to_string(),
                    pr_number: Some(pr_number),
                });
            }
        }
    }

    /// Remove stale branches from the stack.
    pub fn remove_stale_branches<S: StateStore>(&self, state: &S) -> Result<StaleBranches> {
        sync::remove_stale_branches(self.repo, state).map_err(Into::into)
    }

    /// Create a sync plan.
    pub fn create_sync_plan(&self, stack: &Stack, base_branch: &str) -> Result<SyncPlan> {
        sync::create_sync_plan(self.repo, stack, base_branch).map_err(Into::into)
    }

    /// Execute a sync plan.
    pub fn execute_sync<S: StateStore>(&self, state: &S, plan: SyncPlan) -> Result<SyncResult> {
        sync::execute_sync(self.repo, state, plan).map_err(Into::into)
    }

    /// Continue an in-progress sync.
    ///
    /// Note: Currently unused in CLI (continue is handled before GitHub client setup).
    /// Kept for API completeness and testability.
    #[allow(dead_code)]
    pub fn continue_sync<S: StateStore>(&self, state: &S) -> Result<SyncResult> {
        sync::continue_sync(self.repo, state).map_err(Into::into)
    }

    /// Abort an in-progress sync.
    ///
    /// Note: Currently unused in CLI (abort is handled before GitHub client setup).
    /// Kept for API completeness and testability.
    #[allow(dead_code)]
    pub fn abort_sync<S: StateStore>(&self, state: &S) -> Result<()> {
        sync::abort_sync(self.repo, state).map_err(Into::into)
    }

    /// Update GitHub PR base branches for reparented and repaired branches.
    pub async fn update_pr_bases(&self, reconcile_result: &ReconcileResult) -> Result<()> {
        // Collect all PRs that need updating
        let updates_needed: Vec<_> = reconcile_result
            .reparented
            .iter()
            .chain(reconcile_result.repaired.iter())
            .filter_map(|r| {
                r.pr_number
                    .map(|pr| (pr, r.new_parent.clone(), r.old_parent.clone()))
            })
            .collect();

        if updates_needed.is_empty() {
            return Ok(());
        }

        // Re-fetch current PR states to implement no-op check
        let pr_numbers: Vec<u64> = updates_needed.iter().map(|(pr, _, _)| *pr).collect();

        let current_states: HashMap<u64, String> = if pr_numbers.len() > BATCH_THRESHOLD {
            match self
                .client
                .get_prs_batch(&self.owner, &self.repo_name, &pr_numbers)
                .await
            {
                Ok(prs) => prs
                    .into_iter()
                    .map(|(num, pr)| (num, pr.base_branch))
                    .collect(),
                Err(_) => self.fetch_current_bases(&pr_numbers).await,
            }
        } else {
            self.fetch_current_bases(&pr_numbers).await
        };

        // Apply updates with no-op check
        for (pr_number, new_base, _old_base) in updates_needed {
            // No-op check: skip if PR base is already what we want
            if let Some(current_base) = current_states.get(&pr_number) {
                if current_base == &new_base {
                    continue;
                }
            }

            let update = UpdatePullRequest {
                title: None,
                body: None,
                base: Some(new_base),
            };

            let _ = self
                .client
                .update_pr(&self.owner, &self.repo_name, pr_number, update)
                .await;
        }

        Ok(())
    }

    /// Fetch current base branches for a list of PRs individually.
    async fn fetch_current_bases(&self, pr_numbers: &[u64]) -> HashMap<u64, String> {
        let mut result = HashMap::new();
        for &pr_number in pr_numbers {
            if let Ok(pr) = self
                .client
                .get_pr(&self.owner, &self.repo_name, pr_number)
                .await
            {
                result.insert(pr_number, pr.base_branch);
            }
        }
        result
    }

    /// Push all branches in the stack to remote.
    pub fn push_stack_branches<S: StateStore>(&self, state: &S) -> Result<Vec<PushInfo>> {
        let stack = state.load_stack()?;
        let mut results = Vec::new();

        for branch in &stack.branches {
            if self.repo.branch_exists(&branch.name) {
                match self.repo.push(&branch.name, true) {
                    Ok(()) => {
                        results.push(PushInfo {
                            branch: branch.name.to_string(),
                            success: true,
                        });
                    }
                    Err(_) => {
                        results.push(PushInfo {
                            branch: branch.name.to_string(),
                            success: false,
                        });
                    }
                }
            }
        }

        Ok(results)
    }
}

/// Information about a push operation.
#[derive(Debug, Clone)]
pub struct PushInfo {
    pub branch: String,
    pub success: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rung_core::sync::{ExternalMergeInfo, ReparentedBranch};
    use rung_github::PullRequestState;

    #[test]
    fn test_batch_threshold() {
        assert_eq!(BATCH_THRESHOLD, 5);
    }

    #[test]
    fn test_push_info_creation() {
        let info = PushInfo {
            branch: "feature/test".to_string(),
            success: true,
        };
        assert_eq!(info.branch, "feature/test");
        assert!(info.success);
    }

    #[test]
    fn test_push_info_failure() {
        let info = PushInfo {
            branch: "broken-branch".to_string(),
            success: false,
        };
        assert_eq!(info.branch, "broken-branch");
        assert!(!info.success);
    }

    #[test]
    fn test_push_info_clone() {
        let info = PushInfo {
            branch: "test".to_string(),
            success: true,
        };
        let cloned = info.clone();
        assert_eq!(info.branch, cloned.branch);
        assert_eq!(info.success, cloned.success);
    }

    #[test]
    fn test_process_pr_result_merged() {
        let pr = rung_github::PullRequest {
            number: 42,
            title: "Test PR".to_string(),
            body: None,
            state: PullRequestState::Merged,
            base_branch: "main".to_string(),
            head_branch: "feature/test".to_string(),
            html_url: "https://github.com/test/test/pull/42".to_string(),
            mergeable: None,
            mergeable_state: None,
            draft: false,
        };

        let mut merged_prs = Vec::new();
        let mut ghost_parents = Vec::new();

        SyncService::<rung_git::Repository, rung_github::GitHubClient>::process_pr_result(
            &pr,
            "feature/test",
            None,
            42,
            "main",
            &mut merged_prs,
            &mut ghost_parents,
        );

        assert_eq!(merged_prs.len(), 1);
        assert_eq!(merged_prs[0].branch_name, "feature/test");
        assert_eq!(merged_prs[0].pr_number, 42);
        assert_eq!(merged_prs[0].merged_into, "main");
        assert!(ghost_parents.is_empty());
    }

    #[test]
    fn test_process_pr_result_open_matching_base() {
        let pr = rung_github::PullRequest {
            number: 43,
            title: "Open PR".to_string(),
            body: None,
            state: PullRequestState::Open,
            base_branch: "main".to_string(),
            head_branch: "feature/open".to_string(),
            html_url: "https://github.com/test/test/pull/43".to_string(),
            mergeable: Some(true),
            mergeable_state: None,
            draft: false,
        };

        let mut merged_prs = Vec::new();
        let mut ghost_parents = Vec::new();

        SyncService::<rung_git::Repository, rung_github::GitHubClient>::process_pr_result(
            &pr,
            "feature/open",
            None,
            43,
            "main",
            &mut merged_prs,
            &mut ghost_parents,
        );

        assert!(merged_prs.is_empty());
        assert!(ghost_parents.is_empty());
    }

    #[test]
    fn test_process_pr_result_ghost_parent_detected() {
        let pr = rung_github::PullRequest {
            number: 44,
            title: "Ghost Parent PR".to_string(),
            body: None,
            state: PullRequestState::Open,
            base_branch: "old-parent".to_string(),
            head_branch: "feature/ghost".to_string(),
            html_url: "https://github.com/test/test/pull/44".to_string(),
            mergeable: Some(true),
            mergeable_state: None,
            draft: false,
        };

        let mut merged_prs = Vec::new();
        let mut ghost_parents = Vec::new();

        SyncService::<rung_git::Repository, rung_github::GitHubClient>::process_pr_result(
            &pr,
            "feature/ghost",
            None,
            44,
            "main",
            &mut merged_prs,
            &mut ghost_parents,
        );

        assert!(merged_prs.is_empty());
        assert_eq!(ghost_parents.len(), 1);
        assert_eq!(ghost_parents[0].name, "feature/ghost");
        assert_eq!(ghost_parents[0].old_parent, "old-parent");
        assert_eq!(ghost_parents[0].new_parent, "main");
        assert_eq!(ghost_parents[0].pr_number, Some(44));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_process_pr_result_with_stack_parent() {
        use rung_core::BranchName;

        let pr = rung_github::PullRequest {
            number: 45,
            title: "Stacked PR".to_string(),
            body: None,
            state: PullRequestState::Open,
            base_branch: "feature/base".to_string(),
            head_branch: "feature/child".to_string(),
            html_url: "https://github.com/test/test/pull/45".to_string(),
            mergeable: Some(true),
            mergeable_state: None,
            draft: false,
        };

        let mut merged_prs = Vec::new();
        let mut ghost_parents = Vec::new();
        let stack_parent = BranchName::new("feature/base").expect("valid branch name");

        SyncService::<rung_git::Repository, rung_github::GitHubClient>::process_pr_result(
            &pr,
            "feature/child",
            Some(&stack_parent),
            45,
            "main",
            &mut merged_prs,
            &mut ghost_parents,
        );

        // Base matches stack parent, so no ghost parent
        assert!(merged_prs.is_empty());
        assert!(ghost_parents.is_empty());
    }

    #[test]
    fn test_process_pr_result_closed_not_merged() {
        let pr = rung_github::PullRequest {
            number: 46,
            title: "Closed PR".to_string(),
            body: None,
            state: PullRequestState::Closed,
            base_branch: "main".to_string(),
            head_branch: "feature/closed".to_string(),
            html_url: "https://github.com/test/test/pull/46".to_string(),
            mergeable: None,
            mergeable_state: None,
            draft: false,
        };

        let mut merged_prs = Vec::new();
        let mut ghost_parents = Vec::new();

        SyncService::<rung_git::Repository, rung_github::GitHubClient>::process_pr_result(
            &pr,
            "feature/closed",
            None,
            46,
            "main",
            &mut merged_prs,
            &mut ghost_parents,
        );

        // Closed but not merged - should not be in merged_prs
        assert!(merged_prs.is_empty());
        // Also no ghost parent since base matches
        assert!(ghost_parents.is_empty());
    }

    #[test]
    fn test_external_merge_info_fields() {
        let info = ExternalMergeInfo {
            branch_name: "feature/merged".to_string(),
            pr_number: 100,
            merged_into: "main".to_string(),
        };
        assert_eq!(info.branch_name, "feature/merged");
        assert_eq!(info.pr_number, 100);
        assert_eq!(info.merged_into, "main");
    }

    #[test]
    fn test_reparented_branch_fields() {
        let reparent = ReparentedBranch {
            name: "feature/moved".to_string(),
            old_parent: "old-branch".to_string(),
            new_parent: "new-branch".to_string(),
            pr_number: Some(99),
        };
        assert_eq!(reparent.name, "feature/moved");
        assert_eq!(reparent.old_parent, "old-branch");
        assert_eq!(reparent.new_parent, "new-branch");
        assert_eq!(reparent.pr_number, Some(99));
    }

    #[test]
    fn test_reparented_branch_without_pr() {
        let reparent = ReparentedBranch {
            name: "local-only".to_string(),
            old_parent: "old".to_string(),
            new_parent: "new".to_string(),
            pr_number: None,
        };
        assert!(reparent.pr_number.is_none());
    }
}
