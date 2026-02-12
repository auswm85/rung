//! Fold service for combining adjacent branches into one.
//!
//! This service encapsulates the business logic for the fold command,
//! which is the inverse of the split command.

use anyhow::{Context, Result, bail};
use rung_core::{FoldState, StackBranch, StateStore};
use rung_git::{Oid, Repository};
use serde::Serialize;

/// Information about a branch that can be folded.
#[derive(Debug, Clone, Serialize)]
pub struct FoldBranchInfo {
    /// Branch name.
    pub name: String,
    /// Number of commits in this branch.
    pub commit_count: usize,
    /// Associated PR number (if any).
    pub pr: Option<u64>,
}

/// Configuration for a fold operation.
#[derive(Debug, Clone)]
pub struct FoldConfig {
    /// The target branch that will contain all commits after folding.
    /// This is the bottommost branch in the chain being folded.
    pub target_branch: String,
    /// Branches being folded into the target (in parent-to-child order).
    /// Does NOT include the target branch itself.
    pub branches_to_fold: Vec<String>,
    /// The new parent for the target branch (parent of the topmost folded branch).
    pub new_parent: String,
}

/// Analysis of branches that can be folded.
#[derive(Debug, Clone)]
pub struct FoldAnalysis {
    /// The parent branch of the current branch.
    pub parent_branch: Option<String>,
    /// Children of the current branch that could be folded.
    pub children: Vec<FoldBranchInfo>,
}

/// Result of a fold operation.
#[derive(Debug, Clone, Serialize)]
pub struct FoldResult {
    /// The combined branch name.
    pub target_branch: String,
    /// Total number of commits in the combined branch.
    pub total_commits: usize,
    /// Branches that were folded (removed).
    pub branches_folded: Vec<String>,
    /// PRs that should be closed.
    pub prs_to_close: Vec<u64>,
}

/// Service for fold operations.
pub struct FoldService<'a> {
    repo: &'a Repository,
}

impl<'a> FoldService<'a> {
    /// Create a new fold service.
    #[must_use]
    pub const fn new(repo: &'a Repository) -> Self {
        Self { repo }
    }

    /// Analyze the current branch to determine what can be folded.
    pub fn analyze<S: StateStore>(&self, state: &S, branch_name: &str) -> Result<FoldAnalysis> {
        let stack = state.load_stack()?;
        let stack_branch = stack
            .find_branch(branch_name)
            .ok_or_else(|| anyhow::anyhow!("Branch '{branch_name}' not found in stack"))?;

        let parent_branch = stack_branch
            .parent
            .as_ref()
            .map(std::string::ToString::to_string);

        // Get children that could be folded
        let children = self.get_foldable_children(&stack, branch_name)?;

        Ok(FoldAnalysis {
            parent_branch,
            children,
        })
    }

    /// Get children of a branch that could be folded.
    fn get_foldable_children(
        &self,
        stack: &rung_core::Stack,
        branch_name: &str,
    ) -> Result<Vec<FoldBranchInfo>> {
        // For simplicity, only allow folding linear chains (single child at each level)
        let mut result = Vec::new();
        let mut current = branch_name;

        loop {
            let children = stack.children_of(current);
            if children.len() != 1 {
                // Stop if no children or multiple children (branching)
                break;
            }

            let child = children[0];
            let commit_count = self.count_branch_commits(child)?;
            result.push(FoldBranchInfo {
                name: child.name.to_string(),
                commit_count,
                pr: child.pr,
            });
            current = &child.name;
        }

        Ok(result)
    }

    /// Count the number of commits in a branch (between parent and branch tip).
    fn count_branch_commits(&self, branch: &StackBranch) -> Result<usize> {
        let Some(parent) = &branch.parent else {
            return Ok(0);
        };

        let parent_oid = self.repo.branch_commit(parent)?;
        let branch_oid = self.repo.branch_commit(&branch.name)?;
        let commits = self.repo.commits_between(parent_oid, branch_oid)?;
        Ok(commits.len())
    }

