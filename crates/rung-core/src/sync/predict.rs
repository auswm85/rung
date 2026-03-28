//! Conflict prediction for sync operations.
//!
//! This module provides functionality to predict which branches will have
//! conflicts during a sync operation, allowing users to prepare before
//! starting the actual sync.

use super::types::{
    BranchConflictPrediction, CommitConflictPrediction, SyncConflictPrediction, SyncPlan,
};
use crate::error::Result;
use rung_git::GitOps;

/// Predict conflicts for a sync plan.
///
/// Analyzes each branch in the sync plan and predicts which commits would
/// conflict when rebased onto their new base. This allows users to see
/// potential conflicts before starting a sync operation.
///
/// # Arguments
/// * `repo` - Git repository operations
/// * `plan` - The sync plan to analyze
///
/// # Returns
/// `SyncConflictPrediction` containing branches with predicted conflicts.
/// Branches without conflicts are not included in the result.
///
/// # Errors
/// Returns error if git operations fail during prediction.
pub fn predict_sync_conflicts(
    repo: &impl GitOps,
    plan: &SyncPlan,
) -> Result<SyncConflictPrediction> {
    let mut predictions = SyncConflictPrediction::default();

    for action in &plan.branches {
        // Parse the target commit OID
        let onto_oid = rung_git::Oid::from_str(&action.new_base).map_err(|e| {
            crate::error::Error::SyncFailed(format!(
                "invalid commit '{}' for branch '{}': {e}",
                action.new_base, action.branch
            ))
        })?;

        // Get the name of the target (for display purposes)
        // The target is the parent branch's new position
        let onto_name = &action.new_base[..7.min(action.new_base.len())];

        // Predict conflicts for this branch
        let git_predictions = repo.predict_rebase_conflicts(&action.branch, onto_oid)?;

        // Only include branches that have conflicts
        if !git_predictions.is_empty() {
            let conflicts: Vec<CommitConflictPrediction> = git_predictions
                .into_iter()
                .map(|p| CommitConflictPrediction {
                    commit_hash: p.commit.to_string()[..7.min(p.commit.to_string().len())]
                        .to_string(),
                    commit_summary: p.commit_summary,
                    files: p.conflicting_files,
                })
                .collect();

            predictions.branches.push(BranchConflictPrediction {
                branch: action.branch.clone(),
                onto: onto_name.to_string(),
                conflicts,
            });
        }
    }

    Ok(predictions)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::sync::types::SyncAction;

    /// Mock implementation for testing
    struct MockRepo {
        predictions: std::collections::HashMap<String, Vec<rung_git::ConflictPrediction>>,
    }

    impl MockRepo {
        fn new() -> Self {
            Self {
                predictions: std::collections::HashMap::new(),
            }
        }

        fn with_conflicts(
            mut self,
            branch: &str,
            conflicts: Vec<rung_git::ConflictPrediction>,
        ) -> Self {
            self.predictions.insert(branch.to_string(), conflicts);
            self
        }
    }

    impl GitOps for MockRepo {
        fn workdir(&self) -> Option<&std::path::Path> {
            None
        }

        fn current_branch(&self) -> rung_git::Result<String> {
            Ok("main".to_string())
        }

        fn head_detached(&self) -> rung_git::Result<bool> {
            Ok(false)
        }

        fn is_rebasing(&self) -> bool {
            false
        }

        fn branch_exists(&self, _name: &str) -> bool {
            true
        }

        fn create_branch(&self, _name: &str) -> rung_git::Result<rung_git::Oid> {
            Ok(rung_git::Oid::zero())
        }

        fn checkout(&self, _branch: &str) -> rung_git::Result<()> {
            Ok(())
        }

        fn delete_branch(&self, _name: &str) -> rung_git::Result<()> {
            Ok(())
        }

        fn list_branches(&self) -> rung_git::Result<Vec<String>> {
            Ok(vec![])
        }

        fn branch_commit(&self, _branch: &str) -> rung_git::Result<rung_git::Oid> {
            Ok(rung_git::Oid::zero())
        }

        fn remote_branch_commit(&self, _branch: &str) -> rung_git::Result<rung_git::Oid> {
            Ok(rung_git::Oid::zero())
        }

        fn branch_commit_message(&self, _branch: &str) -> rung_git::Result<String> {
            Ok(String::new())
        }

        fn merge_base(
            &self,
            _one: rung_git::Oid,
            _two: rung_git::Oid,
        ) -> rung_git::Result<rung_git::Oid> {
            Ok(rung_git::Oid::zero())
        }

        fn commits_between(
            &self,
            _from: rung_git::Oid,
            _to: rung_git::Oid,
        ) -> rung_git::Result<Vec<rung_git::Oid>> {
            Ok(vec![])
        }

        fn count_commits_between(
            &self,
            _from: rung_git::Oid,
            _to: rung_git::Oid,
        ) -> rung_git::Result<usize> {
            Ok(0)
        }

        fn is_clean(&self) -> rung_git::Result<bool> {
            Ok(true)
        }

        fn require_clean(&self) -> rung_git::Result<()> {
            Ok(())
        }

        fn stage_all(&self) -> rung_git::Result<()> {
            Ok(())
        }

        fn has_staged_changes(&self) -> rung_git::Result<bool> {
            Ok(false)
        }

        fn create_commit(&self, _message: &str) -> rung_git::Result<rung_git::Oid> {
            Ok(rung_git::Oid::zero())
        }

        fn amend_commit(&self, _new_message: Option<&str>) -> rung_git::Result<rung_git::Oid> {
            Ok(rung_git::Oid::zero())
        }

        fn rebase_onto(&self, _target: rung_git::Oid) -> rung_git::Result<()> {
            Ok(())
        }

        fn rebase_onto_from(
            &self,
            _onto: rung_git::Oid,
            _from: rung_git::Oid,
        ) -> rung_git::Result<()> {
            Ok(())
        }

        fn conflicting_files(&self) -> rung_git::Result<Vec<String>> {
            Ok(vec![])
        }

        fn predict_rebase_conflicts(
            &self,
            branch: &str,
            _onto: rung_git::Oid,
        ) -> rung_git::Result<Vec<rung_git::ConflictPrediction>> {
            Ok(self.predictions.get(branch).cloned().unwrap_or_default())
        }

        fn rebase_abort(&self) -> rung_git::Result<()> {
            Ok(())
        }

        fn rebase_continue(&self) -> rung_git::Result<()> {
            Ok(())
        }

        fn origin_url(&self) -> rung_git::Result<String> {
            Ok(String::new())
        }

        fn remote_divergence(&self, _branch: &str) -> rung_git::Result<rung_git::RemoteDivergence> {
            Ok(rung_git::RemoteDivergence::InSync)
        }

        fn detect_default_branch(&self) -> Option<String> {
            Some("main".to_string())
        }

        fn push(&self, _branch: &str, _force: bool) -> rung_git::Result<()> {
            Ok(())
        }

        fn fetch_all(&self) -> rung_git::Result<()> {
            Ok(())
        }

        fn fetch(&self, _branch: &str) -> rung_git::Result<()> {
            Ok(())
        }

        fn pull_ff(&self) -> rung_git::Result<()> {
            Ok(())
        }

        fn reset_branch(&self, _branch: &str, _commit: rung_git::Oid) -> rung_git::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_predict_no_conflicts() {
        let repo = MockRepo::new();
        let plan = SyncPlan {
            branches: vec![SyncAction {
                branch: "feature".to_string(),
                old_base: "abc1234".to_string(),
                new_base: "0000000000000000000000000000000000000000".to_string(),
            }],
        };

        let result = predict_sync_conflicts(&repo, &plan).unwrap();
        assert!(!result.has_conflicts());
        assert_eq!(result.conflict_count(), 0);
    }

    #[test]
    fn test_predict_with_conflicts() {
        let repo = MockRepo::new().with_conflicts(
            "feature",
            vec![rung_git::ConflictPrediction {
                commit: rung_git::Oid::zero(),
                commit_summary: "Add feature".to_string(),
                conflicting_files: vec!["src/lib.rs".to_string()],
            }],
        );

        let plan = SyncPlan {
            branches: vec![SyncAction {
                branch: "feature".to_string(),
                old_base: "abc1234".to_string(),
                new_base: "0000000000000000000000000000000000000000".to_string(),
            }],
        };

        let result = predict_sync_conflicts(&repo, &plan).unwrap();
        assert!(result.has_conflicts());
        assert_eq!(result.conflict_count(), 1);
        assert_eq!(result.branches[0].branch, "feature");
        assert_eq!(result.branches[0].conflicts.len(), 1);
        assert_eq!(result.branches[0].conflicts[0].files, vec!["src/lib.rs"]);
    }

    #[test]
    fn test_predict_multiple_branches() {
        let repo = MockRepo::new()
            .with_conflicts(
                "feature-a",
                vec![rung_git::ConflictPrediction {
                    commit: rung_git::Oid::zero(),
                    commit_summary: "Feature A".to_string(),
                    conflicting_files: vec!["file_a.rs".to_string()],
                }],
            )
            .with_conflicts(
                "feature-b",
                vec![rung_git::ConflictPrediction {
                    commit: rung_git::Oid::zero(),
                    commit_summary: "Feature B".to_string(),
                    conflicting_files: vec!["file_b.rs".to_string()],
                }],
            );

        let plan = SyncPlan {
            branches: vec![
                SyncAction {
                    branch: "feature-a".to_string(),
                    old_base: "abc1234".to_string(),
                    new_base: "0000000000000000000000000000000000000000".to_string(),
                },
                SyncAction {
                    branch: "feature-b".to_string(),
                    old_base: "def5678".to_string(),
                    new_base: "0000000000000000000000000000000000000000".to_string(),
                },
                SyncAction {
                    branch: "feature-c".to_string(), // No conflicts for this one
                    old_base: "ghi9012".to_string(),
                    new_base: "0000000000000000000000000000000000000000".to_string(),
                },
            ],
        };

        let result = predict_sync_conflicts(&repo, &plan).unwrap();
        assert!(result.has_conflicts());
        assert_eq!(result.conflict_count(), 2);

        let branch_names: Vec<&str> = result.branches.iter().map(|b| b.branch.as_str()).collect();
        assert!(branch_names.contains(&"feature-a"));
        assert!(branch_names.contains(&"feature-b"));
        assert!(!branch_names.contains(&"feature-c"));
    }

    #[test]
    fn test_conflicting_files_deduplication() {
        let prediction = BranchConflictPrediction {
            branch: "feature".to_string(),
            onto: "main".to_string(),
            conflicts: vec![
                CommitConflictPrediction {
                    commit_hash: "abc1234".to_string(),
                    commit_summary: "First".to_string(),
                    files: vec!["file.rs".to_string(), "other.rs".to_string()],
                },
                CommitConflictPrediction {
                    commit_hash: "def5678".to_string(),
                    commit_summary: "Second".to_string(),
                    files: vec!["file.rs".to_string(), "another.rs".to_string()],
                },
            ],
        };

        let files = prediction.conflicting_files();
        assert_eq!(files.len(), 3); // file.rs should be deduplicated
        assert!(files.contains(&"file.rs"));
        assert!(files.contains(&"other.rs"));
        assert!(files.contains(&"another.rs"));
    }
}
