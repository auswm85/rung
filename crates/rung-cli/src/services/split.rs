//! Split service for dividing a branch into multiple stacked branches.
//!
//! This service encapsulates the business logic for the split command.

use anyhow::{Context, Result, bail};
use rung_core::{SplitPoint, SplitState, StateStore};
use rung_git::{Oid, Repository};
use serde::Serialize;

/// Information about a commit that can be selected for splitting.
#[derive(Debug, Clone, Serialize)]
pub struct CommitInfo {
    /// The commit SHA.
    pub oid: String,
    /// Short SHA for display.
    pub short_sha: String,
    /// Commit summary (first line of message).
    pub summary: String,
}

/// Configuration for a split operation.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Used in Phase 4
pub struct SplitConfig {
    /// The branch to split.
    pub source_branch: String,
    /// The parent branch.
    pub parent_branch: String,
    /// Split points defining where to create new branches.
    pub split_points: Vec<SplitPoint>,
}

/// Result of analyzing a branch for splitting.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields used in Phase 3/4
pub struct SplitAnalysis {
    /// The branch being analyzed.
    pub source_branch: String,
    /// The parent branch.
    pub parent_branch: String,
    /// Commits available for splitting (oldest first).
    pub commits: Vec<CommitInfo>,
}

/// Result of a split operation.
#[derive(Debug, Clone, Serialize)]
pub struct SplitResult {
    /// The original branch that was split.
    pub source_branch: String,
    /// Branches that were created.
    pub branches_created: Vec<String>,
}

/// Service for split operations.
pub struct SplitService<'a> {
    repo: &'a Repository,
}

impl<'a> SplitService<'a> {
    /// Create a new split service.
    #[must_use]
    pub const fn new(repo: &'a Repository) -> Self {
        Self { repo }
    }

    /// Analyze a branch to get commits available for splitting.
    pub fn analyze<S: StateStore>(&self, state: &S, branch_name: &str) -> Result<SplitAnalysis> {
        let stack = state.load_stack()?;
        let stack_branch = stack
            .find_branch(branch_name)
            .ok_or_else(|| anyhow::anyhow!("Branch '{branch_name}' not found in stack"))?;

        let parent = stack_branch
            .parent
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Cannot split a root branch (no parent)"))?;

        // Get commits between parent and branch
        let parent_oid = self.repo.branch_commit(parent)?;
        let branch_oid = self.repo.branch_commit(branch_name)?;
        let commit_oids = self.repo.commits_between(parent_oid, branch_oid)?;

        // Convert to CommitInfo (reverse to get oldest first)
        let commits: Vec<CommitInfo> = commit_oids
            .into_iter()
            .rev()
            .map(|oid| self.commit_info(oid))
            .collect::<Result<Vec<_>>>()?;

        Ok(SplitAnalysis {
            source_branch: branch_name.to_string(),
            parent_branch: parent.to_string(),
            commits,
        })
    }

    /// Get information about a commit.
    fn commit_info(&self, oid: Oid) -> Result<CommitInfo> {
        let commit = self.repo.find_commit(oid)?;
        let sha = oid.to_string();
        let short_sha = sha[..8.min(sha.len())].to_string();
        let summary = commit.summary().unwrap_or("(no message)").to_string();

        Ok(CommitInfo {
            oid: sha,
            short_sha,
            summary,
        })
    }

    /// Execute a split operation.
    ///
    /// # Errors
    /// Returns error if split fails or conflicts occur.
    #[allow(dead_code, clippy::unused_self)] // Used in Phase 4
    pub fn execute<S: StateStore>(&self, _state: &S, _config: &SplitConfig) -> Result<SplitResult> {
        // TODO: Phase 4 - Split execution engine
        bail!("Split execution not yet implemented");
    }

    /// Continue a paused split operation.
    ///
    /// # Errors
    /// Returns error if no split is in progress or continuation fails.
    #[allow(clippy::unused_self)] // Will use self.repo in Phase 4
    pub fn continue_split<S: StateStore>(&self, state: &S) -> Result<SplitResult> {
        if !state.is_split_in_progress() {
            bail!("No split in progress");
        }

        let _split_state = state.load_split_state()?;

        // TODO: Phase 4 - Continue split execution
        bail!("Split continue not yet implemented");
    }

