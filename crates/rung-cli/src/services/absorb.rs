//! Absorb service for determining base branches and executing absorb operations.
//!
//! This module handles base branch detection and orchestrates the absorb
//! workflow, separated from CLI presentation concerns.

use anyhow::{Context, Result};
use rung_core::StateStore;
use rung_core::absorb::{self, AbsorbPlan, AbsorbResult};
use rung_git::{AbsorbOps, Repository};
use rung_github::{Auth, GitHubClient};

/// Service for absorb operations with trait-based dependencies.
pub struct AbsorbService<'a, G: AbsorbOps> {
    repo: &'a G,
}

impl<'a, G: AbsorbOps> AbsorbService<'a, G> {
    /// Create a new absorb service.
    #[must_use]
    pub const fn new(repo: &'a G) -> Self {
        Self { repo }
    }

    /// Check if there are staged changes to absorb.
    pub fn has_staged_changes(&self) -> Result<bool> {
        Ok(self.repo.has_staged_changes()?)
    }

    /// Detect the base branch by querying GitHub for the default branch.
    #[allow(clippy::future_not_send)] // Git operations are sync; future doesn't need to be Send
    pub async fn detect_base_branch(&self) -> Result<String> {
        let origin_url = self
            .repo
            .origin_url()
            .context("No origin remote configured")?;
        let (owner, repo_name) = Repository::parse_github_remote(&origin_url)
            .context("Could not parse GitHub remote URL")?;

        let client = GitHubClient::new(&Auth::auto()).context(
            "GitHub auth required to detect default branch. Use --base <branch> to specify manually.",
        )?;
        client
            .get_default_branch(&owner, &repo_name)
            .await
            .context("Could not fetch default branch. Use --base <branch> to specify manually.")
    }

    /// Create an absorb plan for the given base branch.
    pub fn create_plan<S: StateStore>(&self, state: &S, base_branch: &str) -> Result<AbsorbPlan> {
        Ok(absorb::create_absorb_plan(self.repo, state, base_branch)?)
    }