    /// Execute a fold operation.
    ///
    /// This combines multiple adjacent branches into one by:
    /// 1. Creating a backup of all involved branches
    /// 2. Resetting the target branch to include all commits
    /// 3. Removing the folded branches from the stack
    /// 4. Updating children to point to the target branch
    pub fn execute<S: StateStore>(&self, state: &S, config: &FoldConfig) -> Result<FoldResult> {
        let original_branch = self.repo.current_branch()?;
        let mut stack = state.load_stack()?;

        // Collect all branches involved and their commits for backup
        let mut backup_branches = vec![(
            config.target_branch.as_str(),
            self.repo.branch_commit(&config.target_branch)?.to_string(),
        )];

        for branch_name in &config.branches_to_fold {
            backup_branches.push((
                branch_name.as_str(),
                self.repo.branch_commit(branch_name)?.to_string(),
            ));
        }

        let backup_refs: Vec<(&str, &str)> = backup_branches
            .iter()
            .map(|(name, sha)| (*name, sha.as_str()))
            .collect();
        let backup_id = state.create_backup(&backup_refs)?;

        // Collect PRs to close
        let prs_to_close: Vec<u64> = config
            .branches_to_fold
            .iter()
            .filter_map(|name| stack.find_branch(name).and_then(|b| b.pr))
            .collect();

        // Initialize fold state for recovery (include original stack for abort)
        let original_stack_json =
            serde_json::to_string(&stack).context("Failed to serialize original stack")?;
        let mut fold_state = FoldState::new(
            backup_id.clone(),
            config.target_branch.clone(),
            config.branches_to_fold.clone(),
            config.new_parent.clone(),
            original_branch.clone(),
            prs_to_close.clone(),
        );
        fold_state.set_original_stack(original_stack_json);
        state.save_fold_state(&fold_state)?;

        // Execute the fold
        match self.execute_fold_inner(state, config, &mut stack, prs_to_close) {
            Ok(result) => {
                // Clean up state on success
                state.clear_fold_state()?;
                state.delete_backup(&backup_id)?;

                // Return to original branch if possible, otherwise target
                let checkout_branch = if config.branches_to_fold.contains(&original_branch) {
                    &config.target_branch
                } else if self.repo.branch_exists(&original_branch) {
                    &original_branch
                } else {
                    &config.target_branch
                };
                self.repo.checkout(checkout_branch)?;

                Ok(result)
            }
            Err(e) => {
                // On error, state remains for --abort
                Err(e)
            }
        }
    }

    /// Inner fold execution logic.
    fn execute_fold_inner<S: StateStore>(
        &self,
        state: &S,
        config: &FoldConfig,
        stack: &mut rung_core::Stack,
        prs_to_close: Vec<u64>,
    ) -> Result<FoldResult> {
        // Validate we have branches to fold
        if config.branches_to_fold.is_empty() {
            bail!("No branches specified to fold");
        }

        // The target branch will be reset to include all commits from folded branches.
        // Since branches are adjacent (parent-child chain), the tip of the last
        // branch in the chain contains all commits.
        let last_branch = &config.branches_to_fold[config.branches_to_fold.len() - 1];
        let final_commit = self.repo.branch_commit(last_branch)?;

        // Find any children of the last folded branch - they need to be reparented
        let children_to_reparent: Vec<String> = stack
            .children_of(last_branch)
            .iter()
            .map(|b| b.name.to_string())
            .collect();

        // Reset target branch to the final commit
        self.repo
            .reset_branch(&config.target_branch, final_commit)?;

        // Update target branch's parent to the new parent
        stack.reparent(&config.target_branch, Some(&config.new_parent))?;

        // Reparent any children of the last folded branch to the target
        for child in &children_to_reparent {
            stack.reparent(child, Some(&config.target_branch))?;
        }

        // Remove folded branches from stack
        let branches_folded: Vec<String> = config.branches_to_fold.clone();
        for branch_name in &branches_folded {
            stack.remove_branch(branch_name);
        }

        // Persist stack state first to ensure consistency
        // If git deletion fails later, stack is already correct and branches can be manually cleaned
        state.save_stack(stack)?;

        // Mark stack as updated so abort knows to restore it
        if let Ok(mut fold_state) = state.load_fold_state() {
            fold_state.mark_stack_updated();
            state
                .save_fold_state(&fold_state)
                .context("Failed to update fold state after stack modification")?;
        }

        // Now delete git branches (best-effort - log errors but continue)
        for branch_name in &branches_folded {
            if self.repo.branch_exists(branch_name)
                && let Err(e) = self.repo.delete_branch(branch_name)
            {
                eprintln!("Warning: Failed to delete branch '{branch_name}': {e}");
            }
        }

        // Count total commits
        let parent_oid = self.repo.branch_commit(&config.new_parent)?;
        let target_oid = self.repo.branch_commit(&config.target_branch)?;
        let total_commits = self.repo.commits_between(parent_oid, target_oid)?.len();

        Ok(FoldResult {
            target_branch: config.target_branch.clone(),
            total_commits,
            branches_folded,
            prs_to_close,
        })
    }

