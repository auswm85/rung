//! Fold service for combining adjacent branches into one.
//!
//! This service encapsulates the business logic for the fold command,
//! which is the inverse of the split command.

use anyhow::{Context, Result, bail};
use rung_core::{FoldState, StackBranch, StateStore};
use rung_git::{Oid, Repository};
use serde::Serialize;

/// Information about a branch that can be folded.
#[derive(Debug, Clone, Serialize)]
pub struct FoldBranchInfo {
    /// Branch name.
    pub name: String,
    /// Number of commits in this branch.
    pub commit_count: usize,
    /// Associated PR number (if any).
    pub pr: Option<u64>,
}

/// Configuration for a fold operation.
#[derive(Debug, Clone)]
pub struct FoldConfig {
    /// The target branch that will contain all commits after folding.
    /// This is the bottommost branch in the chain being folded.
    pub target_branch: String,
    /// Branches being folded into the target (in parent-to-child order).
    /// Does NOT include the target branch itself.
    pub branches_to_fold: Vec<String>,
    /// The new parent for the target branch (parent of the topmost folded branch).
    pub new_parent: String,
}

/// Analysis of branches that can be folded.
#[derive(Debug, Clone)]
pub struct FoldAnalysis {
    /// The parent branch of the current branch.
    pub parent_branch: Option<String>,
    /// Children of the current branch that could be folded.
    pub children: Vec<FoldBranchInfo>,
}

/// Result of a fold operation.
#[derive(Debug, Clone, Serialize)]
pub struct FoldResult {
    /// The combined branch name.
    pub target_branch: String,
    /// Total number of commits in the combined branch.
    pub total_commits: usize,
    /// Branches that were folded (removed).
    pub branches_folded: Vec<String>,
    /// PRs that should be closed.
    pub prs_to_close: Vec<u64>,
}

/// Service for fold operations.
pub struct FoldService<'a> {
    repo: &'a Repository,
}

impl<'a> FoldService<'a> {
    /// Create a new fold service.
    #[must_use]
    pub const fn new(repo: &'a Repository) -> Self {
        Self { repo }
    }

    /// Analyze the current branch to determine what can be folded.
    pub fn analyze<S: StateStore>(&self, state: &S, branch_name: &str) -> Result<FoldAnalysis> {
        let stack = state.load_stack()?;
        let stack_branch = stack
            .find_branch(branch_name)
            .ok_or_else(|| anyhow::anyhow!("Branch '{branch_name}' not found in stack"))?;

        let parent_branch = stack_branch
            .parent
            .as_ref()
            .map(std::string::ToString::to_string);

        // Get children that could be folded
        let children = self.get_foldable_children(&stack, branch_name)?;

        Ok(FoldAnalysis {
            parent_branch,
            children,
        })
    }

    /// Get children of a branch that could be folded.
    fn get_foldable_children(
        &self,
        stack: &rung_core::Stack,
        branch_name: &str,
    ) -> Result<Vec<FoldBranchInfo>> {
        // For simplicity, only allow folding linear chains (single child at each level)
        let mut result = Vec::new();
        let mut current = branch_name;

        loop {
            let children = stack.children_of(current);
            if children.len() != 1 {
                // Stop if no children or multiple children (branching)
                break;
            }

            let child = children[0];
            let commit_count = self.count_branch_commits(child)?;
            result.push(FoldBranchInfo {
                name: child.name.to_string(),
                commit_count,
                pr: child.pr,
            });
            current = &child.name;
        }

        Ok(result)
    }

    /// Count the number of commits in a branch (between parent and branch tip).
    fn count_branch_commits(&self, branch: &StackBranch) -> Result<usize> {
        let Some(parent) = &branch.parent else {
            return Ok(0);
        };

        let parent_oid = self.repo.branch_commit(parent)?;
        let branch_oid = self.repo.branch_commit(&branch.name)?;
        let commits = self.repo.commits_between(parent_oid, branch_oid)?;
        Ok(commits.len())
    }

