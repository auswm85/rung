//! Restack service for moving branches to different parents.
//!
//! This service encapsulates the business logic for the restack command,
//! accepting trait-based dependencies for testability.

use std::collections::VecDeque;

use anyhow::{Result, bail};
use chrono::Utc;
use rung_core::{DivergenceRecord, RestackState, StateStore};
use rung_git::{GitOps, Oid, RemoteDivergence};
use serde::Serialize;

/// Configuration for a restack operation.
#[derive(Debug, Clone)]
pub struct RestackConfig {
    pub target_branch: String,
    pub new_parent: String,
    pub include_children: bool,
}

/// Information about a diverged branch.
#[derive(Debug, Clone, Serialize)]
pub struct DivergenceInfo {
    pub branch: String,
    pub ahead: usize,
    pub behind: usize,
}

impl From<&DivergenceRecord> for DivergenceInfo {
    fn from(record: &DivergenceRecord) -> Self {
        Self {
            branch: record.branch.clone(),
            ahead: record.ahead,
            behind: record.behind,
        }
    }
}

impl From<&DivergenceInfo> for DivergenceRecord {
    fn from(info: &DivergenceInfo) -> Self {
        Self {
            branch: info.branch.clone(),
            ahead: info.ahead,
            behind: info.behind,
        }
    }
}

/// Result of a restack plan creation.
#[derive(Debug, Clone)]
pub struct RestackPlan {
    pub target_branch: String,
    pub new_parent: String,
    pub old_parent: Option<String>,
    pub branches_to_rebase: Vec<String>,
    pub needs_rebase: bool,
    pub diverged: Vec<DivergenceInfo>,
}

/// Result of a restack operation.
#[derive(Debug, Clone, Serialize)]
pub struct RestackResult {
    pub target_branch: String,
    pub old_parent: Option<String>,
    pub new_parent: String,
    pub branches_rebased: Vec<String>,
    pub diverged_branches: Vec<DivergenceInfo>,
}

/// Service for restack operations with trait-based dependencies.
pub struct RestackService<'a, G: GitOps> {
    repo: &'a G,
}

impl<'a, G: GitOps> RestackService<'a, G> {
    /// Create a new restack service.
    #[must_use]
    pub const fn new(repo: &'a G) -> Self {
        Self { repo }
    }

    /// Create a plan for a restack operation.
    pub fn create_plan<S: StateStore>(
        &self,
        state: &S,
        config: &RestackConfig,
    ) -> Result<RestackPlan> {
        let stack = state.load_stack()?;

        // Verify target branch is in the stack
        let branch_entry = stack.find_branch(&config.target_branch).ok_or_else(|| {
            anyhow::anyhow!("Branch '{}' is not in the stack", config.target_branch)
        })?;

        let old_parent = branch_entry.parent.as_ref().map(ToString::to_string);

        // Validate new parent exists
        let new_parent_in_stack = stack.find_branch(&config.new_parent).is_some();
        if !new_parent_in_stack && !self.repo.branch_exists(&config.new_parent) {
            bail!("Branch '{}' does not exist", config.new_parent);
        }

        // Check for cycle
        if stack.would_create_cycle(&config.target_branch, &config.new_parent) {
            bail!(
                "Cannot restack '{}' onto '{}': would create a cycle",
                config.target_branch,
                config.new_parent
            );
        }

        // Check if it's a no-op
        if old_parent.as_deref() == Some(&config.new_parent) {
            return Ok(RestackPlan {
                target_branch: config.target_branch.clone(),
                new_parent: config.new_parent.clone(),
                old_parent,
                branches_to_rebase: vec![],
                needs_rebase: false,
                diverged: vec![],
            });
        }

        // Check if rebase is actually needed via merge-base analysis
        let target_commit = self.repo.branch_commit(&config.target_branch)?;
        let new_parent_commit = self.repo.branch_commit(&config.new_parent)?;
        let merge_base = self.repo.merge_base(target_commit, new_parent_commit)?;

        let needs_rebase = merge_base != new_parent_commit;

        // Build list of branches to rebase
        let mut branches_to_rebase = vec![config.target_branch.clone()];
        if config.include_children {
            let descendants = stack.descendants(&config.target_branch);
            for desc in &descendants {
                branches_to_rebase.push(desc.name.to_string());
            }
        }

        // Check for divergence
        let diverged = self.check_divergence(&branches_to_rebase);

        Ok(RestackPlan {
            target_branch: config.target_branch.clone(),
            new_parent: config.new_parent.clone(),
            old_parent,
            branches_to_rebase,
            needs_rebase,
            diverged,
        })
    }

