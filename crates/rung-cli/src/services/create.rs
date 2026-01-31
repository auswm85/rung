//! Create service for branch creation and stack management.
//!
//! This module handles the logic for creating new branches in the stack,
//! separated from CLI presentation concerns.

use anyhow::{Context, Result};
use rung_core::{BranchName, Stack, StateStore, stack::StackBranch};
use rung_git::GitOps;

/// Result of a branch creation operation.
#[derive(Debug)]
pub struct CreateResult {
    /// The name of the created branch.
    pub branch_name: String,
    /// The parent branch name.
    pub parent_name: String,
    /// Whether a commit was created.
    pub commit_created: bool,
    /// The commit message if a commit was created.
    pub commit_message: Option<String>,
    /// Stack depth after creation.
    pub stack_depth: usize,
}

/// Service for creating branches in the stack with trait-based dependencies.
pub struct CreateService<'a, G: GitOps> {
    repo: &'a G,
}

impl<'a, G: GitOps> CreateService<'a, G> {
    /// Create a new create service.
    #[must_use]
    pub const fn new(repo: &'a G) -> Self {
        Self { repo }
    }

    /// Get the current branch name (will be the parent).
    pub fn current_branch(&self) -> Result<String> {
        Ok(self.repo.current_branch()?)
    }

    /// Check if a branch already exists.
    pub fn branch_exists(&self, name: &str) -> bool {
        self.repo.branch_exists(name)
    }

    /// Check if the working directory is clean.
    pub fn is_clean(&self) -> Result<bool> {
        Ok(self.repo.is_clean()?)
    }

    /// Check if there are staged changes ready to commit.
    pub fn has_staged_changes(&self) -> Result<bool> {
        Ok(self.repo.has_staged_changes()?)
    }

    /// Create a new branch in the stack.
    ///
    /// This will:
    /// 1. Create the git branch at current HEAD
    /// 2. Checkout the new branch
    /// 3. Optionally stage all changes and create a commit
    /// 4. Add it to the stack (only after git operations succeed)
    ///
    /// If any step fails after the branch is created, the branch is deleted
    /// to maintain consistency between git and stack state.
    pub fn create_branch<S: StateStore>(
        &self,
        state: &S,
        branch_name: &BranchName,
        parent: &BranchName,
        message: Option<&str>,
    ) -> Result<CreateResult> {
        let name = branch_name.as_str();
        let parent_str = parent.as_str();

        // Create the branch at current HEAD (parent's tip)
        self.repo.create_branch(name)?;

        // Checkout the new branch (rollback on failure)
        if let Err(e) = self.repo.checkout(name) {
            // Clean up: delete the branch we just created
            let _ = self.repo.delete_branch(name);
            return Err(anyhow::Error::from(e).context("Failed to checkout new branch"));
        }

        // Handle optional commit (rollback on failure)
        let (commit_created, commit_message) = if let Some(msg) = message {
            match self.create_initial_commit(msg) {
                Ok(result) => result,
                Err(e) => {
                    // Clean up: checkout parent and delete the branch
                    let _ = self.repo.checkout(parent_str);
                    let _ = self.repo.delete_branch(name);
                    return Err(e.context("Failed to create initial commit"));
                }
            }
        } else {
            (false, None)
        };

        // All git operations succeeded - now persist to stack
        let mut stack = match state.load_stack() {
            Ok(s) => s,
            Err(e) => {
                // Clean up: checkout parent and delete the branch
                let _ = self.repo.checkout(parent_str);
                let _ = self.repo.delete_branch(name);
                return Err(e.into());
            }
        };
        let branch = StackBranch::new(branch_name.clone(), Some(parent.clone()));
        stack.add_branch(branch);
        if let Err(e) = state.save_stack(&stack) {
            // Clean up: checkout parent and delete the branch
            let _ = self.repo.checkout(parent_str);
            let _ = self.repo.delete_branch(name);
            return Err(e.into());
        }

        // Calculate stack depth
        let stack_depth = stack.ancestry(name).len();

        Ok(CreateResult {
            branch_name: name.to_string(),
            parent_name: parent_str.to_string(),
            commit_created,
            commit_message,
            stack_depth,
        })
    }

