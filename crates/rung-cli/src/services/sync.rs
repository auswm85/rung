//! Sync service for rebasing the stack when base moves.
//!
//! This service encapsulates the business logic for the sync command,
//! accepting trait-based dependencies for testability.

#![allow(dead_code)] // Services not yet wired up to commands

use std::collections::HashMap;

use anyhow::Result;
use rung_core::StateStore;
use rung_core::stack::Stack;
use rung_core::sync::{
    self, ExternalMergeInfo, ReconcileResult, ReparentedBranch, StaleBranches, SyncPlan, SyncResult,
};
use rung_git::GitOps;
use rung_github::{GitHubApi, PullRequestState, UpdatePullRequest};
use serde::Serialize;

/// Threshold for switching from individual REST calls to batched GraphQL query.
const BATCH_THRESHOLD: usize = 5;

/// Configuration for a sync operation.
#[derive(Debug, Clone)]
pub struct SyncConfig {
    pub base_branch: String,
    pub no_push: bool,
}

/// Result of a sync plan creation.
#[derive(Debug)]
pub struct SyncPlanResult {
    pub reconcile_result: ReconcileResult,
    pub stale_result: StaleBranches,
    pub sync_plan: SyncPlan,
    pub stack: Stack,
}

/// Information about a branch that was pushed.
#[derive(Debug, Clone, Serialize)]
pub struct PushResult {
    pub branch: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

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
    /// Returns Ok(()) on success, or an error message on failure.
    pub fn fetch_base(&self, base_branch: &str) -> Result<(), String> {
        self.repo.fetch(base_branch).map_err(|e| e.to_string())
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
        Ok(sync::remove_stale_branches(self.repo, state)?)
    }

    /// Create a sync plan.
    pub fn create_sync_plan(&self, stack: &Stack, base_branch: &str) -> Result<SyncPlan> {
        Ok(sync::create_sync_plan(self.repo, stack, base_branch)?)
    }

    /// Execute a sync plan.
    pub fn execute_sync<S: StateStore>(&self, state: &S, plan: SyncPlan) -> Result<SyncResult> {
        Ok(sync::execute_sync(self.repo, state, plan)?)
    }

    /// Continue an in-progress sync.
    pub fn continue_sync<S: StateStore>(&self, state: &S) -> Result<SyncResult> {
        Ok(sync::continue_sync(self.repo, state)?)
    }

    /// Abort an in-progress sync.
    pub fn abort_sync<S: StateStore>(&self, state: &S) -> Result<()> {
        Ok(sync::abort_sync(self.repo, state)?)
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
    pub fn push_stack_branches<S: StateStore>(&self, state: &S) -> Result<Vec<PushResult>> {
        let stack = state.load_stack()?;
        let mut results = Vec::new();

        for branch in &stack.branches {
            if self.repo.branch_exists(&branch.name) {
                match self.repo.push(&branch.name, true) {
                    Ok(()) => {
                        results.push(PushResult {
                            branch: branch.name.to_string(),
                            success: true,
                            error: None,
                        });
                    }
                    Err(e) => {
                        results.push(PushResult {
                            branch: branch.name.to_string(),
                            success: false,
                            error: Some(e.to_string()),
                        });
                    }
                }
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_threshold() {
        assert_eq!(BATCH_THRESHOLD, 5);
    }
}
