//! Log service for retrieving commits between branches.
//!
//! This module handles the logic for getting commit history between
//! a branch and its parent, separated from CLI presentation concerns.

use anyhow::{Result, bail};
use rung_core::{Stack, State};
use rung_git::Repository;
use serde::Serialize;

/// Information about a single commit.
#[derive(Debug, Clone, Serialize)]
pub struct CommitInfo {
    pub hash: String,
    pub message: String,
    pub author: String,
}

/// Complete log output for a branch.
#[derive(Debug, Clone, Serialize)]
pub struct LogResult {
    pub commits: Vec<CommitInfo>,
    pub branch: String,
    pub parent: String,
}

/// Service for retrieving commit logs.
pub struct LogService<'a> {
    repo: &'a Repository,
    state: &'a State,
}

impl<'a> LogService<'a> {
    /// Create a new log service.
    pub const fn new(repo: &'a Repository, state: &'a State) -> Self {
        Self { repo, state }
    }

    /// Get the current branch name.
    pub fn current_branch(&self) -> Result<String> {
        Ok(self.repo.current_branch()?)
    }

    /// Load the stack.
    pub fn load_stack(&self) -> Result<Stack> {
        Ok(self.state.load_stack()?)
    }

    /// Get commits between the current branch and its parent.
    pub fn get_branch_log(&self, branch_name: &str) -> Result<LogResult> {
        let stack = self.state.load_stack()?;

        let Some(head) = stack.find_branch(branch_name) else {
            bail!("Branch '{branch_name}' is not in stack")
        };

        let Some(parent) = &head.parent else {
            bail!("Branch '{branch_name}' has no parent branch")
        };

        let head_oid = self.repo.branch_commit(head.name.as_str())?;
        let base_oid = self.repo.branch_commit(parent.as_str())?;
        let commits = self.repo.commits_between(base_oid, head_oid)?;

        let commits_info: Result<Vec<CommitInfo>> = commits
            .iter()
            .map(|&oid| {
                let commit = self.repo.find_commit(oid)?;
                let id_str = commit.id().to_string();
                let hash = id_str.get(..7).unwrap_or(&id_str).to_owned();
                let message = commit.message().unwrap_or("").trim().to_owned();
                let sig = commit.author();
                let author = sig.name().unwrap_or("unknown").to_owned();

                Ok(CommitInfo {
                    hash,
                    message,
                    author,
                })
            })
            .collect();

        Ok(LogResult {
            commits: commits_info?,
            branch: branch_name.to_string(),
            parent: parent.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::expect_used)]
    fn test_commit_info_serializes() {
        let info = CommitInfo {
            hash: "abc1234".to_string(),
            message: "Test commit".to_string(),
            author: "Test Author".to_string(),
        };
        let json = serde_json::to_string(&info).expect("serialization should succeed");
        assert!(json.contains("abc1234"));
        assert!(json.contains("Test commit"));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_log_result_serializes() {
        let result = LogResult {
            commits: vec![
                CommitInfo {
                    hash: "abc1234".to_string(),
                    message: "First commit".to_string(),
                    author: "Alice".to_string(),
                },
                CommitInfo {
                    hash: "def5678".to_string(),
                    message: "Second commit".to_string(),
                    author: "Bob".to_string(),
                },
            ],
            branch: "feature/test".to_string(),
            parent: "main".to_string(),
        };

        let json = serde_json::to_string(&result).expect("serialization should succeed");
        assert!(json.contains("feature/test"));
        assert!(json.contains("main"));
        assert!(json.contains("First commit"));
        assert!(json.contains("Second commit"));
    }

    #[test]
    fn test_log_result_empty_commits() {
        let result = LogResult {
            commits: vec![],
            branch: "empty-branch".to_string(),
            parent: "main".to_string(),
        };

        assert!(result.commits.is_empty());
        assert_eq!(result.branch, "empty-branch");
    }

    #[test]
    fn test_commit_info_clone() {
        let info = CommitInfo {
            hash: "abc1234".to_string(),
            message: "Test".to_string(),
            author: "Author".to_string(),
        };
        let cloned = info.clone();
        assert_eq!(info.hash, cloned.hash);
        assert_eq!(info.message, cloned.message);
    }
}