    /// Execute a restack plan.
    ///
    /// Returns the restack state for interruption recovery.
    pub fn execute<S: StateStore>(
        &self,
        state: &S,
        plan: &RestackPlan,
        original_branch: &str,
    ) -> Result<RestackState> {
        // If no rebase needed, just update the stack
        if !plan.needs_rebase {
            let mut stack = state.load_stack()?;
            stack.reparent(&plan.target_branch, Some(&plan.new_parent))?;
            state.save_stack(&stack)?;

            return Ok(RestackState {
                started_at: Utc::now(),
                backup_id: String::new(),
                target_branch: plan.target_branch.clone(),
                new_parent: plan.new_parent.clone(),
                old_parent: plan.old_parent.clone(),
                original_branch: original_branch.to_string(),
                current_branch: String::new(),
                completed: vec![plan.target_branch.clone()],
                remaining: VecDeque::new(),
                stack_updated: true,
                diverged_branches: vec![],
            });
        }

        // Create backup
        let mut backup_commits: Vec<String> = Vec::with_capacity(plan.branches_to_rebase.len());
        for branch_name in &plan.branches_to_rebase {
            let commit = self.repo.branch_commit(branch_name)?;
            backup_commits.push(commit.to_string());
        }
        let backup_refs: Vec<(&str, &str)> = plan
            .branches_to_rebase
            .iter()
            .zip(backup_commits.iter())
            .map(|(name, sha)| (name.as_str(), sha.as_str()))
            .collect();
        let backup_id = state.create_backup(&backup_refs)?;

        // Create restack state
        let diverged_records: Vec<DivergenceRecord> =
            plan.diverged.iter().map(DivergenceRecord::from).collect();

        let restack_state = RestackState::new(
            backup_id,
            plan.target_branch.clone(),
            plan.new_parent.clone(),
            plan.old_parent.clone(),
            original_branch.to_string(),
            plan.branches_to_rebase.clone(),
            diverged_records,
        );

        state.save_restack_state(&restack_state)?;

        Ok(restack_state)
    }

    /// Execute the restack loop (initial or continued).
    pub fn execute_restack_loop<S: StateStore>(
        &self,
        state: &S,
        original_branch: &str,
    ) -> Result<RestackResult> {
        let stack = state.load_stack()?;

        loop {
            let mut restack_state = state.load_restack_state()?;

            // Check if complete
            if restack_state.is_complete() {
                return self.finalize_restack(state, original_branch, restack_state);
            }

            // Process current branch
            let current_branch = restack_state.current_branch.clone();
            if current_branch.is_empty() {
                return self.finalize_restack(state, original_branch, restack_state);
            }

            // Checkout the branch
            self.repo.checkout(&current_branch)?;

            // Determine the rebase target
            let rebase_onto = if current_branch == restack_state.target_branch {
                restack_state.new_parent.clone()
            } else {
                stack
                    .find_branch(&current_branch)
                    .and_then(|b| b.parent.as_ref().map(ToString::to_string))
                    .unwrap_or_else(|| restack_state.target_branch.clone())
            };

            // Get the parent's current commit
            let parent_commit = self.repo.branch_commit(&rebase_onto)?;

            // Rebase onto the parent
            match self.repo.rebase_onto(parent_commit) {
                Ok(()) => {
                    restack_state.advance();
                    state.save_restack_state(&restack_state)?;
                }
                Err(rung_git::Error::RebaseConflict(files)) => {
                    state.save_restack_state(&restack_state)?;
                    bail!("Rebase conflict in '{current_branch}': {files:?}");
                }
                Err(e) => {
                    self.restore_from_backup(state, &restack_state, original_branch);
                    return Err(e.into());
                }
            }
        }
    }