    /// Abort a split operation and restore from backup.
    ///
    /// # Errors
    /// Returns error if no split is in progress or abort fails.
    pub fn abort<S: StateStore>(&self, state: &S) -> Result<()> {
        if !state.is_split_in_progress() {
            bail!("No split in progress");
        }

        let split_state = state.load_split_state()?;

        // Restore from backup
        self.restore_from_backup(state, &split_state)?;

        // Clear split state
        state.clear_split_state()?;

        Ok(())
    }

    /// Restore branches from backup.
    ///
    /// This function is designed to be robust against partial failures:
    /// 1. Validates all backup refs exist before mutating any state
    /// 2. Tracks successfully restored branches for recovery reporting
    /// 3. Defers backup deletion until all operations succeed
    fn restore_from_backup<S: StateStore>(
        &self,
        state: &S,
        split_state: &SplitState,
    ) -> Result<()> {
        let backup_refs = state.load_backup(&split_state.backup_id)?;

        // Phase 1: Validate all backup refs before mutating any state
        // This ensures we fail fast if any commit SHA is invalid or missing
        let validated_refs: Vec<(String, Oid)> = backup_refs
            .iter()
            .map(|(branch_name, commit_sha)| {
                let oid = Oid::from_str(commit_sha).with_context(|| {
                    format!(
                        "Invalid commit SHA '{}' for branch '{}' in backup '{}'",
                        commit_sha, branch_name, split_state.backup_id
                    )
                })?;

                // Verify the commit actually exists in the repository
                self.repo.find_commit(oid).with_context(|| {
                    format!(
                        "Commit {} for branch '{}' not found in repository. \
                         Manual recovery may be needed using backup '{}'",
                        commit_sha, branch_name, split_state.backup_id
                    )
                })?;

                Ok((branch_name.clone(), oid))
            })
            .collect::<Result<Vec<_>>>()?;

        // Phase 2: Reset branches, tracking successes for recovery reporting
        let mut restored_branches: Vec<String> = Vec::new();

        for (branch_name, oid) in &validated_refs {
            if let Err(e) = self.repo.reset_branch(branch_name, *oid) {
                // Log which branches were successfully restored before failure
                let restored_list = if restored_branches.is_empty() {
                    "none".to_string()
                } else {
                    restored_branches.join(", ")
                };

                bail!(
                    "Failed to reset branch '{}' to {}: {}. \
                     Successfully restored: [{}]. \
                     Remaining branches may need manual recovery from backup '{}'",
                    branch_name,
                    oid,
                    e,
                    restored_list,
                    split_state.backup_id
                );
            }
            restored_branches.push(branch_name.clone());
        }

        // Phase 3: Checkout original branch (only after all resets succeed)
        if let Err(e) = self.repo.checkout(&split_state.original_branch) {
            bail!(
                "All branches restored successfully [{}], but failed to checkout '{}': {}. \
                 Backup '{}' preserved for safety - delete manually after resolving",
                restored_branches.join(", "),
                split_state.original_branch,
                e,
                split_state.backup_id
            );
        }

        // Phase 4: Delete backup only after everything succeeds
        // If this fails, we've successfully restored but have orphaned backup data
        // Silently ignore - backup can be manually cleaned up via .git/rung/backups/
        let _ = state.delete_backup(&split_state.backup_id);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_commit_info_creation() {
        let info = CommitInfo {
            oid: "abc123def456".to_string(),
            short_sha: "abc123de".to_string(),
            summary: "Test commit".to_string(),
        };
        assert_eq!(info.short_sha, "abc123de");
        assert_eq!(info.summary, "Test commit");
    }

    #[test]
    fn test_split_config_creation() {
        let config = SplitConfig {
            source_branch: "feature".to_string(),
            parent_branch: "main".to_string(),
            split_points: vec![],
        };
        assert_eq!(config.source_branch, "feature");
        assert!(config.split_points.is_empty());
    }

    #[test]
    fn test_split_analysis_creation() {
        let analysis = SplitAnalysis {
            source_branch: "feature".to_string(),
            parent_branch: "main".to_string(),
            commits: vec![],
        };
        assert_eq!(analysis.parent_branch, "main");
        assert!(analysis.commits.is_empty());
    }

    #[test]
    fn test_split_result_creation() {
        let result = SplitResult {
            source_branch: "feature".to_string(),
            branches_created: vec!["feature-1".to_string(), "feature-2".to_string()],
        };
        assert_eq!(result.branches_created.len(), 2);
    }
}
