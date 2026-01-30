//! Create service for branch creation and stack management.
//!
//! This module handles the logic for creating new branches in the stack,
//! separated from CLI presentation concerns.

use anyhow::{Context, Result};
use rung_core::{BranchName, Stack, State, stack::StackBranch};
use rung_git::Repository;

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

/// Service for creating branches in the stack.
pub struct CreateService<'a> {
    repo: &'a Repository,
    state: &'a State,
}

impl<'a> CreateService<'a> {
    /// Create a new create service.
    pub const fn new(repo: &'a Repository, state: &'a State) -> Self {
        Self { repo, state }
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

    /// Create a new branch in the stack.
    ///
    /// This will:
    /// 1. Create the git branch at current HEAD
    /// 2. Add it to the stack with the given parent
    /// 3. Checkout the new branch
    /// 4. Optionally stage all changes and create a commit
    pub fn create_branch(
        &self,
        branch_name: &BranchName,
        parent: &BranchName,
        message: Option<&str>,
    ) -> Result<CreateResult> {
        let name = branch_name.as_str();
        let parent_str = parent.as_str();

        // Create the branch at current HEAD (parent's tip)
        self.repo.create_branch(name)?;

        // Add to stack
        let mut stack = self.state.load_stack()?;
        let branch = StackBranch::new(branch_name.clone(), Some(parent.clone()));
        stack.add_branch(branch);
        self.state.save_stack(&stack)?;

        // Checkout the new branch
        self.repo.checkout(name)?;

        // Handle optional commit
        let (commit_created, commit_message) = if let Some(msg) = message {
            self.create_initial_commit(msg)?
        } else {
            (false, None)
        };

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
        // Check for changes before staging
        if self.repo.is_clean()? {
            return Ok((false, None));
        }

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
    #[allow(dead_code)]
    pub fn load_stack(&self) -> Result<Stack> {
        Ok(self.state.load_stack()?)
    }
}

#[cfg(test)]
mod tests {
    // Integration tests require a real git repository
    // Unit tests are limited since the service wraps external operations
}
