//! Adopt service for bringing existing branches into the stack.
//!
//! This module handles the logic for adopting branches into the stack,
//! separated from CLI presentation concerns.

use anyhow::{Context, Result, bail};
use rung_core::{BranchName, State, stack::StackBranch};
use rung_git::Repository;

/// Result of an adopt operation.
#[derive(Debug)]
pub struct AdoptResult {
    /// The name of the adopted branch.
    pub branch_name: String,
    /// The parent branch name.
    pub parent_name: String,
    /// Stack depth after adoption.
    pub stack_depth: usize,
}

/// Service for adopting branches into the stack.
pub struct AdoptService<'a> {
    repo: &'a Repository,
    state: &'a State,
}

impl<'a> AdoptService<'a> {
    /// Create a new adopt service.
    pub const fn new(repo: &'a Repository, state: &'a State) -> Self {
        Self { repo, state }
    }

    /// Get the current branch name.
    pub fn current_branch(&self) -> Result<String> {
        Ok(self.repo.current_branch()?)
    }

    /// Check if a branch exists in git.
    pub fn branch_exists(&self, name: &str) -> bool {
        self.repo.branch_exists(name)
    }

    /// Check if a branch is already in the stack.
    pub fn is_in_stack(&self, name: &str) -> Result<bool> {
        let stack = self.state.load_stack()?;
        Ok(stack.find_branch(name).is_some())
    }

    /// Get the default/base branch name.
    pub fn default_branch(&self) -> Result<String> {
        Ok(self.state.default_branch()?)
    }

    /// Get available parent choices (base branch + stack branches).
    pub fn get_parent_choices(&self) -> Result<Vec<String>> {
        let base_branch = self.state.default_branch()?;
        let stack = self.state.load_stack()?;

        let mut choices = vec![base_branch];
        for b in &stack.branches {
            choices.push(b.name.to_string());
        }
        Ok(choices)
    }

    /// Validate that a parent is valid (exists and is either base or in stack).
    pub fn validate_parent(&self, parent_name: &str) -> Result<()> {
        let base_branch = self.state.default_branch()?;
        let stack = self.state.load_stack()?;

        let parent_is_base = parent_name == base_branch;
        let parent_in_stack = stack.find_branch(parent_name).is_some();

        if !parent_is_base && !parent_in_stack {
            if !self.repo.branch_exists(parent_name) {
                bail!("Parent branch '{parent_name}' does not exist");
            }
            bail!(
                "Parent branch '{parent_name}' is not in the stack. \
                 Add it first with `rung adopt {parent_name}` or use the base branch '{base_branch}'"
            );
        }

        Ok(())
    }

    /// Adopt a branch into the stack.
    pub fn adopt_branch(&self, branch_name: &BranchName, parent_name: &str) -> Result<AdoptResult> {
        let base_branch = self.state.default_branch()?;
        let mut stack = self.state.load_stack()?;

        // Determine parent (None if base branch)
        let parent_branch = if parent_name == base_branch {
            None
        } else {
            Some(BranchName::new(parent_name).context("Invalid parent branch name")?)
        };

        // Add to stack
        let branch = StackBranch::new(branch_name.clone(), parent_branch);
        stack.add_branch(branch);
        self.state.save_stack(&stack)?;

        // Calculate stack depth
        let stack_depth = stack.ancestry(branch_name.as_str()).len();

        Ok(AdoptResult {
            branch_name: branch_name.to_string(),
            parent_name: parent_name.to_string(),
            stack_depth,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adopt_result_fields() {
        let result = AdoptResult {
            branch_name: "feature/existing".to_string(),
            parent_name: "main".to_string(),
            stack_depth: 1,
        };

        assert_eq!(result.branch_name, "feature/existing");
        assert_eq!(result.parent_name, "main");
        assert_eq!(result.stack_depth, 1);
    }

    #[test]
    fn test_adopt_result_nested() {
        let result = AdoptResult {
            branch_name: "feature/child".to_string(),
            parent_name: "feature/parent".to_string(),
            stack_depth: 3,
        };

        assert_eq!(result.stack_depth, 3);
        assert_eq!(result.parent_name, "feature/parent");
    }

    #[test]
    fn test_adopt_result_debug_impl() {
        let result = AdoptResult {
            branch_name: "test".to_string(),
            parent_name: "main".to_string(),
            stack_depth: 1,
        };
        // Test that Debug is implemented
        let debug_str = format!("{result:?}");
        assert!(debug_str.contains("test"));
        assert!(debug_str.contains("main"));
    }

    #[test]
    fn test_adopt_result_root_level() {
        let result = AdoptResult {
            branch_name: "feature/new".to_string(),
            parent_name: "main".to_string(),
            stack_depth: 1,
        };

        // Root level adoption has depth 1
        assert_eq!(result.stack_depth, 1);
    }

    #[test]
    fn test_adopt_result_deep_stack() {
        let result = AdoptResult {
            branch_name: "feature/deep".to_string(),
            parent_name: "feature/level4".to_string(),
            stack_depth: 5,
        };

        assert_eq!(result.stack_depth, 5);
    }

    #[test]
    fn test_adopt_result_special_branch_names() {
        let result = AdoptResult {
            branch_name: "fix/issue-123".to_string(),
            parent_name: "hotfix/urgent".to_string(),
            stack_depth: 2,
        };

        assert_eq!(result.branch_name, "fix/issue-123");
        assert_eq!(result.parent_name, "hotfix/urgent");
    }
}