    /// Execute a fold operation.
    ///
    /// This combines multiple adjacent branches into one by:
    /// 1. Creating a backup of all involved branches
    /// 2. Resetting the target branch to include all commits
    /// 3. Removing the folded branches from the stack
    /// 4. Updating children to point to the target branch
    pub fn execute<S: StateStore>(&self, state: &S, config: &FoldConfig) -> Result<FoldResult> {
        let original_branch = self.repo.current_branch()?;
        let mut stack = state.load_stack()?;

        // Collect all branches involved and their commits for backup
        let mut backup_branches = vec![(
            config.target_branch.as_str(),
            self.repo.branch_commit(&config.target_branch)?.to_string(),
        )];

        for branch_name in &config.branches_to_fold {
            backup_branches.push((
                branch_name.as_str(),
                self.repo.branch_commit(branch_name)?.to_string(),
            ));
        }

        let backup_refs: Vec<(&str, &str)> = backup_branches
            .iter()
            .map(|(name, sha)| (*name, sha.as_str()))
            .collect();
        let backup_id = state.create_backup(&backup_refs)?;

        // Collect PRs to close
        let prs_to_close: Vec<u64> = config
            .branches_to_fold
            .iter()
            .filter_map(|name| stack.find_branch(name).and_then(|b| b.pr))
            .collect();

        // Initialize fold state for recovery (include original stack for abort)
        let original_stack_json =
            serde_json::to_string(&stack).context("Failed to serialize original stack")?;
        let mut fold_state = FoldState::new(
            backup_id.clone(),
            config.target_branch.clone(),
            config.branches_to_fold.clone(),
            config.new_parent.clone(),
            original_branch.clone(),
            prs_to_close.clone(),
        );
        fold_state.set_original_stack(original_stack_json);
        state.save_fold_state(&fold_state)?;

        // Execute the fold
        match self.execute_fold_inner(state, config, &mut stack, prs_to_close) {
            Ok(result) => {
                // Clean up state on success
                state.clear_fold_state()?;
                state.delete_backup(&backup_id)?;

                // Return to original branch if possible, otherwise target
                let checkout_branch = if config.branches_to_fold.contains(&original_branch) {
                    &config.target_branch
                } else if self.repo.branch_exists(&original_branch) {
                    &original_branch
                } else {
                    &config.target_branch
                };
                self.repo.checkout(checkout_branch)?;

                Ok(result)
            }
            Err(e) => {
                // On error, state remains for --abort
                Err(e)
            }
        }
    }

    /// Inner fold execution logic.
    fn execute_fold_inner<S: StateStore>(
        &self,
        state: &S,
        config: &FoldConfig,
        stack: &mut rung_core::Stack,
        prs_to_close: Vec<u64>,
    ) -> Result<FoldResult> {
        // Validate we have branches to fold
        if config.branches_to_fold.is_empty() {
            bail!("No branches specified to fold");
        }

        // The target branch will be reset to include all commits from folded branches.
        // Since branches are adjacent (parent-child chain), the tip of the last
        // branch in the chain contains all commits.
        let last_branch = &config.branches_to_fold[config.branches_to_fold.len() - 1];
        let final_commit = self.repo.branch_commit(last_branch)?;

        // Find any children of the last folded branch - they need to be reparented
        let children_to_reparent: Vec<String> = stack
            .children_of(last_branch)
            .iter()
            .map(|b| b.name.to_string())
            .collect();

        // Reset target branch to the final commit
        self.repo
            .reset_branch(&config.target_branch, final_commit)?;

        // Update target branch's parent to the new parent
        stack.reparent(&config.target_branch, Some(&config.new_parent))?;

        // Reparent any children of the last folded branch to the target
        for child in &children_to_reparent {
            stack.reparent(child, Some(&config.target_branch))?;
        }

        // Remove folded branches from stack
        let branches_folded: Vec<String> = config.branches_to_fold.clone();
        for branch_name in &branches_folded {
            stack.remove_branch(branch_name);
        }

        // Persist stack state first to ensure consistency
        // If git deletion fails later, stack is already correct and branches can be manually cleaned
        state.save_stack(stack)?;

        // Mark stack as updated so abort knows to restore it
        let mut fold_state = state
            .load_fold_state()
            .context("Failed to load fold state after stack modification")?;
        fold_state.mark_stack_updated();
        state
            .save_fold_state(&fold_state)
            .context("Failed to update fold state after stack modification")?;

        // Now delete git branches (best-effort - log errors but continue)
        // Track completed deletions in fold state for abort recovery
        for branch_name in &branches_folded {
            if self.repo.branch_exists(branch_name) {
                if let Err(e) = self.repo.delete_branch(branch_name) {
                    eprintln!("Warning: Failed to delete branch '{branch_name}': {e}");
                } else {
                    // Track successful deletion
                    fold_state.completed.push(branch_name.clone());
                    // Best-effort save - don't fail the overall operation
                    let _ = state.save_fold_state(&fold_state);
                }
            } else {
                // Branch already gone, consider it completed
                fold_state.completed.push(branch_name.clone());
            }
        }

        // Count total commits
        let parent_oid = self.repo.branch_commit(&config.new_parent)?;
        let target_oid = self.repo.branch_commit(&config.target_branch)?;
        let total_commits = self.repo.commits_between(parent_oid, target_oid)?.len();

        Ok(FoldResult {
            target_branch: config.target_branch.clone(),
            total_commits,
            branches_folded,
            prs_to_close,
        })
    }

