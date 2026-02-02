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
use thiserror::Error;

/// Errors specific to restack operations.
#[derive(Debug, Error)]
pub enum RestackError {
    /// A rebase conflict occurred with the specified files.
    #[error("Rebase conflict in '{branch}'")]
    Conflict { branch: String, files: Vec<String> },
    /// A general error occurred.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<rung_core::Error> for RestackError {
    fn from(err: rung_core::Error) -> Self {
        Self::Other(err.into())
    }
}

impl From<rung_git::Error> for RestackError {
    fn from(err: rung_git::Error) -> Self {
        Self::Other(err.into())
    }
}

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

            let diverged_records: Vec<DivergenceRecord> =
                plan.diverged.iter().map(DivergenceRecord::from).collect();

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
                diverged_branches: diverged_records,
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
    ///
    /// # Errors
    ///
    /// Returns `RestackError::Conflict` if a rebase conflict occurs, allowing
    /// callers to handle conflicts with typed pattern matching.
    pub fn execute_restack_loop<S: StateStore>(
        &self,
        state: &S,
        original_branch: &str,
    ) -> Result<RestackResult, RestackError> {
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
                    return Err(RestackError::Conflict {
                        branch: current_branch,
                        files,
                    });
                }
                Err(e) => {
                    self.restore_from_backup(state, &restack_state, original_branch);
                    return Err(RestackError::from(e));
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
    ) -> Result<RestackResult, RestackError> {
        // Update stack topology with transaction-like semantics:
        // 1. Modify in-memory, 2. Persist stack, 3. Mark updated, 4. Clear state
        if !restack_state.stack_updated {
            let mut stack = state.load_stack()?;
            stack.reparent(
                &restack_state.target_branch,
                Some(&restack_state.new_parent),
            )?;

            // Persist stack first - if this fails, restack state remains for recovery
            state.save_stack(&stack)?;

            // Only mark as updated after stack is persisted
            restack_state.mark_stack_updated();

            // Save restack state to record that stack was updated
            // If this fails, stack is saved but we may retry reparent (which is idempotent)
            state.save_restack_state(&restack_state)?;
        }

        // Only clear restack state after all updates are successfully persisted
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
    ///
    /// # Errors
    ///
    /// Returns `RestackError::Conflict` if a rebase conflict occurs, allowing
    /// callers to handle conflicts with typed pattern matching.
    pub fn continue_restack<S: StateStore>(
        &self,
        state: &S,
    ) -> Result<RestackResult, RestackError> {
        if !state.is_restack_in_progress() {
            return Err(RestackError::Other(anyhow::anyhow!(
                "No restack in progress to continue"
            )));
        }

        let mut restack_state = state.load_restack_state()?;

        // Detect stale state from crashed process
        if !self.repo.is_rebasing() && !restack_state.current_branch.is_empty() {
            return Err(RestackError::Other(anyhow::anyhow!(
                "Restack state exists but no rebase in progress (process may have crashed).\n\
                 Run `rung restack --abort` to clean up and restore branches."
            )));
        }

        let current_branch = restack_state.current_branch.clone();

        let original_branch = restack_state.original_branch.clone();

        // Continue the in-progress rebase
        match self.repo.rebase_continue() {
            Ok(()) => {
                restack_state.advance();
                state.save_restack_state(&restack_state)?;
                self.execute_restack_loop(state, &original_branch)
            }
            Err(rung_git::Error::RebaseConflict(files)) => Err(RestackError::Conflict {
                branch: current_branch,
                files,
            }),
            Err(e) => {
                self.restore_from_backup(state, &restack_state, &original_branch);
                Err(RestackError::from(e))
            }
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

    // Mock-based tests for RestackService methods
    #[allow(clippy::unwrap_used)]
    mod mock_tests {
        use super::*;
        use crate::services::test_mocks::{MockGitOps, MockStateStore};
        use rung_core::stack::{Stack, StackBranch};
        use rung_git::Oid;

        #[test]
        fn test_create_plan_branch_not_in_stack() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("main", oid);
            let state = MockStateStore::new();

            let service = RestackService::new(&git);
            let config = RestackConfig {
                target_branch: "nonexistent".to_string(),
                new_parent: "main".to_string(),
                include_children: false,
            };

            let result = service.create_plan(&state, &config);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("not in the stack"));
        }

        #[test]
        fn test_create_plan_new_parent_not_exists() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("feature/a", oid);

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/a", None::<&str>).unwrap());

            let state = MockStateStore::new().with_stack(stack);

            let service = RestackService::new(&git);
            let config = RestackConfig {
                target_branch: "feature/a".to_string(),
                new_parent: "nonexistent".to_string(),
                include_children: false,
            };

            let result = service.create_plan(&state, &config);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("does not exist"));
        }

        #[test]
        fn test_create_plan_noop_same_parent() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("main", oid)
                .with_branch("feature/a", oid);

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/a", Some("main")).unwrap());

            let state = MockStateStore::new().with_stack(stack);

            let service = RestackService::new(&git);
            let config = RestackConfig {
                target_branch: "feature/a".to_string(),
                new_parent: "main".to_string(), // Same as current parent
                include_children: false,
            };

            let plan = service.create_plan(&state, &config).unwrap();
            assert!(!plan.needs_rebase);
            assert!(plan.branches_to_rebase.is_empty());
        }

        #[test]
        fn test_create_plan_needs_rebase() {
            let oid1 = Oid::zero();
            // Different commit to simulate divergence
            let oid2 = Oid::from_str("1234567890123456789012345678901234567890").unwrap();
            let git = MockGitOps::new()
                .with_branch("main", oid1)
                .with_branch("develop", oid2)
                .with_branch("feature/a", oid1);

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/a", Some("main")).unwrap());

            let state = MockStateStore::new().with_stack(stack);

            let service = RestackService::new(&git);
            let config = RestackConfig {
                target_branch: "feature/a".to_string(),
                new_parent: "develop".to_string(),
                include_children: false,
            };

            let plan = service.create_plan(&state, &config).unwrap();
            assert_eq!(plan.target_branch, "feature/a");
            assert_eq!(plan.new_parent, "develop");
            assert_eq!(plan.old_parent, Some("main".to_string()));
            assert!(plan.branches_to_rebase.contains(&"feature/a".to_string()));
        }

        #[test]
        fn test_create_plan_with_children() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("main", oid)
                .with_branch("develop", oid)
                .with_branch("feature/parent", oid)
                .with_branch("feature/child", oid);

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/parent", Some("main")).unwrap());
            stack
                .add_branch(StackBranch::try_new("feature/child", Some("feature/parent")).unwrap());

            let state = MockStateStore::new().with_stack(stack);

            let service = RestackService::new(&git);
            let config = RestackConfig {
                target_branch: "feature/parent".to_string(),
                new_parent: "develop".to_string(),
                include_children: true,
            };

            let plan = service.create_plan(&state, &config).unwrap();
            assert!(
                plan.branches_to_rebase
                    .contains(&"feature/parent".to_string())
            );
            assert!(
                plan.branches_to_rebase
                    .contains(&"feature/child".to_string())
            );
            assert_eq!(plan.branches_to_rebase.len(), 2);
        }

        #[test]
        fn test_execute_no_rebase_needed() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("main", oid)
                .with_branch("feature/a", oid);

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/a", Some("develop")).unwrap());

            let state = MockStateStore::new().with_stack(stack);

            let service = RestackService::new(&git);
            let plan = RestackPlan {
                target_branch: "feature/a".to_string(),
                new_parent: "main".to_string(),
                old_parent: Some("develop".to_string()),
                branches_to_rebase: vec![],
                needs_rebase: false,
                diverged: vec![],
            };

            let result = service.execute(&state, &plan, "feature/a").unwrap();

            // Stack should be updated
            let updated_stack = state.load_stack().unwrap();
            let branch = updated_stack.find_branch("feature/a").unwrap();
            assert_eq!(branch.parent.as_deref(), Some("main"));

            // Completed should contain the target branch
            assert!(result.completed.contains(&"feature/a".to_string()));
        }

        #[test]
        fn test_abort_no_restack_in_progress() {
            let git = MockGitOps::new();
            let state = MockStateStore::new();

            let service = RestackService::new(&git);

            let result = service.abort(&state);
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("No restack in progress")
            );
        }

        #[test]
        fn test_continue_no_restack_in_progress() {
            let git = MockGitOps::new();
            let state = MockStateStore::new();

            let service = RestackService::new(&git);

            let result = service.continue_restack(&state);
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("No restack in progress")
            );
        }

        #[test]
        fn test_check_divergence_no_divergence() {
            let git = MockGitOps::new();
            let service = RestackService::new(&git);

            let diverged = service.check_divergence(&["feature/a".to_string()]);
            assert!(diverged.is_empty());
        }

        #[test]
        fn test_check_divergence_with_diverged_branches() {
            let git = MockGitOps::new();
            git.remote_divergence_map.borrow_mut().insert(
                "feature/diverged".to_string(),
                rung_git::RemoteDivergence::Diverged {
                    ahead: 3,
                    behind: 2,
                },
            );

            let service = RestackService::new(&git);

            let branches = vec!["feature/ok".to_string(), "feature/diverged".to_string()];
            let diverged = service.check_divergence(&branches);

            assert_eq!(diverged.len(), 1);
            assert_eq!(diverged[0].branch, "feature/diverged");
            assert_eq!(diverged[0].ahead, 3);
            assert_eq!(diverged[0].behind, 2);
        }

        #[test]
        fn test_create_plan_cycle_detection() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("feature/a", oid)
                .with_branch("feature/b", oid);

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/a", None::<&str>).unwrap());
            stack.add_branch(StackBranch::try_new("feature/b", Some("feature/a")).unwrap());

            let state = MockStateStore::new().with_stack(stack);

            let service = RestackService::new(&git);
            // Try to make feature/a depend on feature/b (which already depends on feature/a)
            let config = RestackConfig {
                target_branch: "feature/a".to_string(),
                new_parent: "feature/b".to_string(),
                include_children: false,
            };

            let result = service.create_plan(&state, &config);
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("would create a cycle")
            );
        }

        #[test]
        fn test_execute_restack_loop_completes_immediately() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("main", oid)
                .with_branch("develop", oid)
                .with_branch("feature/a", oid)
                .with_current_branch("feature/a");

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/a", Some("main")).unwrap());

            // Create a restack state that's already complete (empty branches_to_rebase)
            let restack_state = RestackState::new(
                "backup-123".to_string(),
                "feature/a".to_string(),
                "develop".to_string(),
                Some("main".to_string()),
                "feature/a".to_string(),
                vec![], // Empty = is_complete() returns true immediately
                vec![],
            );

            let state = MockStateStore::new()
                .with_stack(stack)
                .with_restack_state(restack_state);
            *state.restack_in_progress.borrow_mut() = true;

            let service = RestackService::new(&git);
            let result = service.execute_restack_loop(&state, "feature/a");

            // Should complete successfully since is_complete() is true
            assert!(result.is_ok());
            let restack_result = result.unwrap();
            assert_eq!(restack_result.target_branch, "feature/a");
        }

        #[test]
        fn test_execute_restack_loop_with_conflict() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("main", oid)
                .with_branch("develop", oid)
                .with_branch("feature/a", oid)
                .with_current_branch("feature/a")
                .with_rebase_failure(); // Rebase will fail

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/a", Some("main")).unwrap());

            // Create a restack state with work to do (feature/a needs rebasing)
            let restack_state = RestackState::new(
                "backup-123".to_string(),
                "feature/a".to_string(),
                "develop".to_string(),
                Some("main".to_string()),
                "feature/a".to_string(),
                vec!["feature/a".to_string()], // This branch needs rebasing
                vec![],
            );

            let state = MockStateStore::new()
                .with_stack(stack)
                .with_restack_state(restack_state);
            *state.restack_in_progress.borrow_mut() = true;

            let service = RestackService::new(&git);
            let result = service.execute_restack_loop(&state, "feature/a");

            // Should return a conflict error
            assert!(result.is_err());
            match result.unwrap_err() {
                RestackError::Conflict { branch, files } => {
                    assert_eq!(branch, "feature/a");
                    assert!(!files.is_empty());
                }
                other @ RestackError::Other(_) => {
                    panic!("Expected Conflict error, got: {other:?}")
                }
            }
        }

        #[test]
        fn test_abort_with_restack_in_progress() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("main", oid)
                .with_branch("feature/a", oid)
                .with_current_branch("feature/a");

            // Explicitly set the restack state so the test is deterministic
            let restack_state = RestackState::new(
                "backup-123".to_string(),
                "feature".to_string(), // target_branch
                "develop".to_string(),
                Some("main".to_string()),
                "feature".to_string(),
                vec![], // No branches to rebase
                vec![],
            );

            let state = MockStateStore::new().with_restack_state(restack_state);
            *state.restack_in_progress.borrow_mut() = true;

            let service = RestackService::new(&git);
            let result = service.abort(&state);

            // Should succeed
            assert!(result.is_ok());
            let abort_result = result.unwrap();
            // Result contains the info from the restack state
            assert_eq!(abort_result.target_branch, "feature");
            assert!(abort_result.branches_rebased.is_empty()); // Aborted, nothing rebased
        }

        #[test]
        fn test_continue_with_stale_state() {
            let git = MockGitOps::new()
                .with_branch("main", Oid::zero())
                .with_branch("feature/a", Oid::zero());
            // Note: is_rebasing is false but restack_in_progress is true

            // Create a restack state with a non-empty current_branch
            // This triggers the stale state detection when is_rebasing() is false
            let restack_state = RestackState::new(
                "backup-123".to_string(),
                "feature/a".to_string(),
                "develop".to_string(),
                Some("main".to_string()),
                "feature/a".to_string(),
                vec!["feature/a".to_string()], // current_branch will be "feature/a"
                vec![],
            );

            let state = MockStateStore::new().with_restack_state(restack_state);
            *state.restack_in_progress.borrow_mut() = true;

            let service = RestackService::new(&git);
            let result = service.continue_restack(&state);

            // Should error due to stale state (no rebase in progress but state says there is)
            assert!(result.is_err());
            let err_msg = result.unwrap_err().to_string();
            assert!(
                err_msg.contains("process may have crashed")
                    || err_msg.contains("no rebase in progress")
            );
        }

        #[test]
        fn test_restack_error_from_core_error() {
            let core_err = rung_core::Error::NotInitialized;
            let restack_err: RestackError = core_err.into();
            match restack_err {
                RestackError::Other(e) => {
                    assert!(e.to_string().contains("not initialized"));
                }
                RestackError::Conflict { .. } => panic!("Expected Other variant"),
            }
        }

        #[test]
        fn test_restack_error_from_git_error() {
            let git_err = rung_git::Error::BranchNotFound("test".to_string());
            let restack_err: RestackError = git_err.into();
            match restack_err {
                RestackError::Other(e) => {
                    assert!(e.to_string().contains("test"));
                }
                RestackError::Conflict { .. } => panic!("Expected Other variant"),
            }
        }

        #[test]
        fn test_restack_error_conflict_display() {
            let err = RestackError::Conflict {
                branch: "feature/test".to_string(),
                files: vec!["file1.rs".to_string(), "file2.rs".to_string()],
            };
            let display = err.to_string();
            assert!(display.contains("feature/test"));
            assert!(display.contains("Rebase conflict"));
        }

        #[test]
        fn test_execute_needs_rebase_creates_backup() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("main", oid)
                .with_branch("develop", oid)
                .with_branch("feature/a", oid);

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/a", Some("main")).unwrap());

            let state = MockStateStore::new().with_stack(stack);

            let service = RestackService::new(&git);
            let plan = RestackPlan {
                target_branch: "feature/a".to_string(),
                new_parent: "develop".to_string(),
                old_parent: Some("main".to_string()),
                branches_to_rebase: vec!["feature/a".to_string()],
                needs_rebase: true,
                diverged: vec![],
            };

            let result = service.execute(&state, &plan, "feature/a").unwrap();

            // Backup should be created
            assert!(!result.backup_id.is_empty());
            // Restack state should be saved
            assert!(state.is_restack_in_progress());
        }

        #[test]
        fn test_execute_with_diverged_branches() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("main", oid)
                .with_branch("develop", oid)
                .with_branch("feature/a", oid);

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/a", Some("main")).unwrap());

            let state = MockStateStore::new().with_stack(stack);

            let service = RestackService::new(&git);
            let plan = RestackPlan {
                target_branch: "feature/a".to_string(),
                new_parent: "develop".to_string(),
                old_parent: Some("main".to_string()),
                branches_to_rebase: vec!["feature/a".to_string()],
                needs_rebase: true,
                diverged: vec![DivergenceInfo {
                    branch: "feature/a".to_string(),
                    ahead: 2,
                    behind: 3,
                }],
            };

            let result = service.execute(&state, &plan, "feature/a").unwrap();

            // Diverged branches should be recorded in state
            assert_eq!(result.diverged_branches.len(), 1);
            assert_eq!(result.diverged_branches[0].branch, "feature/a");
        }
    }
}
