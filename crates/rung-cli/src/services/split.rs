//! Split service for dividing a branch into multiple stacked branches.
//!
//! This service encapsulates the business logic for the split command.

use anyhow::{Context, Result, bail};
use chrono::Utc;
use rung_core::{SplitPoint, SplitState, StackBranch, StateStore};
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

    /// Suggest a branch name based on a commit summary.
    ///
    /// Derives a kebab-case name from the first few words of the summary.
    #[must_use]
    pub fn suggest_branch_name(summary: &str, fallback_prefix: &str, index: usize) -> String {
        let cleaned: String = summary
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' {
                    c
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split_whitespace()
            .take(4)
            .collect::<Vec<_>>()
            .join("-")
            .to_lowercase();

        if cleaned.is_empty() {
            format!("{fallback_prefix}-part-{}", index + 1)
        } else {
            cleaned
        }
    }

    /// Execute a split operation.
    ///
    /// Creates new branches at each split point and updates the stack topology.
    ///
    /// # Errors
    /// Returns error if split fails.
    pub fn execute<S: StateStore>(&self, state: &S, config: &SplitConfig) -> Result<SplitResult> {
        // Get current branch for restoration after operation
        let original_branch = self.repo.current_branch()?;

        // Create backup
        let source_oid = self.repo.branch_commit(&config.source_branch)?;
        let backup_id = state.create_backup(&[(&config.source_branch, &source_oid.to_string())])?;

        // Initialize split state for recovery
        let split_state = SplitState {
            started_at: Utc::now(),
            backup_id: backup_id.clone(),
            source_branch: config.source_branch.clone(),
            parent_branch: config.parent_branch.clone(),
            original_branch: original_branch.clone(),
            split_points: config.split_points.clone(),
            current_index: 0,
            completed: vec![],
            stack_updated: false,
        };
        state.save_split_state(&split_state)?;

        // Execute the split
        match self.execute_split_loop(state, config) {
            Ok(result) => {
                // Clean up state on success
                state.clear_split_state()?;
                state.delete_backup(&backup_id)?;

                // Return to original branch if possible
                if self.repo.branch_exists(&original_branch) {
                    self.repo.checkout(&original_branch)?;
                }

                Ok(result)
            }
            Err(e) => {
                // On error, state remains for --continue or --abort
                Err(e)
            }
        }
    }

    /// Execute the split loop, creating branches at each split point.
    fn execute_split_loop<S: StateStore>(
        &self,
        state: &S,
        config: &SplitConfig,
    ) -> Result<SplitResult> {
        let mut stack = state.load_stack()?;
        let mut created_branches = Vec::new();
        let mut previous_parent = config.parent_branch.clone();

        for (idx, split_point) in config.split_points.iter().enumerate() {
            // Create the new branch at the split point commit
            let commit_oid = Oid::from_str(&split_point.commit_sha)?;

            // Create branch (initially at HEAD, then reset to target)
            self.repo.create_branch(&split_point.branch_name)?;
            self.repo
                .reset_branch(&split_point.branch_name, commit_oid)?;

            // Add to stack with proper parent
            let stack_branch =
                StackBranch::try_new(&split_point.branch_name, Some(&previous_parent))?;
            stack.add_branch(stack_branch);
            created_branches.push(split_point.branch_name.clone());

            // Update split state progress
            let mut split_state = state.load_split_state()?;
            split_state.current_index = idx + 1;
            split_state.completed.push(split_point.branch_name.clone());
            state.save_split_state(&split_state)?;

            // Next branch's parent is this branch
            previous_parent.clone_from(&split_point.branch_name);
        }

        // Update source branch's parent to be the last created branch
        if !created_branches.is_empty() {
            stack.reparent(&config.source_branch, Some(&previous_parent))?;
        }

        // Save updated stack
        state.save_stack(&stack)?;

        // Mark stack as updated
        let mut split_state = state.load_split_state()?;
        split_state.stack_updated = true;
        state.save_split_state(&split_state)?;

        Ok(SplitResult {
            source_branch: config.source_branch.clone(),
            branches_created: created_branches,
        })
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
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;
    use chrono::Utc;
    use rung_core::stack::{Stack, StackBranch};
    use rung_core::state::{SplitState, State};
    use std::fs;
    use std::process::Command as StdCommand;
    use tempfile::TempDir;

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

    #[test]
    fn test_suggest_branch_name_from_summary() {
        // Normal commit message
        let name = SplitService::suggest_branch_name("feat: add user auth", "feature", 0);
        assert_eq!(name, "feat-add-user-auth");

        // Long message - only takes first 4 words
        let name = SplitService::suggest_branch_name(
            "fix: resolve the complex issue with multiple components",
            "feature",
            0,
        );
        assert_eq!(name, "fix-resolve-the-complex");

        // Special characters stripped
        let name = SplitService::suggest_branch_name("feat(api): add endpoint!", "feature", 0);
        assert_eq!(name, "feat-api-add-endpoint");
    }

    #[test]
    fn test_suggest_branch_name_fallback() {
        // Empty summary falls back to prefix with index
        let name = SplitService::suggest_branch_name("", "feature", 0);
        assert_eq!(name, "feature-part-1");

        let name = SplitService::suggest_branch_name("", "my-branch", 2);
        assert_eq!(name, "my-branch-part-3");

        // Only special characters also falls back
        let name = SplitService::suggest_branch_name("!!!", "feature", 1);
        assert_eq!(name, "feature-part-2");
    }

    // === real-repo integration helpers ===

    /// Run a git command in the repo, asserting success.
    fn git(temp: &TempDir, args: &[&str]) {
        let out = StdCommand::new("git")
            .args(args)
            .current_dir(temp)
            .output()
            .expect("failed to run git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Create a temp git repo (branch `main`) with an initial commit, plus an
    /// initialized rung `State`.
    fn setup_repo() -> (TempDir, Repository, State) {
        let temp = TempDir::new().expect("temp dir");
        git(&temp, &["init"]);
        git(&temp, &["config", "user.email", "test@example.com"]);
        git(&temp, &["config", "user.name", "Test User"]);
        fs::write(temp.path().join("README.md"), "# Test\n").expect("write README");
        git(&temp, &["add", "."]);
        git(&temp, &["commit", "-m", "Initial commit"]);
        git(&temp, &["branch", "-M", "main"]);

        let repo = Repository::open(temp.path()).expect("open repo");
        let state = State::new(temp.path()).expect("state");
        state.init().expect("init state");
        (temp, repo, state)
    }

    fn add_commit(temp: &TempDir, file: &str, msg: &str) {
        fs::write(temp.path().join(file), format!("{file} content")).expect("write file");
        git(temp, &["add", "."]);
        git(temp, &["commit", "-m", msg]);
    }

    /// Build main -> feature with three commits on feature, and persist the stack.
    fn feature_with_three_commits(temp: &TempDir, state: &State) {
        git(temp, &["checkout", "-b", "feature"]);
        add_commit(temp, "one.txt", "commit one");
        add_commit(temp, "two.txt", "commit two");
        add_commit(temp, "three.txt", "commit three");

        let mut stack = Stack::new();
        stack.add_branch(StackBranch::try_new("feature", Some("main")).unwrap());
        state.save_stack(&stack).unwrap();
    }

    // === analyze ===

    #[test]
    fn test_analyze_returns_commits_oldest_first() {
        let (temp, repo, state) = setup_repo();
        feature_with_three_commits(&temp, &state);

        let service = SplitService::new(&repo);
        let analysis = service.analyze(&state, "feature").unwrap();

        assert_eq!(analysis.source_branch, "feature");
        assert_eq!(analysis.parent_branch, "main");
        assert_eq!(analysis.commits.len(), 3);
        assert_eq!(analysis.commits[0].summary, "commit one");
        assert_eq!(analysis.commits[2].summary, "commit three");
        // Short SHA is a prefix of the full oid.
        assert!(
            analysis.commits[0]
                .oid
                .starts_with(&analysis.commits[0].short_sha)
        );
    }

    #[test]
    fn test_analyze_root_branch_errors() {
        let (temp, repo, state) = setup_repo();
        // A branch with no parent cannot be split.
        let mut stack = Stack::new();
        stack.add_branch(StackBranch::try_new("feature", None::<String>).unwrap());
        state.save_stack(&stack).unwrap();
        git(&temp, &["checkout", "-b", "feature"]);

        let service = SplitService::new(&repo);
        assert!(service.analyze(&state, "feature").is_err());
    }

    #[test]
    fn test_analyze_branch_not_found() {
        let (_temp, repo, state) = setup_repo();
        let service = SplitService::new(&repo);
        assert!(service.analyze(&state, "does-not-exist").is_err());
    }

    // === execute ===

    #[test]
    fn test_execute_split_creates_branches() {
        let (temp, repo, state) = setup_repo();
        feature_with_three_commits(&temp, &state);

        let service = SplitService::new(&repo);
        let commits = service.analyze(&state, "feature").unwrap().commits;
        let one = &commits[0]; // "commit one"
        let two = &commits[1]; // "commit two"

        let config = SplitConfig {
            source_branch: "feature".to_string(),
            parent_branch: "main".to_string(),
            split_points: vec![
                SplitPoint {
                    commit_sha: one.oid.clone(),
                    message: one.summary.clone(),
                    branch_name: "part-one".to_string(),
                },
                SplitPoint {
                    commit_sha: two.oid.clone(),
                    message: two.summary.clone(),
                    branch_name: "part-two".to_string(),
                },
            ],
        };
        let result = service.execute(&state, &config).unwrap();

        assert_eq!(
            result.branches_created,
            vec!["part-one".to_string(), "part-two".to_string()]
        );
        assert!(repo.branch_exists("part-one"));
        assert!(repo.branch_exists("part-two"));
        assert_eq!(repo.branch_commit("part-one").unwrap().to_string(), one.oid);

        // Topology: main -> part-one -> part-two -> feature.
        let saved = state.load_stack().unwrap();
        assert_eq!(
            saved.find_branch("part-one").unwrap().parent.as_deref(),
            Some("main")
        );
        assert_eq!(
            saved.find_branch("part-two").unwrap().parent.as_deref(),
            Some("part-one")
        );
        assert_eq!(
            saved.find_branch("feature").unwrap().parent.as_deref(),
            Some("part-two")
        );
        assert!(!state.is_split_in_progress());
    }

    #[test]
    fn test_execute_split_no_points_is_noop() {
        let (temp, repo, state) = setup_repo();
        feature_with_three_commits(&temp, &state);

        let service = SplitService::new(&repo);
        let config = SplitConfig {
            source_branch: "feature".to_string(),
            parent_branch: "main".to_string(),
            split_points: vec![],
        };
        let result = service.execute(&state, &config).unwrap();

        assert!(result.branches_created.is_empty());
        // Source branch parent is unchanged.
        let saved = state.load_stack().unwrap();
        assert_eq!(
            saved.find_branch("feature").unwrap().parent.as_deref(),
            Some("main")
        );
        assert!(!state.is_split_in_progress());
    }

    // === abort ===

    #[test]
    fn test_abort_no_split_in_progress() {
        let (_temp, repo, state) = setup_repo();
        let service = SplitService::new(&repo);
        assert!(service.abort(&state).is_err());
    }

    #[test]
    fn test_abort_restores_source_branch() {
        let (temp, repo, state) = setup_repo();
        git(&temp, &["checkout", "-b", "feature"]);
        add_commit(&temp, "one.txt", "commit one");

        let original_sha = repo.branch_commit("feature").unwrap().to_string();

        // Back up feature at its current tip, then advance it (simulating an
        // interrupted split that mutated the branch).
        let backup_id = state.create_backup(&[("feature", &original_sha)]).unwrap();
        add_commit(&temp, "two.txt", "commit two");
        assert_ne!(
            repo.branch_commit("feature").unwrap().to_string(),
            original_sha
        );

        let split_state = SplitState {
            started_at: Utc::now(),
            backup_id,
            source_branch: "feature".to_string(),
            parent_branch: "main".to_string(),
            original_branch: "feature".to_string(),
            split_points: vec![],
            current_index: 0,
            completed: vec![],
            stack_updated: false,
        };
        state.save_split_state(&split_state).unwrap();
        assert!(state.is_split_in_progress());

        let service = SplitService::new(&repo);
        service.abort(&state).unwrap();

        // feature is reset back to its backed-up tip and state is cleared.
        assert_eq!(
            repo.branch_commit("feature").unwrap().to_string(),
            original_sha
        );
        assert!(!state.is_split_in_progress());
    }
}