    /// Stage all changes and create a commit if there are staged changes.
    fn create_initial_commit(&self, message: &str) -> Result<(bool, Option<String>)> {
        // Check for pre-staged changes first (user may have staged specific files)
        if self.repo.has_staged_changes()? {
            self.repo
                .create_commit(message)
                .context("Failed to create commit")?;
            return Ok((true, Some(message.to_string())));
        }

        // No staged changes - check if there are unstaged changes to stage
        if self.repo.is_clean()? {
            return Ok((false, None));
        }

        // Stage all unstaged changes
        self.repo.stage_all().context("Failed to stage changes")?;

        if self.repo.has_staged_changes()? {
            self.repo
                .create_commit(message)
                .context("Failed to create commit")?;
            Ok((true, Some(message.to_string())))
        } else {
            Ok((false, None))
        }
    }

    /// Get the stack for reading (useful for dry-run scenarios).
    #[allow(dead_code, clippy::unused_self)]
    pub fn load_stack<S: StateStore>(&self, state: &S) -> Result<Stack> {
        Ok(state.load_stack()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::test_mocks::{MockGitOps, MockStateStore};
    use rung_git::Oid;

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_create_service_current_branch() {
        let mock_repo = MockGitOps::new().with_current_branch("feature/test");
        let service = CreateService::new(&mock_repo);

        let result = service.current_branch().unwrap();
        assert_eq!(result, "feature/test");
    }

    #[test]
    fn test_create_service_branch_exists() {
        let mock_repo = MockGitOps::new().with_branch("existing-branch", Oid::zero());
        let service = CreateService::new(&mock_repo);

        assert!(service.branch_exists("existing-branch"));
        assert!(!service.branch_exists("non-existent"));
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_create_service_is_clean() {
        let mock_repo = MockGitOps::new().with_clean(true);
        let service = CreateService::new(&mock_repo);
        assert!(service.is_clean().unwrap());

        let mock_repo_dirty = MockGitOps::new().with_clean(false);
        let service_dirty = CreateService::new(&mock_repo_dirty);
        assert!(!service_dirty.is_clean().unwrap());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_create_service_has_staged_changes() {
        let mock_repo = MockGitOps::new().with_staged_changes(true);
        let service = CreateService::new(&mock_repo);
        assert!(service.has_staged_changes().unwrap());

        let mock_repo_no_staged = MockGitOps::new().with_staged_changes(false);
        let service_no_staged = CreateService::new(&mock_repo_no_staged);
        assert!(!service_no_staged.has_staged_changes().unwrap());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_create_branch_success() {
        let mock_repo = MockGitOps::new()
            .with_current_branch("main")
            .with_branch("main", Oid::zero());
        let mock_state = MockStateStore::new();

        let service = CreateService::new(&mock_repo);
        let branch_name = BranchName::new("feature/new").unwrap();
        let parent = BranchName::new("main").unwrap();

        let result = service
            .create_branch(&mock_state, &branch_name, &parent, None)
            .unwrap();

        assert_eq!(result.branch_name, "feature/new");
        assert_eq!(result.parent_name, "main");
        assert!(!result.commit_created);
        assert!(result.commit_message.is_none());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_create_branch_with_commit_when_staged() {
        let mock_repo = MockGitOps::new()
            .with_current_branch("main")
            .with_branch("main", Oid::zero())
            .with_staged_changes(true);
        let mock_state = MockStateStore::new();

        let service = CreateService::new(&mock_repo);
        let branch_name = BranchName::new("feature/with-commit").unwrap();
        let parent = BranchName::new("main").unwrap();

        let result = service
            .create_branch(&mock_state, &branch_name, &parent, Some("Initial commit"))
            .unwrap();

        assert_eq!(result.branch_name, "feature/with-commit");
        assert!(result.commit_created);
        assert_eq!(result.commit_message, Some("Initial commit".to_string()));
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_create_branch_with_message_clean_repo() {
        // When repo is clean and no staged changes, no commit is created
        let mock_repo = MockGitOps::new()
            .with_current_branch("main")
            .with_branch("main", Oid::zero())
            .with_clean(true)
            .with_staged_changes(false);
        let mock_state = MockStateStore::new();

        let service = CreateService::new(&mock_repo);
        let branch_name = BranchName::new("feature/clean").unwrap();
        let parent = BranchName::new("main").unwrap();

        let result = service
            .create_branch(&mock_state, &branch_name, &parent, Some("Message"))
            .unwrap();

        assert_eq!(result.branch_name, "feature/clean");
        assert!(!result.commit_created);
        assert!(result.commit_message.is_none());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_create_branch_with_message_dirty_repo() {
        // When repo is dirty but nothing staged, stage_all is called
        let mock_repo = MockGitOps::new()
            .with_current_branch("main")
            .with_branch("main", Oid::zero())
            .with_clean(false)
            .with_staged_changes(false);
        let mock_state = MockStateStore::new();

        let service = CreateService::new(&mock_repo);
        let branch_name = BranchName::new("feature/dirty").unwrap();
        let parent = BranchName::new("main").unwrap();

        // Note: mock stage_all doesn't actually stage anything, so no commit
        let result = service
            .create_branch(&mock_state, &branch_name, &parent, Some("Staged changes"))
            .unwrap();

        assert_eq!(result.branch_name, "feature/dirty");
        // Since mock doesn't actually stage, result depends on has_staged_changes
        assert!(!result.commit_created);
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_create_service_load_stack() {
        let mock_repo = MockGitOps::new();
        let mock_state = MockStateStore::new();

        let service = CreateService::new(&mock_repo);
        let stack = service.load_stack(&mock_state).unwrap();

        assert!(stack.is_empty());
    }

    #[test]
    fn test_create_result_fields() {
        let result = CreateResult {
            branch_name: "feature/test".to_string(),
            parent_name: "main".to_string(),
            commit_created: true,
            commit_message: Some("Initial commit".to_string()),
            stack_depth: 2,
        };

        assert_eq!(result.branch_name, "feature/test");
        assert_eq!(result.parent_name, "main");
        assert!(result.commit_created);
        assert_eq!(result.commit_message, Some("Initial commit".to_string()));
        assert_eq!(result.stack_depth, 2);
    }

    #[test]
    fn test_create_result_without_commit() {
        let result = CreateResult {
            branch_name: "feature/no-commit".to_string(),
            parent_name: "develop".to_string(),
            commit_created: false,
            commit_message: None,
            stack_depth: 1,
        };

        assert!(!result.commit_created);
        assert!(result.commit_message.is_none());
    }

    #[test]
    fn test_create_result_deep_stack() {
        let result = CreateResult {
            branch_name: "feature/deep".to_string(),
            parent_name: "feature/level2".to_string(),
            commit_created: false,
            commit_message: None,
            stack_depth: 5,
        };

        assert_eq!(result.stack_depth, 5);
        assert_eq!(result.parent_name, "feature/level2");
    }

    #[test]
    fn test_create_result_debug_impl() {
        let result = CreateResult {
            branch_name: "test".to_string(),
            parent_name: "main".to_string(),
            commit_created: true,
            commit_message: Some("msg".to_string()),
            stack_depth: 1,
        };
        // Test that Debug is implemented
        let debug_str = format!("{result:?}");
        assert!(debug_str.contains("test"));
        assert!(debug_str.contains("main"));
    }

    #[test]
    fn test_create_result_root_branch() {
        let result = CreateResult {
            branch_name: "first-branch".to_string(),
            parent_name: "main".to_string(),
            commit_created: true,
            commit_message: Some("Initial work".to_string()),
            stack_depth: 1,
        };

        assert_eq!(result.stack_depth, 1);
        assert!(result.commit_created);
    }

    #[test]
    fn test_create_result_long_commit_message() {
        let long_message = "This is a much longer commit message that spans multiple words and describes the changes in detail for testing purposes";
        let result = CreateResult {
            branch_name: "feature/detailed".to_string(),
            parent_name: "main".to_string(),
            commit_created: true,
            commit_message: Some(long_message.to_string()),
            stack_depth: 1,
        };

        assert_eq!(result.commit_message, Some(long_message.to_string()));
    }
}