    /// Abort a fold operation and restore from backup.
    pub fn abort<S: StateStore>(&self, state: &S) -> Result<()> {
        if !state.is_fold_in_progress() {
            bail!("No fold in progress");
        }

        let fold_state = state.load_fold_state()?;

        // Restore stack if it was updated
        if fold_state.stack_updated
            && let Some(ref original_json) = fold_state.original_stack_json
        {
            let original_stack: rung_core::Stack = serde_json::from_str(original_json)
                .context("Failed to deserialize original stack")?;
            state.save_stack(&original_stack)?;
        }

        self.restore_from_backup(state, &fold_state)?;
        state.clear_fold_state()?;

        Ok(())
    }

    /// Restore branches from backup.
    fn restore_from_backup<S: StateStore>(&self, state: &S, fold_state: &FoldState) -> Result<()> {
        let backup_refs = state.load_backup(&fold_state.backup_id)?;

        // Validate all backup refs first
        let validated_refs: Vec<(String, Oid)> = backup_refs
            .iter()
            .map(|(branch_name, commit_sha)| {
                let oid = Oid::from_str(commit_sha).with_context(|| {
                    format!("Invalid commit SHA '{commit_sha}' for branch '{branch_name}'")
                })?;
                self.repo.find_commit(oid).with_context(|| {
                    format!("Commit {commit_sha} for branch '{branch_name}' not found")
                })?;
                Ok((branch_name.clone(), oid))
            })
            .collect::<Result<Vec<_>>>()?;

        // Recreate deleted branches and reset existing ones
        for (branch_name, oid) in &validated_refs {
            if !self.repo.branch_exists(branch_name) {
                self.repo.create_branch(branch_name)?;
            }
            self.repo.reset_branch(branch_name, *oid)?;
        }

        // Checkout original branch
        self.repo.checkout(&fold_state.original_branch)?;

        // Delete backup
        let _ = state.delete_backup(&fold_state.backup_id);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fold_branch_info() {
        let info = FoldBranchInfo {
            name: "feature/test".to_string(),
            commit_count: 3,
            pr: Some(42),
        };
        assert_eq!(info.name, "feature/test");
        assert_eq!(info.commit_count, 3);
        assert_eq!(info.pr, Some(42));
    }

    #[test]
    fn test_fold_config() {
        let config = FoldConfig {
            target_branch: "feature/base".to_string(),
            branches_to_fold: vec!["feature/child".to_string()],
            new_parent: "main".to_string(),
        };
        assert_eq!(config.target_branch, "feature/base");
        assert_eq!(config.branches_to_fold.len(), 1);
    }

    #[test]
    fn test_fold_result() {
        let result = FoldResult {
            target_branch: "feature/combined".to_string(),
            total_commits: 5,
            branches_folded: vec!["feature/a".to_string(), "feature/b".to_string()],
            prs_to_close: vec![42, 43],
        };
        assert_eq!(result.total_commits, 5);
        assert_eq!(result.branches_folded.len(), 2);
        assert_eq!(result.prs_to_close.len(), 2);
    }
}
