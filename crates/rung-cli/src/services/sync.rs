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
        match pr.state {
            PullRequestState::Merged => {
                merged_prs.push(ExternalMergeInfo {
                    branch_name: branch_name.to_string(),
                    pr_number,
                    merged_into: pr.base_branch.clone(),
                });
            }
            PullRequestState::Open => {
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
            PullRequestState::Closed => {
                // Closed PRs are ignored - they are neither merged nor candidates for reparenting
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
            if let Some(current_base) = current_states.get(&pr_number)
                && current_base == &new_base
            {
                continue;
            }

            let update = UpdatePullRequest {
                title: None,
                body: None,
                base: Some(new_base.clone()),
            };

            if let Err(e) = self
                .client
                .update_pr(&self.owner, &self.repo_name, pr_number, update)
                .await
            {
                eprintln!("Warning: Failed to update PR #{pr_number} base to '{new_base}': {e}");
            }
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
        // Closed PRs are ignored entirely, so no ghost parent even if base matches
        assert!(ghost_parents.is_empty());
    }

    #[test]
    fn test_process_pr_result_closed_with_mismatched_base() {
        // A closed PR with a mismatched base should still be ignored
        let pr = rung_github::PullRequest {
            number: 47,
            title: "Closed PR with wrong base".to_string(),
            body: None,
            state: PullRequestState::Closed,
            base_branch: "old-parent".to_string(), // Mismatched base
            head_branch: "feature/closed-mismatch".to_string(),
            html_url: "https://github.com/test/test/pull/47".to_string(),
            mergeable: None,
            mergeable_state: None,
            draft: false,
        };

        let mut merged_prs = Vec::new();
        let mut ghost_parents = Vec::new();

        SyncService::<rung_git::Repository, rung_github::GitHubClient>::process_pr_result(
            &pr,
            "feature/closed-mismatch",
            None,
            47,
            "main", // Expected base is "main" but PR has "old-parent"
            &mut merged_prs,
            &mut ghost_parents,
        );

        // Closed PRs are ignored - not in merged_prs
        assert!(merged_prs.is_empty());
        // Closed PRs are ignored - not in ghost_parents even with mismatched base
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

    // Tests using mock implementations
    #[allow(clippy::manual_async_fn, clippy::unwrap_used)]
    mod mock_tests {
        use super::super::*;
        use crate::services::test_mocks::{MockGitOps, MockStateStore};
        use rung_core::stack::{Stack, StackBranch};
        use rung_git::Oid;

        // Mock GitHubApi for testing
        struct MockGitHubClient;

        impl rung_github::GitHubApi for MockGitHubClient {
            fn get_pr(
                &self,
                _owner: &str,
                _repo: &str,
                number: u64,
            ) -> impl std::future::Future<Output = rung_github::Result<rung_github::PullRequest>> + Send
            {
                async move { Err(rung_github::Error::PrNotFound(number)) }
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
                async move { Err(rung_github::Error::PrNotFound(number)) }
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
                async {
                    Ok(rung_github::MergeResult {
                        sha: "abc123".to_string(),
                        merged: true,
                        message: "Merged".to_string(),
                    })
                }
            }

            fn delete_ref(
                &self,
                _owner: &str,
                _repo: &str,
                _ref_name: &str,
            ) -> impl std::future::Future<Output = rung_github::Result<()>> + Send {
                async { Ok(()) }
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
        fn test_push_stack_branches_empty_stack() {
            let git = MockGitOps::new();
            let state = MockStateStore::new();
            let client = MockGitHubClient;

            let service = SyncService::new(&git, &client, "owner".to_string(), "repo".to_string());
            let result = service.push_stack_branches(&state).unwrap();

            assert!(result.is_empty());
        }

        #[test]
        fn test_push_stack_branches_with_branches() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("feature/a", oid)
                .with_branch("feature/b", oid);

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/a", None::<&str>).unwrap());
            stack.add_branch(StackBranch::try_new("feature/b", Some("feature/a")).unwrap());

            let state = MockStateStore::new().with_stack(stack);
            let client = MockGitHubClient;

            let service = SyncService::new(&git, &client, "owner".to_string(), "repo".to_string());
            let result = service.push_stack_branches(&state).unwrap();

            assert_eq!(result.len(), 2);
            assert!(result.iter().all(|r| r.success));
        }

        #[test]
        fn test_push_stack_branches_with_push_failure() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("feature/a", oid)
                .with_branch("feature/b", oid)
                .with_push_result("feature/b", false);

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/a", None::<&str>).unwrap());
            stack.add_branch(StackBranch::try_new("feature/b", Some("feature/a")).unwrap());

            let state = MockStateStore::new().with_stack(stack);
            let client = MockGitHubClient;

            let service = SyncService::new(&git, &client, "owner".to_string(), "repo".to_string());
            let result = service.push_stack_branches(&state).unwrap();

            assert_eq!(result.len(), 2);
            assert!(result[0].success); // feature/a succeeds
            assert!(!result[1].success); // feature/b fails
        }

        #[test]
        fn test_push_stack_branches_skips_nonexistent() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("feature/a", oid);
            // Note: feature/b is NOT in git but IS in stack

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/a", None::<&str>).unwrap());
            stack.add_branch(StackBranch::try_new("feature/b", Some("feature/a")).unwrap());

            let state = MockStateStore::new().with_stack(stack);
            let client = MockGitHubClient;

            let service = SyncService::new(&git, &client, "owner".to_string(), "repo".to_string());
            let result = service.push_stack_branches(&state).unwrap();

            // Only feature/a should be pushed (feature/b doesn't exist in git)
            assert_eq!(result.len(), 1);
            assert_eq!(result[0].branch, "feature/a");
        }

        #[test]
        fn test_fetch_base_success() {
            let git = MockGitOps::new();
            let client = MockGitHubClient;

            let service = SyncService::new(&git, &client, "owner".to_string(), "repo".to_string());
            let result = service.fetch_base("main");

            assert!(result.is_ok());
        }

        #[test]
        fn test_sync_service_creation() {
            let git = MockGitOps::new();
            let client = MockGitHubClient;

            let service = SyncService::new(
                &git,
                &client,
                "test-owner".to_string(),
                "test-repo".to_string(),
            );

            // Service is created successfully - we can't access private fields
            // but we can verify it doesn't panic
            assert!(service.fetch_base("main").is_ok());
        }

        // Configurable mock for more complex scenarios
        struct ConfigurableMockGitHubClient {
            return_merged_prs: Vec<u64>,
            pr_base_branches: std::collections::HashMap<u64, String>,
        }

        impl ConfigurableMockGitHubClient {
            fn new() -> Self {
                Self {
                    return_merged_prs: Vec::new(),
                    pr_base_branches: std::collections::HashMap::new(),
                }
            }

            fn with_merged_pr(mut self, pr_number: u64) -> Self {
                self.return_merged_prs.push(pr_number);
                self
            }

            fn with_pr_base(mut self, pr_number: u64, base: &str) -> Self {
                self.pr_base_branches.insert(pr_number, base.to_string());
                self
            }
        }

        impl rung_github::GitHubApi for ConfigurableMockGitHubClient {
            fn get_pr(
                &self,
                _owner: &str,
                _repo: &str,
                number: u64,
            ) -> impl std::future::Future<Output = rung_github::Result<rung_github::PullRequest>> + Send
            {
                let is_merged = self.return_merged_prs.contains(&number);
                let base = self
                    .pr_base_branches
                    .get(&number)
                    .cloned()
                    .unwrap_or_else(|| "main".to_string());
                async move {
                    Ok(rung_github::PullRequest {
                        number,
                        title: format!("PR #{number}"),
                        body: None,
                        state: if is_merged {
                            rung_github::PullRequestState::Merged
                        } else {
                            rung_github::PullRequestState::Open
                        },
                        base_branch: base,
                        head_branch: format!("feature-{number}"),
                        html_url: format!("https://github.com/test/repo/pull/{number}"),
                        mergeable: Some(true),
                        mergeable_state: None,
                        draft: false,
                    })
                }
            }

            fn get_prs_batch(
                &self,
                _owner: &str,
                _repo: &str,
                numbers: &[u64],
            ) -> impl std::future::Future<
                Output = rung_github::Result<
                    std::collections::HashMap<u64, rung_github::PullRequest>,
                >,
            > + Send {
                let mut result = std::collections::HashMap::new();
                for &number in numbers {
                    let is_merged = self.return_merged_prs.contains(&number);
                    let base = self
                        .pr_base_branches
                        .get(&number)
                        .cloned()
                        .unwrap_or_else(|| "main".to_string());
                    result.insert(
                        number,
                        rung_github::PullRequest {
                            number,
                            title: format!("PR #{number}"),
                            body: None,
                            state: if is_merged {
                                rung_github::PullRequestState::Merged
                            } else {
                                rung_github::PullRequestState::Open
                            },
                            base_branch: base,
                            head_branch: format!("feature-{number}"),
                            html_url: format!("https://github.com/test/repo/pull/{number}"),
                            mergeable: Some(true),
                            mergeable_state: None,
                            draft: false,
                        },
                    );
                }
                async move { Ok(result) }
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
                async move {
                    Ok(rung_github::PullRequest {
                        number,
                        title: "Updated".to_string(),
                        body: None,
                        state: rung_github::PullRequestState::Open,
                        base_branch: "main".to_string(),
                        head_branch: "feature".to_string(),
                        html_url: format!("https://github.com/test/repo/pull/{number}"),
                        mergeable: Some(true),
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
                async {
                    Ok(rung_github::MergeResult {
                        sha: "abc123".to_string(),
                        merged: true,
                        message: "Merged".to_string(),
                    })
                }
            }

            fn delete_ref(
                &self,
                _owner: &str,
                _repo: &str,
                _ref_name: &str,
            ) -> impl std::future::Future<Output = rung_github::Result<()>> + Send {
                async { Ok(()) }
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

        #[tokio::test]
        async fn test_detect_and_reconcile_no_prs() {
            let git = MockGitOps::new();
            let client = ConfigurableMockGitHubClient::new();
            let state = MockStateStore::new();

            let service = SyncService::new(&git, &client, "owner".to_string(), "repo".to_string());

            let result = service.detect_and_reconcile_merged(&state, "main").await;
            assert!(result.is_ok());
            let reconciled = result.unwrap();
            assert!(reconciled.merged.is_empty());
            assert!(reconciled.reparented.is_empty());
            assert!(reconciled.repaired.is_empty());
        }

        #[tokio::test]
        async fn test_detect_and_reconcile_with_open_prs() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("feature/a", oid);
            let client = ConfigurableMockGitHubClient::new().with_pr_base(10, "main");

            let mut stack = Stack::default();
            let mut branch = StackBranch::try_new("feature/a", None::<&str>).unwrap();
            branch.pr = Some(10);
            stack.add_branch(branch);

            let state = MockStateStore::new().with_stack(stack);

            let service = SyncService::new(&git, &client, "owner".to_string(), "repo".to_string());

            let result = service.detect_and_reconcile_merged(&state, "main").await;
            assert!(result.is_ok());
            let reconciled = result.unwrap();
            // PR is open and base matches, so no changes
            assert!(reconciled.merged.is_empty());
            assert!(reconciled.reparented.is_empty());
        }

        #[tokio::test]
        async fn test_detect_and_reconcile_detects_merged_pr() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("feature/a", oid);
            let client = ConfigurableMockGitHubClient::new()
                .with_merged_pr(10)
                .with_pr_base(10, "main");

            let mut stack = Stack::default();
            let mut branch = StackBranch::try_new("feature/a", None::<&str>).unwrap();
            branch.pr = Some(10);
            stack.add_branch(branch);

            let state = MockStateStore::new().with_stack(stack);

            let service = SyncService::new(&git, &client, "owner".to_string(), "repo".to_string());

            let result = service.detect_and_reconcile_merged(&state, "main").await;
            assert!(result.is_ok());
            let reconciled = result.unwrap();
            // PR was merged, should be detected
            assert_eq!(reconciled.merged.len(), 1);
            assert_eq!(reconciled.merged[0].name, "feature/a");
        }

        #[tokio::test]
        async fn test_detect_ghost_parent() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("feature/a", oid);
            // PR base is "old-parent" but stack says it should be "main"
            let client = ConfigurableMockGitHubClient::new().with_pr_base(10, "old-parent");

            let mut stack = Stack::default();
            let mut branch = StackBranch::try_new("feature/a", None::<&str>).unwrap();
            branch.pr = Some(10);
            stack.add_branch(branch);

            let state = MockStateStore::new().with_stack(stack);

            let service = SyncService::new(&git, &client, "owner".to_string(), "repo".to_string());

            let result = service.detect_and_reconcile_merged(&state, "main").await;
            assert!(result.is_ok());
            let reconciled = result.unwrap();
            // Should detect ghost parent
            assert!(reconciled.merged.is_empty());
            assert_eq!(reconciled.repaired.len(), 1);
            assert_eq!(reconciled.repaired[0].name, "feature/a");
            assert_eq!(reconciled.repaired[0].old_parent, "old-parent");
            assert_eq!(reconciled.repaired[0].new_parent, "main");
        }

        #[tokio::test]
        async fn test_update_pr_bases_empty() {
            let git = MockGitOps::new();
            let client = ConfigurableMockGitHubClient::new();

            let service = SyncService::new(&git, &client, "owner".to_string(), "repo".to_string());

            let empty_result = rung_core::sync::ReconcileResult::default();
            let result = service.update_pr_bases(&empty_result).await;
            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn test_update_pr_bases_with_reparented() {
            let git = MockGitOps::new();
            let client = ConfigurableMockGitHubClient::new().with_pr_base(10, "old-parent");

            let service = SyncService::new(&git, &client, "owner".to_string(), "repo".to_string());

            let reconcile_result = rung_core::sync::ReconcileResult {
                merged: vec![],
                reparented: vec![rung_core::sync::ReparentedBranch {
                    name: "feature/a".to_string(),
                    old_parent: "old-parent".to_string(),
                    new_parent: "main".to_string(),
                    pr_number: Some(10),
                }],
                repaired: vec![],
            };

            let result = service.update_pr_bases(&reconcile_result).await;
            assert!(result.is_ok());
        }
    }
}