    /// Execute an absorb plan.
    pub fn execute_plan(&self, plan: &AbsorbPlan) -> Result<AbsorbResult> {
        Ok(absorb::execute_absorb(self.repo, plan)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::test_mocks::MockGitOps;
    use rung_git::{AbsorbOps, BlameResult, Hunk, Oid};

    /// Wrapper that implements `AbsorbOps` for testing.
    struct MockAbsorbOps {
        inner: MockGitOps,
        staged_changes: bool,
    }

    impl MockAbsorbOps {
        fn new() -> Self {
            Self {
                inner: MockGitOps::new(),
                staged_changes: false,
            }
        }

        fn with_staged_changes(mut self) -> Self {
            self.staged_changes = true;
            self
        }
    }

    impl rung_git::GitOps for MockAbsorbOps {
        fn workdir(&self) -> Option<&std::path::Path> {
            self.inner.workdir()
        }
        fn current_branch(&self) -> rung_git::Result<String> {
            self.inner.current_branch()
        }
        fn head_detached(&self) -> rung_git::Result<bool> {
            self.inner.head_detached()
        }
        fn is_rebasing(&self) -> bool {
            self.inner.is_rebasing()
        }
        fn branch_exists(&self, name: &str) -> bool {
            self.inner.branch_exists(name)
        }
        fn create_branch(&self, name: &str) -> rung_git::Result<Oid> {
            self.inner.create_branch(name)
        }
        fn checkout(&self, branch: &str) -> rung_git::Result<()> {
            self.inner.checkout(branch)
        }
        fn delete_branch(&self, name: &str) -> rung_git::Result<()> {
            self.inner.delete_branch(name)
        }
        fn list_branches(&self) -> rung_git::Result<Vec<String>> {
            self.inner.list_branches()
        }
        fn branch_commit(&self, branch: &str) -> rung_git::Result<Oid> {
            self.inner.branch_commit(branch)
        }
        fn remote_branch_commit(&self, branch: &str) -> rung_git::Result<Oid> {
            self.inner.remote_branch_commit(branch)
        }
        fn branch_commit_message(&self, branch: &str) -> rung_git::Result<String> {
            self.inner.branch_commit_message(branch)
        }
        fn merge_base(&self, one: Oid, two: Oid) -> rung_git::Result<Oid> {
            self.inner.merge_base(one, two)
        }
        fn commits_between(&self, from: Oid, to: Oid) -> rung_git::Result<Vec<Oid>> {
            self.inner.commits_between(from, to)
        }
        fn count_commits_between(&self, from: Oid, to: Oid) -> rung_git::Result<usize> {
            self.inner.count_commits_between(from, to)
        }
        fn is_clean(&self) -> rung_git::Result<bool> {
            self.inner.is_clean()
        }
        fn require_clean(&self) -> rung_git::Result<()> {
            self.inner.require_clean()
        }
        fn stage_all(&self) -> rung_git::Result<()> {
            self.inner.stage_all()
        }
        fn has_staged_changes(&self) -> rung_git::Result<bool> {
            Ok(self.staged_changes)
        }
        fn create_commit(&self, message: &str) -> rung_git::Result<Oid> {
            self.inner.create_commit(message)
        }
        fn rebase_onto(&self, target: Oid) -> rung_git::Result<()> {
            self.inner.rebase_onto(target)
        }
        fn rebase_onto_from(&self, onto: Oid, from: Oid) -> rung_git::Result<()> {
            self.inner.rebase_onto_from(onto, from)
        }
        fn conflicting_files(&self) -> rung_git::Result<Vec<String>> {
            self.inner.conflicting_files()
        }
        fn rebase_abort(&self) -> rung_git::Result<()> {
            self.inner.rebase_abort()
        }
        fn rebase_continue(&self) -> rung_git::Result<()> {
            self.inner.rebase_continue()
        }
        fn origin_url(&self) -> rung_git::Result<String> {
            self.inner.origin_url()
        }
        fn remote_divergence(&self, branch: &str) -> rung_git::Result<rung_git::RemoteDivergence> {
            self.inner.remote_divergence(branch)
        }
        fn detect_default_branch(&self) -> Option<String> {
            self.inner.detect_default_branch()
        }
        fn push(&self, branch: &str, force: bool) -> rung_git::Result<()> {
            self.inner.push(branch, force)
        }
        fn fetch_all(&self) -> rung_git::Result<()> {
            self.inner.fetch_all()
        }
        fn fetch(&self, branch: &str) -> rung_git::Result<()> {
            self.inner.fetch(branch)
        }
        fn pull_ff(&self) -> rung_git::Result<()> {
            self.inner.pull_ff()
        }
        fn reset_branch(&self, branch: &str, commit: Oid) -> rung_git::Result<()> {
            self.inner.reset_branch(branch, commit)
        }
    }

    impl AbsorbOps for MockAbsorbOps {
        fn staged_diff_hunks(&self) -> rung_git::Result<Vec<Hunk>> {
            Ok(vec![])
        }

        fn blame_lines(
            &self,
            _file_path: &str,
            _start: u32,
            _end: u32,
        ) -> rung_git::Result<Vec<BlameResult>> {
            Ok(vec![])
        }

        fn is_ancestor(&self, _ancestor: Oid, _descendant: Oid) -> rung_git::Result<bool> {
            Ok(true)
        }

        fn create_fixup_commit(&self, _target: Oid) -> rung_git::Result<Oid> {
            Ok(Oid::zero())
        }
    }

    #[test]
    fn test_absorb_service_creation() {
        let mock_repo = MockAbsorbOps::new();
        let _service = AbsorbService::new(&mock_repo);
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_absorb_service_has_staged_changes_false() {
        let mock_repo = MockAbsorbOps::new();
        let service = AbsorbService::new(&mock_repo);

        let has_changes = service.has_staged_changes().unwrap();
        assert!(!has_changes);
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_absorb_service_has_staged_changes_true() {
        let mock_repo = MockAbsorbOps::new().with_staged_changes();
        let service = AbsorbService::new(&mock_repo);

        let has_changes = service.has_staged_changes().unwrap();
        assert!(has_changes);
    }
}