    /// Abort a fold operation and restore from backup.
    pub fn abort<S: StateStore>(&self, state: &S) -> Result<()> {
        if !state.is_fold_in_progress() {
            bail!("No fold in progress");
        }

        let fold_state = state.load_fold_state()?;

        // Restore stack if it was updated
        if fold_state.stack_updated
            && let Some(ref original_json) = fold_state.original_stack_json
        {
            let original_stack: rung_core::Stack = serde_json::from_str(original_json)
                .context("Failed to deserialize original stack")?;
            state.save_stack(&original_stack)?;
        }

        self.restore_from_backup(state, &fold_state)?;
        state.clear_fold_state()?;

        Ok(())
    }

    /// Restore branches from backup.
    fn restore_from_backup<S: StateStore>(&self, state: &S, fold_state: &FoldState) -> Result<()> {
        let backup_refs = state.load_backup(&fold_state.backup_id)?;

        // Validate all backup refs first
        let validated_refs: Vec<(String, Oid)> = backup_refs
            .iter()
            .map(|(branch_name, commit_sha)| {
                let oid = Oid::from_str(commit_sha).with_context(|| {
                    format!("Invalid commit SHA '{commit_sha}' for branch '{branch_name}'")
                })?;
                self.repo.find_commit(oid).with_context(|| {
                    format!("Commit {commit_sha} for branch '{branch_name}' not found")
                })?;
                Ok((branch_name.clone(), oid))
            })
            .collect::<Result<Vec<_>>>()?;

        // Recreate deleted branches and reset existing ones
        for (branch_name, oid) in &validated_refs {
            if !self.repo.branch_exists(branch_name) {
                self.repo.create_branch(branch_name)?;
            }
            self.repo.reset_branch(branch_name, *oid)?;
        }

        // Checkout original branch
        self.repo.checkout(&fold_state.original_branch)?;

        // Delete backup
        let _ = state.delete_backup(&fold_state.backup_id);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;
    use rung_core::stack::{Stack, StackBranch};
    use rung_core::state::State;
    use std::fs;
    use std::process::Command as StdCommand;
    use tempfile::TempDir;

    // === struct tests ===

    #[test]
    fn test_fold_branch_info() {
        let info = FoldBranchInfo {
            name: "feature/test".to_string(),
            commit_count: 3,
            pr: Some(42),
        };
        assert_eq!(info.name, "feature/test");
        assert_eq!(info.commit_count, 3);
        assert_eq!(info.pr, Some(42));
    }

    #[test]
    fn test_fold_config() {
        let config = FoldConfig {
            target_branch: "feature/base".to_string(),
            branches_to_fold: vec!["feature/child".to_string()],
            new_parent: "main".to_string(),
        };
        assert_eq!(config.target_branch, "feature/base");
        assert_eq!(config.branches_to_fold.len(), 1);
    }

    #[test]
    fn test_fold_result() {
        let result = FoldResult {
            target_branch: "feature/combined".to_string(),
            total_commits: 5,
            branches_folded: vec!["feature/a".to_string(), "feature/b".to_string()],
            prs_to_close: vec![42, 43],
        };
        assert_eq!(result.total_commits, 5);
        assert_eq!(result.branches_folded.len(), 2);
        assert_eq!(result.prs_to_close.len(), 2);
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

    /// Create `branch` off the current HEAD and add one commit touching `file`.
    fn branch_with_commit(temp: &TempDir, branch: &str, file: &str, msg: &str) {
        git(temp, &["checkout", "-b", branch]);
        fs::write(temp.path().join(file), format!("{file} content")).expect("write file");
        git(temp, &["add", "."]);
        git(temp, &["commit", "-m", msg]);
    }

    fn checkout(temp: &TempDir, branch: &str) {
        git(temp, &["checkout", branch]);
    }

    /// Build a linear chain main -> feature-a -> feature-b -> feature-c and
    /// persist the matching stack. PRs: a=None, b=7, c=8.
    fn linear_chain(temp: &TempDir, state: &State) -> Stack {
        branch_with_commit(temp, "feature-a", "a.txt", "commit a");
        branch_with_commit(temp, "feature-b", "b.txt", "commit b");
        branch_with_commit(temp, "feature-c", "c.txt", "commit c");

        let mut stack = Stack::new();
        stack.add_branch(StackBranch::try_new("feature-a", Some("main")).unwrap());
        let mut b = StackBranch::try_new("feature-b", Some("feature-a")).unwrap();
        b.pr = Some(7);
        stack.add_branch(b);
        let mut c = StackBranch::try_new("feature-c", Some("feature-b")).unwrap();
        c.pr = Some(8);
        stack.add_branch(c);
        state.save_stack(&stack).unwrap();
        stack
    }

    // === analyze ===

    #[test]
    fn test_analyze_linear_chain() {
        let (temp, repo, state) = setup_repo();
        linear_chain(&temp, &state);
        checkout(&temp, "feature-a");

        let service = FoldService::new(&repo);
        let analysis = service.analyze(&state, "feature-a").unwrap();

        assert_eq!(analysis.parent_branch.as_deref(), Some("main"));
        assert_eq!(analysis.children.len(), 2);
        assert_eq!(analysis.children[0].name, "feature-b");
        assert_eq!(analysis.children[0].commit_count, 1);
        assert_eq!(analysis.children[0].pr, Some(7));
        assert_eq!(analysis.children[1].name, "feature-c");
        assert_eq!(analysis.children[1].pr, Some(8));
    }

    #[test]
    fn test_analyze_stops_at_branching() {
        let (temp, repo, state) = setup_repo();
        branch_with_commit(&temp, "feature-a", "a.txt", "commit a");
        checkout(&temp, "feature-a");
        branch_with_commit(&temp, "feature-b", "b.txt", "commit b");
        checkout(&temp, "feature-a");
        branch_with_commit(&temp, "feature-c", "c.txt", "commit c");

        let mut stack = Stack::new();
        stack.add_branch(StackBranch::try_new("feature-a", Some("main")).unwrap());
        stack.add_branch(StackBranch::try_new("feature-b", Some("feature-a")).unwrap());
        stack.add_branch(StackBranch::try_new("feature-c", Some("feature-a")).unwrap());
        state.save_stack(&stack).unwrap();

        let service = FoldService::new(&repo);
        let analysis = service.analyze(&state, "feature-a").unwrap();

        // Two children => branching, so nothing is foldable as a linear chain.
        assert!(analysis.children.is_empty());
    }

    #[test]
    fn test_analyze_branch_not_found() {
        let (_temp, repo, state) = setup_repo();
        let service = FoldService::new(&repo);
        assert!(service.analyze(&state, "does-not-exist").is_err());
    }

    // === execute ===

    #[test]
    fn test_execute_fold_child_into_parent() {
        let (temp, repo, state) = setup_repo();
        branch_with_commit(&temp, "feature-a", "a.txt", "commit a");
        branch_with_commit(&temp, "feature-b", "b.txt", "commit b");

        let mut stack = Stack::new();
        stack.add_branch(StackBranch::try_new("feature-a", Some("main")).unwrap());
        let mut b = StackBranch::try_new("feature-b", Some("feature-a")).unwrap();
        b.pr = Some(7);
        stack.add_branch(b);
        state.save_stack(&stack).unwrap();

        let b_tip = repo.branch_commit("feature-b").unwrap();
        checkout(&temp, "feature-a");

        let service = FoldService::new(&repo);
        let config = FoldConfig {
            target_branch: "feature-a".to_string(),
            branches_to_fold: vec!["feature-b".to_string()],
            new_parent: "main".to_string(),
        };
        let result = service.execute(&state, &config).unwrap();

        assert_eq!(result.target_branch, "feature-a");
        assert_eq!(result.branches_folded, vec!["feature-b".to_string()]);
        assert_eq!(result.prs_to_close, vec![7]);
        assert_eq!(result.total_commits, 2);

        // feature-a now points at feature-b's old tip; feature-b is gone.
        assert_eq!(repo.branch_commit("feature-a").unwrap(), b_tip);
        assert!(!repo.branch_exists("feature-b"));

        let saved = state.load_stack().unwrap();
        assert!(saved.find_branch("feature-b").is_none());
        assert!(saved.find_branch("feature-a").is_some());
        assert!(!state.is_fold_in_progress());
    }

    #[test]
    fn test_execute_fold_reparents_grandchildren() {
        let (temp, repo, state) = setup_repo();
        linear_chain(&temp, &state);
        checkout(&temp, "feature-a");

        let service = FoldService::new(&repo);
        // Fold feature-b into feature-a; feature-c (child of b) must reparent to a.
        let config = FoldConfig {
            target_branch: "feature-a".to_string(),
            branches_to_fold: vec!["feature-b".to_string()],
            new_parent: "main".to_string(),
        };
        service.execute(&state, &config).unwrap();

        let saved = state.load_stack().unwrap();
        assert!(saved.find_branch("feature-b").is_none());
        assert_eq!(
            saved.find_branch("feature-c").unwrap().parent.as_deref(),
            Some("feature-a")
        );
    }

    #[test]
    fn test_execute_fold_multiple_children() {
        let (temp, repo, state) = setup_repo();
        linear_chain(&temp, &state);
        let c_tip = repo.branch_commit("feature-c").unwrap();
        checkout(&temp, "feature-a");

        let service = FoldService::new(&repo);
        // Fold both feature-b and feature-c into feature-a.
        let config = FoldConfig {
            target_branch: "feature-a".to_string(),
            branches_to_fold: vec!["feature-b".to_string(), "feature-c".to_string()],
            new_parent: "main".to_string(),
        };
        let result = service.execute(&state, &config).unwrap();

        assert_eq!(
            result.branches_folded,
            vec!["feature-b".to_string(), "feature-c".to_string()]
        );
        assert_eq!(result.prs_to_close, vec![7, 8]);
        assert_eq!(repo.branch_commit("feature-a").unwrap(), c_tip);
        assert!(!repo.branch_exists("feature-b"));
        assert!(!repo.branch_exists("feature-c"));

        let saved = state.load_stack().unwrap();
        assert_eq!(saved.len(), 1);
        assert!(saved.find_branch("feature-a").is_some());
    }

    #[test]
    fn test_execute_empty_branches_errors() {
        let (temp, repo, state) = setup_repo();
        branch_with_commit(&temp, "feature-a", "a.txt", "commit a");
        let mut stack = Stack::new();
        stack.add_branch(StackBranch::try_new("feature-a", Some("main")).unwrap());
        state.save_stack(&stack).unwrap();
        checkout(&temp, "feature-a");

        let service = FoldService::new(&repo);
        let config = FoldConfig {
            target_branch: "feature-a".to_string(),
            branches_to_fold: vec![],
            new_parent: "main".to_string(),
        };
        assert!(service.execute(&state, &config).is_err());
    }

    // === abort ===

    #[test]
    fn test_abort_no_fold_in_progress() {
        let (_temp, repo, state) = setup_repo();
        let service = FoldService::new(&repo);
        assert!(service.abort(&state).is_err());
    }

    #[test]
    fn test_abort_restores_after_interrupted_fold() {
        // Uses slash-style branch names because the backup ref store maps
        // '/' <-> '-', so hyphenated names do not round-trip.
        let (temp, repo, state) = setup_repo();
        branch_with_commit(&temp, "feature/a", "a.txt", "commit a");
        branch_with_commit(&temp, "feature/b", "b.txt", "commit b");

        let mut stack = Stack::new();
        stack.add_branch(StackBranch::try_new("feature/a", Some("main")).unwrap());
        stack.add_branch(StackBranch::try_new("feature/b", Some("feature/a")).unwrap());
        state.save_stack(&stack).unwrap();

        let a_sha = repo.branch_commit("feature/a").unwrap().to_string();
        let b_sha = repo.branch_commit("feature/b").unwrap().to_string();
        let original_stack_json = serde_json::to_string(&stack).unwrap();

        // Simulate an interrupted fold: back up both branches, then delete
        // feature/b and drop it from the persisted stack.
        checkout(&temp, "feature/a");
        let backup_id = state
            .create_backup(&[("feature/a", &a_sha), ("feature/b", &b_sha)])
            .unwrap();

        let mut reduced = stack.clone();
        reduced.remove_branch("feature/b");
        state.save_stack(&reduced).unwrap();
        git(&temp, &["branch", "-D", "feature/b"]);

        let mut fold_state = FoldState::new(
            backup_id,
            "feature/a".to_string(),
            vec!["feature/b".to_string()],
            "main".to_string(),
            "feature/a".to_string(),
            vec![],
        );
        fold_state.set_original_stack(original_stack_json);
        fold_state.mark_stack_updated();
        state.save_fold_state(&fold_state).unwrap();
        assert!(state.is_fold_in_progress());

        let service = FoldService::new(&repo);
        service.abort(&state).unwrap();

        // feature/b is recreated at its backed-up commit and the stack restored.
        assert!(repo.branch_exists("feature/b"));
        assert_eq!(repo.branch_commit("feature/b").unwrap().to_string(), b_sha);
        let restored = state.load_stack().unwrap();
        assert!(restored.find_branch("feature/b").is_some());
        assert!(!state.is_fold_in_progress());
    }
}