    /// Finalize a completed restack operation.
    fn finalize_restack<S: StateStore>(
        &self,
        state: &S,
        original_branch: &str,
        mut restack_state: RestackState,
    ) -> Result<RestackResult> {
        // Update stack topology
        if !restack_state.stack_updated {
            let mut stack = state.load_stack()?;
            stack.reparent(
                &restack_state.target_branch,
                Some(&restack_state.new_parent),
            )?;
            state.save_stack(&stack)?;
            restack_state.mark_stack_updated();
            state.save_restack_state(&restack_state)?;
        }

        // Clear restack state
        state.clear_restack_state()?;

        // Restore original branch
        if original_branch != restack_state.target_branch {
            let _ = self.repo.checkout(original_branch);
        }

        let diverged_info: Vec<DivergenceInfo> = restack_state
            .diverged_branches
            .iter()
            .map(DivergenceInfo::from)
            .collect();

        Ok(RestackResult {
            target_branch: restack_state.target_branch,
            old_parent: restack_state.old_parent,
            new_parent: restack_state.new_parent,
            branches_rebased: restack_state.completed,
            diverged_branches: diverged_info,
        })
    }

    /// Restore branches from backup after a failure.
    fn restore_from_backup<S: StateStore>(
        &self,
        state: &S,
        restack_state: &RestackState,
        original_branch: &str,
    ) {
        let _ = self.repo.rebase_abort();
        if let Ok(refs) = state.load_backup(&restack_state.backup_id) {
            for (branch_name, sha) in refs {
                if let Ok(oid) = Oid::from_str(&sha) {
                    let _ = self.repo.reset_branch(&branch_name, oid);
                }
            }
        }
        let _ = self.repo.checkout(original_branch);
        let _ = state.clear_restack_state();
    }

    /// Handle --abort flag.
    pub fn abort<S: StateStore>(&self, state: &S) -> Result<RestackResult> {
        if !state.is_restack_in_progress() {
            bail!("No restack in progress to abort");
        }

        let restack_state = state.load_restack_state()?;

        // Abort any in-progress rebase
        if self.repo.is_rebasing() {
            let _ = self.repo.rebase_abort();
        }

        // Restore all branches from backup
        let refs = state.load_backup(&restack_state.backup_id)?;
        for (branch_name, sha) in refs {
            let oid = Oid::from_str(&sha)
                .map_err(|e| anyhow::anyhow!("Invalid backup ref for {branch_name}: {e}"))?;
            self.repo.reset_branch(&branch_name, oid)?;
        }

        // Restore original branch
        let _ = self.repo.checkout(&restack_state.original_branch);

        // Clear restack state
        state.clear_restack_state()?;

        Ok(RestackResult {
            target_branch: restack_state.target_branch,
            old_parent: restack_state.old_parent,
            new_parent: restack_state.new_parent,
            branches_rebased: vec![],
            diverged_branches: vec![],
        })
    }

