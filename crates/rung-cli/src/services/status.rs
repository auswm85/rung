//! Status service for computing branch states and stack information.
//!
//! This module contains the logic for determining branch sync states
//! and divergence information, separated from CLI presentation concerns.

use anyhow::Result;
use rung_core::{BranchState, Stack, State, stack::StackBranch};
use rung_git::{RemoteDivergence, Repository};
use serde::Serialize;

/// Computed information about a branch's status.
#[derive(Debug, Clone, Serialize)]
pub struct BranchStatusInfo {
    pub name: String,
    pub parent: Option<String>,
    pub state: BranchState,
    pub pr: Option<u64>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub is_current: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_divergence: Option<RemoteDivergenceInfo>,
}

/// Serializable remote divergence info.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RemoteDivergenceInfo {
    InSync,
    Ahead { commits: usize },
    Behind { commits: usize },
    Diverged { ahead: usize, behind: usize },
    NoRemote,
}

impl From<&RemoteDivergence> for RemoteDivergenceInfo {
    fn from(d: &RemoteDivergence) -> Self {
        match d {
            RemoteDivergence::InSync => Self::InSync,
            RemoteDivergence::Ahead { commits } => Self::Ahead { commits: *commits },
            RemoteDivergence::Behind { commits } => Self::Behind { commits: *commits },
            RemoteDivergence::Diverged { ahead, behind } => Self::Diverged {
                ahead: *ahead,
                behind: *behind,
            },
            RemoteDivergence::NoRemote => Self::NoRemote,
        }
    }
}

/// Complete status report for the stack.
#[derive(Debug, Clone, Serialize)]
pub struct StackStatus {
    pub branches: Vec<BranchStatusInfo>,
    pub current_branch: Option<String>,
}

impl StackStatus {
    /// Create an empty status (no branches in stack).
    #[allow(dead_code)]
    pub const fn empty() -> Self {
        Self {
            branches: vec![],
            current_branch: None,
        }
    }

    /// Check if the stack is empty.
    pub fn is_empty(&self) -> bool {
        self.branches.is_empty()
    }
}

/// Service for computing stack and branch status.
pub struct StatusService<'a> {
    repo: &'a Repository,
    #[allow(dead_code)]
    state: &'a State,
    stack: &'a Stack,
}

impl<'a> StatusService<'a> {
    /// Create a new status service.
    pub const fn new(repo: &'a Repository, state: &'a State, stack: &'a Stack) -> Self {
        Self { repo, state, stack }
    }

    /// Fetch latest from remote.
    pub fn fetch_remote(&self) -> Result<()> {
        self.repo.fetch_all()?;
        Ok(())
    }

    /// Compute the complete status of the stack.
    pub fn compute_status(&self) -> Result<StackStatus> {
        let current = self.repo.current_branch().ok();

        if self.stack.is_empty() {
            return Ok(StackStatus {
                branches: vec![],
                current_branch: current,
            });
        }

        let mut branches = Vec::with_capacity(self.stack.branches.len());

        for branch in &self.stack.branches {
            let state = self.compute_branch_state(branch)?;
            let remote_divergence = self
                .repo
                .remote_divergence(&branch.name)
                .ok()
                .map(|d| RemoteDivergenceInfo::from(&d));

            branches.push(BranchStatusInfo {
                name: branch.name.to_string(),
                parent: branch.parent.as_ref().map(ToString::to_string),
                state,
                pr: branch.pr,
                is_current: current.as_deref() == Some(branch.name.as_str()),
                remote_divergence,
            });
        }

        Ok(StackStatus {
            branches,
            current_branch: current,
        })
    }

    /// Compute the sync state of a branch relative to its parent.
    pub fn compute_branch_state(&self, branch: &StackBranch) -> Result<BranchState> {
        let Some(parent_name) = &branch.parent else {
            // Root branch, always synced
            return Ok(BranchState::Synced);
        };

        // Check if parent exists in repo
        if !self.repo.branch_exists(parent_name) {
            // Check if parent is in stack (might be deleted)
            if self.stack.find_branch(parent_name).is_some() {
                return Ok(BranchState::Detached);
            }
            // Parent is external (like main), check if it exists
            if !self.repo.branch_exists(parent_name) {
                return Ok(BranchState::Detached);
            }
        }

        // Get commits
        let branch_commit = self.repo.branch_commit(&branch.name)?;
        let parent_commit = self.repo.branch_commit(parent_name)?;

        // Find merge base
        let merge_base = self.repo.merge_base(branch_commit, parent_commit)?;

        // If merge base is the parent commit, we're synced
        if merge_base == parent_commit {
            return Ok(BranchState::Synced);
        }

        // Count how many commits behind
        let commits_behind = self.repo.count_commits_between(merge_base, parent_commit)?;

        Ok(BranchState::Diverged { commits_behind })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stack_status_empty() {
        let status = StackStatus::empty();
        assert!(status.is_empty());
        assert!(status.current_branch.is_none());
    }

    #[test]
    fn test_remote_divergence_from() {
        let in_sync = RemoteDivergenceInfo::from(&RemoteDivergence::InSync);
        assert!(matches!(in_sync, RemoteDivergenceInfo::InSync));

        let ahead = RemoteDivergenceInfo::from(&RemoteDivergence::Ahead { commits: 3 });
        assert!(matches!(ahead, RemoteDivergenceInfo::Ahead { commits: 3 }));

        let behind = RemoteDivergenceInfo::from(&RemoteDivergence::Behind { commits: 2 });
        assert!(matches!(
            behind,
            RemoteDivergenceInfo::Behind { commits: 2 }
        ));

        let diverged = RemoteDivergenceInfo::from(&RemoteDivergence::Diverged {
            ahead: 1,
            behind: 2,
        });
        assert!(matches!(
            diverged,
            RemoteDivergenceInfo::Diverged {
                ahead: 1,
                behind: 2
            }
        ));

        let no_remote = RemoteDivergenceInfo::from(&RemoteDivergence::NoRemote);
        assert!(matches!(no_remote, RemoteDivergenceInfo::NoRemote));
    }
}
