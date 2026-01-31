//! Status service for computing branch states and stack information.
//!
//! This module contains the logic for determining branch sync states
//! and divergence information, separated from CLI presentation concerns.

use anyhow::Result;
use rung_core::{BranchState, Stack, stack::StackBranch};
use rung_git::{GitOps, RemoteDivergence};
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
            branches: Vec::new(),
            current_branch: None,
        }
    }

    /// Check if the stack is empty.
    pub const fn is_empty(&self) -> bool {
        self.branches.is_empty()
    }
}

/// Service for computing stack and branch status with trait-based dependencies.
pub struct StatusService<'a, G: GitOps> {
    repo: &'a G,
    stack: &'a Stack,
}

impl<'a, G: GitOps> StatusService<'a, G> {
    /// Create a new status service.
    pub const fn new(repo: &'a G, stack: &'a Stack) -> Self {
        Self { repo, stack }
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

        // Check if parent is in stack but deleted from git
        if self.stack.find_branch(parent_name).is_some() && !self.repo.branch_exists(parent_name) {
            return Ok(BranchState::Detached);
        }

        // Check if external parent (like main) doesn't exist in repo
        if !self.repo.branch_exists(parent_name) {
            return Ok(BranchState::Detached);
        }

        // Check if the branch itself still exists
        if !self.repo.branch_exists(&branch.name) {
            return Ok(BranchState::Detached);
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
    use crate::services::test_mocks::MockGitOps;
    use rung_core::stack::StackBranch;
    use rung_core::{BranchName, BranchState};
    use rung_git::Oid;

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_status_service_compute_status_empty_stack() {
        let mock_repo = MockGitOps::new().with_current_branch("main");
        let stack = Stack::default();
        let service = StatusService::new(&mock_repo, &stack);

        let status = service.compute_status().unwrap();
        assert!(status.is_empty());
        assert_eq!(status.current_branch, Some("main".to_string()));
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_status_service_compute_status_with_branches() {
        let mock_repo = MockGitOps::new()
            .with_current_branch("feature/test")
            .with_branch("main", Oid::zero())
            .with_branch("feature/test", Oid::zero());

        let mut stack = Stack::default();
        let branch = StackBranch::new(BranchName::new("feature/test").unwrap(), None);
        stack.add_branch(branch);

        let service = StatusService::new(&mock_repo, &stack);

        let status = service.compute_status().unwrap();
        assert!(!status.is_empty());
        assert_eq!(status.branches.len(), 1);
        assert_eq!(status.branches[0].name, "feature/test");
        assert!(status.branches[0].is_current);
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_status_service_compute_branch_state_root_synced() {
        let mock_repo = MockGitOps::new().with_branch("feature/root", Oid::zero());

        let stack = Stack::default();
        let service = StatusService::new(&mock_repo, &stack);

        // Root branch (no parent) is always synced
        let branch = StackBranch::new(BranchName::new("feature/root").unwrap(), None);
        let state = service.compute_branch_state(&branch).unwrap();
        assert!(matches!(state, BranchState::Synced));
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_status_service_compute_branch_state_parent_missing() {
        let mock_repo = MockGitOps::new().with_branch("feature/child", Oid::zero());
        // Note: parent branch doesn't exist

        let stack = Stack::default();
        let service = StatusService::new(&mock_repo, &stack);

        let branch = StackBranch::new(
            BranchName::new("feature/child").unwrap(),
            Some(BranchName::new("deleted-parent").unwrap()),
        );
        let state = service.compute_branch_state(&branch).unwrap();
        assert!(matches!(state, BranchState::Detached));
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_status_service_compute_branch_state_branch_missing() {
        let mock_repo = MockGitOps::new().with_branch("main", Oid::zero());
        // Note: the branch itself doesn't exist, only parent

        let stack = Stack::default();
        let service = StatusService::new(&mock_repo, &stack);

        let branch = StackBranch::new(
            BranchName::new("deleted-branch").unwrap(),
            Some(BranchName::new("main").unwrap()),
        );
        let state = service.compute_branch_state(&branch).unwrap();
        assert!(matches!(state, BranchState::Detached));
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_status_service_compute_branch_state_synced_with_parent() {
        // When merge base equals parent commit, branch is synced
        let oid = Oid::zero();
        let mock_repo = MockGitOps::new()
            .with_branch("main", oid)
            .with_branch("feature/child", oid);

        let stack = Stack::default();
        let service = StatusService::new(&mock_repo, &stack);

        let branch = StackBranch::new(
            BranchName::new("feature/child").unwrap(),
            Some(BranchName::new("main").unwrap()),
        );
        let state = service.compute_branch_state(&branch).unwrap();
        assert!(matches!(state, BranchState::Synced));
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_status_service_multiple_branches() {
        let mock_repo = MockGitOps::new()
            .with_current_branch("feature/b")
            .with_branch("main", Oid::zero())
            .with_branch("feature/a", Oid::zero())
            .with_branch("feature/b", Oid::zero());

        let mut stack = Stack::default();
        let branch_a = StackBranch::new(BranchName::new("feature/a").unwrap(), None);
        let branch_b = StackBranch::new(
            BranchName::new("feature/b").unwrap(),
            Some(BranchName::new("feature/a").unwrap()),
        );
        stack.add_branch(branch_a);
        stack.add_branch(branch_b);

        let service = StatusService::new(&mock_repo, &stack);

        let status = service.compute_status().unwrap();
        assert_eq!(status.branches.len(), 2);
        assert!(!status.branches[0].is_current); // feature/a
        assert!(status.branches[1].is_current); // feature/b
    }

    #[test]
    fn test_stack_status_empty() {
        let status = StackStatus::empty();
        assert!(status.is_empty());
        assert!(status.current_branch.is_none());
    }

    #[test]
    fn test_stack_status_with_branches() {
        let status = StackStatus {
            branches: vec![BranchStatusInfo {
                name: "feature/test".to_string(),
                parent: Some("main".to_string()),
                state: BranchState::Synced,
                pr: Some(123),
                is_current: true,
                remote_divergence: Some(RemoteDivergenceInfo::InSync),
            }],
            current_branch: Some("feature/test".to_string()),
        };
        assert!(!status.is_empty());
        assert_eq!(status.branches.len(), 1);
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_branch_status_info_serializes() {
        let info = BranchStatusInfo {
            name: "feature/auth".to_string(),
            parent: Some("main".to_string()),
            state: BranchState::Synced,
            pr: Some(42),
            is_current: true,
            remote_divergence: Some(RemoteDivergenceInfo::Ahead { commits: 2 }),
        };
        let json = serde_json::to_string(&info).expect("serialization should succeed");
        assert!(json.contains("feature/auth"));
        assert!(json.contains("42"));
        assert!(json.contains("is_current"));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_branch_status_info_skips_false_current() {
        let info = BranchStatusInfo {
            name: "other".to_string(),
            parent: None,
            state: BranchState::Synced,
            pr: None,
            is_current: false,
            remote_divergence: None,
        };
        let json = serde_json::to_string(&info).expect("serialization should succeed");
        // is_current: false should be skipped
        assert!(!json.contains("is_current"));
        // remote_divergence: None should be skipped
        assert!(!json.contains("remote_divergence"));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_remote_divergence_serializes() {
        let ahead = RemoteDivergenceInfo::Ahead { commits: 5 };
        let json = serde_json::to_string(&ahead).expect("serialization should succeed");
        assert!(json.contains("ahead"));
        assert!(json.contains('5'));

        let diverged = RemoteDivergenceInfo::Diverged {
            ahead: 3,
            behind: 2,
        };
        let json = serde_json::to_string(&diverged).expect("serialization should succeed");
        assert!(json.contains("diverged"));
        assert!(json.contains('3'));
        assert!(json.contains('2'));
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

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_status_service_fetch_remote() {
        let mock_repo = MockGitOps::new();
        let stack = Stack::default();
        let service = StatusService::new(&mock_repo, &stack);

        // fetch_remote should succeed with mock (no-op)
        let result = service.fetch_remote();
        assert!(result.is_ok());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_status_service_compute_branch_state_parent_in_stack_but_deleted() {
        // Test the edge case where parent is in the stack but deleted from git
        let mock_repo = MockGitOps::new().with_branch("feature/child", Oid::zero());
        // Note: parent "feature/parent" does NOT exist in git

        let mut stack = Stack::default();
        // Add the parent branch to the stack (but it's deleted from git)
        let parent_branch = StackBranch::new(BranchName::new("feature/parent").unwrap(), None);
        let child_branch = StackBranch::new(
            BranchName::new("feature/child").unwrap(),
            Some(BranchName::new("feature/parent").unwrap()),
        );
        stack.add_branch(parent_branch);
        stack.add_branch(child_branch);

        let service = StatusService::new(&mock_repo, &stack);

        // Parent is in stack but deleted from git -> Detached
        let state = service.compute_branch_state(&stack.branches[1]).unwrap();
        assert!(matches!(state, BranchState::Detached));
    }
}