    /// Handle --continue flag.
    pub fn continue_restack<S: StateStore>(&self, state: &S) -> Result<RestackResult> {
        if !state.is_restack_in_progress() {
            bail!("No restack in progress to continue");
        }

        let mut restack_state = state.load_restack_state()?;

        // Detect stale state from crashed process
        if !self.repo.is_rebasing() && !restack_state.current_branch.is_empty() {
            bail!(
                "Restack state exists but no rebase in progress (process may have crashed).\n\
                 Run `rung restack --abort` to clean up and restore branches."
            );
        }

        // Continue the in-progress rebase
        match self.repo.rebase_continue() {
            Ok(()) => {
                restack_state.advance();
                state.save_restack_state(&restack_state)?;
                self.execute_restack_loop(state, &restack_state.original_branch.clone())
            }
            Err(rung_git::Error::RebaseConflict(files)) => {
                bail!("Rebase conflict: {files:?}");
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Check if any branches have diverged from their remote tracking branches.
    fn check_divergence(&self, branches: &[String]) -> Vec<DivergenceInfo> {
        let mut diverged = Vec::new();

        for branch in branches {
            if let Ok(RemoteDivergence::Diverged { ahead, behind }) =
                self.repo.remote_divergence(branch)
            {
                diverged.push(DivergenceInfo {
                    branch: branch.clone(),
                    ahead,
                    behind,
                });
            }
        }

        diverged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_divergence_info_conversion() {
        let info = DivergenceInfo {
            branch: "test".to_string(),
            ahead: 2,
            behind: 3,
        };
        let record = DivergenceRecord::from(&info);
        assert_eq!(record.branch, "test");
        assert_eq!(record.ahead, 2);
        assert_eq!(record.behind, 3);
    }

    #[test]
    fn test_divergence_info_from_record() {
        let record = DivergenceRecord {
            branch: "feature/x".to_string(),
            ahead: 5,
            behind: 10,
        };
        let info = DivergenceInfo::from(&record);
        assert_eq!(info.branch, "feature/x");
        assert_eq!(info.ahead, 5);
        assert_eq!(info.behind, 10);
    }

    #[test]
    fn test_divergence_info_clone() {
        let info = DivergenceInfo {
            branch: "test".to_string(),
            ahead: 1,
            behind: 2,
        };
        let cloned = info.clone();
        assert_eq!(info.branch, cloned.branch);
        assert_eq!(info.ahead, cloned.ahead);
        assert_eq!(info.behind, cloned.behind);
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_divergence_info_serializes() {
        let info = DivergenceInfo {
            branch: "feature/test".to_string(),
            ahead: 3,
            behind: 5,
        };
        let json = serde_json::to_string(&info).expect("serialization should succeed");
        assert!(json.contains("feature/test"));
        assert!(json.contains('3'));
        assert!(json.contains('5'));
    }

    #[test]
    fn test_restack_config_creation() {
        let config = RestackConfig {
            target_branch: "feature/child".to_string(),
            new_parent: "main".to_string(),
            include_children: true,
        };
        assert_eq!(config.target_branch, "feature/child");
        assert_eq!(config.new_parent, "main");
        assert!(config.include_children);
    }

    #[test]
    fn test_restack_config_without_children() {
        let config = RestackConfig {
            target_branch: "feature/only".to_string(),
            new_parent: "develop".to_string(),
            include_children: false,
        };
        assert!(!config.include_children);
    }

    #[test]
    fn test_restack_config_clone() {
        let config = RestackConfig {
            target_branch: "test".to_string(),
            new_parent: "main".to_string(),
            include_children: true,
        };
        let cloned = config.clone();
        assert_eq!(config.target_branch, cloned.target_branch);
        assert_eq!(config.new_parent, cloned.new_parent);
        assert_eq!(config.include_children, cloned.include_children);
    }

    #[test]
    fn test_restack_plan_no_rebase_needed() {
        let plan = RestackPlan {
            target_branch: "feature/test".to_string(),
            new_parent: "main".to_string(),
            old_parent: Some("main".to_string()),
            branches_to_rebase: vec![],
            needs_rebase: false,
            diverged: vec![],
        };
        assert!(!plan.needs_rebase);
        assert!(plan.branches_to_rebase.is_empty());
    }

    #[test]
    fn test_restack_plan_with_children() {
        let plan = RestackPlan {
            target_branch: "feature/parent".to_string(),
            new_parent: "main".to_string(),
            old_parent: Some("develop".to_string()),
            branches_to_rebase: vec![
                "feature/parent".to_string(),
                "feature/child1".to_string(),
                "feature/child2".to_string(),
            ],
            needs_rebase: true,
            diverged: vec![],
        };
        assert!(plan.needs_rebase);
        assert_eq!(plan.branches_to_rebase.len(), 3);
    }

    #[test]
    fn test_restack_plan_with_diverged() {
        let plan = RestackPlan {
            target_branch: "feature/diverged".to_string(),
            new_parent: "main".to_string(),
            old_parent: None,
            branches_to_rebase: vec!["feature/diverged".to_string()],
            needs_rebase: true,
            diverged: vec![DivergenceInfo {
                branch: "feature/diverged".to_string(),
                ahead: 2,
                behind: 1,
            }],
        };
        assert_eq!(plan.diverged.len(), 1);
        assert_eq!(plan.diverged[0].ahead, 2);
    }

    #[test]
    fn test_restack_plan_clone() {
        let plan = RestackPlan {
            target_branch: "test".to_string(),
            new_parent: "main".to_string(),
            old_parent: Some("old".to_string()),
            branches_to_rebase: vec!["test".to_string()],
            needs_rebase: true,
            diverged: vec![],
        };
        let cloned = plan.clone();
        assert_eq!(plan.target_branch, cloned.target_branch);
        assert_eq!(plan.needs_rebase, cloned.needs_rebase);
    }

    #[test]
    fn test_restack_result_creation() {
        let result = RestackResult {
            target_branch: "feature/x".to_string(),
            old_parent: Some("develop".to_string()),
            new_parent: "main".to_string(),
            branches_rebased: vec!["feature/x".to_string()],
            diverged_branches: vec![],
        };
        assert_eq!(result.target_branch, "feature/x");
        assert_eq!(result.old_parent, Some("develop".to_string()));
        assert_eq!(result.new_parent, "main");
        assert_eq!(result.branches_rebased.len(), 1);
    }

    #[test]
    fn test_restack_result_with_diverged() {
        let result = RestackResult {
            target_branch: "test".to_string(),
            old_parent: None,
            new_parent: "main".to_string(),
            branches_rebased: vec!["test".to_string()],
            diverged_branches: vec![DivergenceInfo {
                branch: "test".to_string(),
                ahead: 1,
                behind: 2,
            }],
        };
        assert_eq!(result.diverged_branches.len(), 1);
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_restack_result_serializes() {
        let result = RestackResult {
            target_branch: "feature/serialize".to_string(),
            old_parent: Some("old".to_string()),
            new_parent: "new".to_string(),
            branches_rebased: vec!["feature/serialize".to_string()],
            diverged_branches: vec![],
        };
        let json = serde_json::to_string(&result).expect("serialization should succeed");
        assert!(json.contains("feature/serialize"));
        assert!(json.contains("old"));
        assert!(json.contains("new"));
    }

    #[test]
    fn test_restack_result_clone() {
        let result = RestackResult {
            target_branch: "test".to_string(),
            old_parent: None,
            new_parent: "main".to_string(),
            branches_rebased: vec!["test".to_string()],
            diverged_branches: vec![],
        };
        let cloned = result.clone();
        assert_eq!(result.target_branch, cloned.target_branch);
        assert_eq!(result.new_parent, cloned.new_parent);
    }

    #[test]
    fn test_restack_result_multiple_rebased() {
        let result = RestackResult {
            target_branch: "parent".to_string(),
            old_parent: Some("base".to_string()),
            new_parent: "main".to_string(),
            branches_rebased: vec![
                "parent".to_string(),
                "child1".to_string(),
                "child2".to_string(),
                "grandchild".to_string(),
            ],
            diverged_branches: vec![],
        };
        assert_eq!(result.branches_rebased.len(), 4);
        assert!(result.branches_rebased.contains(&"grandchild".to_string()));
    }
}
