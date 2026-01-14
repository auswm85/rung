//! Sync engine for rebasing stack branches.
//!
//! This module contains the core logic for the `rung sync` command,
//! which recursively rebases all branches in a stack when the base moves.

use crate::error::Result;
use crate::stack::Stack;
use crate::state::State;

/// Result of a sync operation.
#[derive(Debug)]
pub enum SyncResult {
    /// Stack was already up-to-date.
    AlreadySynced,

    /// Sync completed successfully.
    Complete {
        /// Number of branches rebased.
        branches_rebased: usize,
        /// Backup ID that can be used for undo.
        backup_id: String,
    },

    /// Sync paused due to conflict.
    Paused {
        /// Branch where conflict occurred.
        at_branch: String,
        /// Files with conflicts.
        conflict_files: Vec<String>,
        /// Backup ID for potential undo.
        backup_id: String,
    },
}

/// Plan for syncing a stack.
#[derive(Debug)]
pub struct SyncPlan {
    /// Branches to rebase, in order.
    pub branches: Vec<SyncAction>,
}

/// A single rebase action in the sync plan.
#[derive(Debug)]
pub struct SyncAction {
    /// Branch to rebase.
    pub branch: String,
    /// Current base commit (will be replaced).
    pub old_base: String,
    /// New base commit (parent's new tip).
    pub new_base: String,
}

impl SyncPlan {
    /// Check if the plan is empty (nothing to sync).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.branches.is_empty()
    }
}

/// Branches that were found to be stale (in stack but not in git).
#[derive(Debug, Default)]
pub struct StaleBranches {
    /// Names of branches that were removed from the stack.
    pub removed: Vec<String>,
}

/// Create a sync plan for the given stack.
///
/// Analyzes which branches need rebasing based on their parent's current position.
/// Branches are processed in stack order (parents before children) to ensure
/// each branch is rebased onto the correct target.
///
/// Stale branches (in stack but not in git) are detected and can be cleaned up
/// by calling `remove_stale_branches`.
///
/// # Errors
/// Returns error if git operations fail.
pub fn create_sync_plan(
    repo: &rung_git::Repository,
    stack: &Stack,
    base_branch: &str,
) -> Result<SyncPlan> {
    let mut actions = Vec::new();

    // Process branches in stack order (parents before children)
    for branch in &stack.branches {
        // Skip branches that don't exist locally (stale branches)
        // These will be handled separately by remove_stale_branches
        if !repo.branch_exists(&branch.name) {
            continue;
        }

        // Determine the parent branch name
        let parent_name = branch.parent.as_deref().unwrap_or(base_branch);

        // Skip if parent doesn't exist (external branch like main might not exist locally)
        if !repo.branch_exists(parent_name) && branch.parent.is_none() {
            // Base branch doesn't exist - this is an error
            return Err(crate::error::Error::BranchNotFound(parent_name.to_string()));
        }

        // If parent is a stack branch that doesn't exist, skip this branch too
        // (it will be handled when we clean up stale branches)
        if branch.parent.is_some() && !repo.branch_exists(parent_name) {
            continue;
        }

        // Get commits
        let branch_commit = repo.branch_commit(&branch.name)?;
        let parent_commit = repo.branch_commit(parent_name)?;

        // Find where this branch diverged from parent
        let merge_base = repo.merge_base(branch_commit, parent_commit)?;

        // If merge base is not the parent's tip, we need to rebase
        if merge_base != parent_commit {
            actions.push(SyncAction {
                branch: branch.name.clone(),
                old_base: merge_base.to_string(),
                new_base: parent_commit.to_string(),
            });
        }
    }

    Ok(SyncPlan { branches: actions })
}

/// Find and remove stale branches from the stack.
///
/// A stale branch is one that exists in `stack.json` but not in the local git repository.
/// This can happen if a branch was deleted externally or if the stack got out of sync.
///
/// Returns information about the branches that were removed.
///
/// # Errors
/// Returns error if stack operations fail.
pub fn remove_stale_branches(repo: &rung_git::Repository, state: &State) -> Result<StaleBranches> {
    let mut stack = state.load_stack()?;
    let mut removed = Vec::new();

    // Find branches that don't exist locally
    let missing: Vec<String> = stack
        .branches
        .iter()
        .filter(|b| !repo.branch_exists(&b.name))
        .map(|b| b.name.clone())
        .collect();

    if missing.is_empty() {
        return Ok(StaleBranches::default());
    }

    // For each stale branch, re-parent its children to its parent
    for missing_name in &missing {
        let missing_parent = stack
            .find_branch(missing_name)
            .and_then(|b| b.parent.clone());

        // Re-parent children of this stale branch
        for branch in &mut stack.branches {
            if branch.parent.as_ref() == Some(missing_name) {
                branch.parent.clone_from(&missing_parent);
            }
        }

        removed.push(missing_name.clone());
    }

    // Remove stale branches from stack
    stack.branches.retain(|b| !missing.contains(&b.name));
    state.save_stack(&stack)?;

    Ok(StaleBranches { removed })
}

/// Execute a sync operation.
///
/// Rebases all branches in the plan onto their new bases. If a conflict occurs,
/// the sync is paused and can be continued with `continue_sync` after resolution.
///
/// # Errors
/// Returns error if sync fails.
pub fn execute_sync(
    repo: &rung_git::Repository,
    state: &State,
    plan: SyncPlan,
) -> Result<SyncResult> {
    use crate::state::SyncState;

    // If plan is empty, nothing to do
    if plan.is_empty() {
        return Ok(SyncResult::AlreadySynced);
    }

    // Create backup of all branches in the plan
    let branches_to_backup: Vec<(String, String)> = plan
        .branches
        .iter()
        .map(|action| {
            let commit = repo.branch_commit(&action.branch)?;
            Ok((action.branch.clone(), commit.to_string()))
        })
        .collect::<Result<Vec<_>>>()?;

    let backup_refs: Vec<(&str, &str)> = branches_to_backup
        .iter()
        .map(|(b, c)| (b.as_str(), c.as_str()))
        .collect();

    let backup_id = state.create_backup(&backup_refs)?;

    // Save original branch to restore later
    let original_branch = repo.current_branch().ok();

    // Create sync state
    let branch_names: Vec<String> = plan.branches.iter().map(|a| a.branch.clone()).collect();
    let mut sync_state = SyncState::new(backup_id.clone(), branch_names);
    state.save_sync_state(&sync_state)?;

    // Execute each rebase
    for action in plan.branches {
        // Checkout the branch
        repo.checkout(&action.branch)?;

        // Get target commit
        let new_base = rung_git::Oid::from_str(&action.new_base)
            .map_err(|e| crate::error::Error::RebaseFailed(action.branch.clone(), e.to_string()))?;

        // Rebase onto new base
        match repo.rebase_onto(new_base) {
            Ok(()) => {
                // Success - mark as complete and save state
                sync_state.advance();
                state.save_sync_state(&sync_state)?;
            }
            Err(rung_git::Error::RebaseConflict(files)) => {
                // Conflict - save state and return Paused
                state.save_sync_state(&sync_state)?;
                return Ok(SyncResult::Paused {
                    at_branch: action.branch,
                    conflict_files: files,
                    backup_id,
                });
            }
            Err(e) => {
                // Other error - abort and return error
                let _ = repo.rebase_abort(); // Best effort
                state.clear_sync_state()?;
                return Err(e.into());
            }
        }
    }

    // All done - clean up sync state
    state.clear_sync_state()?;

    // Restore original branch if possible
    if let Some(branch) = original_branch {
        let _ = repo.checkout(&branch); // Best effort
    }

    Ok(SyncResult::Complete {
        branches_rebased: sync_state.completed.len(),
        backup_id,
    })
}

/// Continue a paused sync after conflict resolution.
///
/// User must have resolved conflicts and staged the changes before calling this.
///
/// # Errors
/// Returns error if no sync in progress or continuation fails.
pub fn continue_sync(repo: &rung_git::Repository, state: &State) -> Result<SyncResult> {
    // Load sync state
    let mut sync_state = state.load_sync_state()?;
    let backup_id = sync_state.backup_id.clone();

    // Continue the current rebase
    match repo.rebase_continue() {
        Ok(()) => {
            // Success - mark current branch as complete
            sync_state.advance();
            state.save_sync_state(&sync_state)?;
        }
        Err(rung_git::Error::RebaseConflict(files)) => {
            // More conflicts
            return Ok(SyncResult::Paused {
                at_branch: sync_state.current_branch.clone(),
                conflict_files: files,
                backup_id,
            });
        }
        Err(e) => {
            return Err(e.into());
        }
    }

    // Process remaining branches
    for branch_name in sync_state.remaining.clone() {
        // Checkout the branch
        repo.checkout(&branch_name)?;

        // Get parent's current tip (we need to look this up from the stack)
        // For now, we'll get the merge base and target from the previous branch
        let stack = state.load_stack()?;
        let branch = stack
            .find_branch(&branch_name)
            .ok_or_else(|| crate::error::Error::NotInStack(branch_name.clone()))?;

        let parent_name = branch.parent.as_deref().unwrap_or("main");
        let parent_commit = repo.branch_commit(parent_name)?;

        // Rebase onto parent's tip
        match repo.rebase_onto(parent_commit) {
            Ok(()) => {
                sync_state.advance();
                state.save_sync_state(&sync_state)?;
            }
            Err(rung_git::Error::RebaseConflict(files)) => {
                state.save_sync_state(&sync_state)?;
                return Ok(SyncResult::Paused {
                    at_branch: branch_name,
                    conflict_files: files,
                    backup_id,
                });
            }
            Err(e) => {
                let _ = repo.rebase_abort();
                state.clear_sync_state()?;
                return Err(e.into());
            }
        }
    }

    // All done
    state.clear_sync_state()?;

    Ok(SyncResult::Complete {
        branches_rebased: sync_state.completed.len(),
        backup_id,
    })
}

/// Abort a paused sync and restore from backup.
///
/// # Errors
/// Returns error if no sync in progress or abort fails.
pub fn abort_sync(repo: &rung_git::Repository, state: &State) -> Result<()> {
    // Load sync state
    let sync_state = state.load_sync_state()?;

    // Abort any in-progress rebase
    if repo.is_rebasing() {
        let _ = repo.rebase_abort();
    }

    // Restore all branches from backup
    let refs = state.load_backup(&sync_state.backup_id)?;
    for (branch_name, sha) in refs {
        let oid = rung_git::Oid::from_str(&sha)
            .map_err(|e| crate::error::Error::RebaseFailed(branch_name.clone(), e.to_string()))?;
        repo.reset_branch(&branch_name, oid)?;
    }

    // Clear sync state
    state.clear_sync_state()?;

    Ok(())
}

/// Result of an undo operation.
#[derive(Debug)]
pub struct UndoResult {
    /// Number of branches restored.
    pub branches_restored: usize,
    /// The backup ID that was used.
    pub backup_id: String,
}

/// Undo the last sync operation.
///
/// Restores all branches to their state before the most recent sync.
///
/// # Errors
/// Returns error if no backup found or undo fails.
pub fn undo_sync(repo: &rung_git::Repository, state: &State) -> Result<UndoResult> {
    // Find latest backup
    let backup_id = state.latest_backup()?;
    let refs = state.load_backup(&backup_id)?;

    // Reset each branch to its saved SHA
    for (branch_name, sha) in &refs {
        let oid = rung_git::Oid::from_str(sha)
            .map_err(|e| crate::error::Error::RebaseFailed(branch_name.clone(), e.to_string()))?;
        repo.reset_branch(branch_name, oid)?;
    }

    let branches_restored = refs.len();

    // Delete the backup after successful restore
    state.delete_backup(&backup_id)?;

    Ok(UndoResult {
        branches_restored,
        backup_id,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::stack::StackBranch;
    use std::fs;
    use tempfile::TempDir;

    /// Create a test repository with an initial commit
    fn init_test_repo() -> (TempDir, rung_git::Repository, git2::Repository) {
        let temp = TempDir::new().unwrap();
        let git_repo = git2::Repository::init(temp.path()).unwrap();

        // Create initial commit
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        fs::write(temp.path().join("README.md"), "# Test").unwrap();

        let mut index = git_repo.index().unwrap();
        index.add_path(std::path::Path::new("README.md")).unwrap();
        index.write().unwrap();

        let tree_id = index.write_tree().unwrap();
        let tree = git_repo.find_tree(tree_id).unwrap();
        git_repo
            .commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
            .unwrap();
        drop(tree);

        let rung_repo = rung_git::Repository::open(temp.path()).unwrap();
        (temp, rung_repo, git_repo)
    }

    /// Add a commit to the current branch
    fn add_commit(temp: &TempDir, git_repo: &git2::Repository, filename: &str, message: &str) {
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        fs::write(temp.path().join(filename), "content").unwrap();

        let mut index = git_repo.index().unwrap();
        index.add_path(std::path::Path::new(filename)).unwrap();
        index.write().unwrap();

        let tree_id = index.write_tree().unwrap();
        let tree = git_repo.find_tree(tree_id).unwrap();
        let parent = git_repo.head().unwrap().peel_to_commit().unwrap();

        git_repo
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
            .unwrap();
    }

    #[test]
    fn test_sync_plan_empty_when_synced() {
        let (_temp, rung_repo, git_repo) = init_test_repo();

        // Get main branch name
        let main_branch = rung_repo.current_branch().unwrap();

        // Create feature branch at current HEAD
        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo.branch("feature-a", &head, false).unwrap();

        // Create stack with feature-a based on main
        let mut stack = Stack::new();
        stack.add_branch(StackBranch::new("feature-a", Some(main_branch.clone())));

        // Plan should be empty - feature-a is at same commit as main
        let plan = create_sync_plan(&rung_repo, &stack, &main_branch).unwrap();
        assert!(plan.is_empty());
    }

    #[test]
    fn test_sync_plan_detects_divergence() {
        let (temp, rung_repo, git_repo) = init_test_repo();

        // Get main branch name
        let main_branch = rung_repo.current_branch().unwrap();

        // Create feature branch at current HEAD
        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo.branch("feature-a", &head, false).unwrap();

        // Add a commit to main (making feature-a diverge)
        add_commit(&temp, &git_repo, "main-update.txt", "Update main");

        // Create stack with feature-a based on main
        let mut stack = Stack::new();
        stack.add_branch(StackBranch::new("feature-a", Some(main_branch.clone())));

        // Plan should have one action - rebase feature-a
        let plan = create_sync_plan(&rung_repo, &stack, &main_branch).unwrap();
        assert_eq!(plan.branches.len(), 1);
        assert_eq!(plan.branches[0].branch, "feature-a");
    }

    #[test]
    fn test_sync_plan_chain() {
        let (temp, rung_repo, git_repo) = init_test_repo();

        let main_branch = rung_repo.current_branch().unwrap();

        // Create feature-a at current HEAD
        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo.branch("feature-a", &head, false).unwrap();

        // Checkout feature-a and add a commit
        git_repo.set_head("refs/heads/feature-a").unwrap();
        git_repo
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        add_commit(&temp, &git_repo, "feature-a.txt", "Feature A commit");

        // Create feature-b based on feature-a
        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo.branch("feature-b", &head, false).unwrap();

        // Go back to main and add a commit
        git_repo
            .set_head(&format!("refs/heads/{main_branch}"))
            .unwrap();
        git_repo
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        add_commit(&temp, &git_repo, "main-update.txt", "Update main");

        // Create stack
        let mut stack = Stack::new();
        stack.add_branch(StackBranch::new("feature-a", Some(main_branch.clone())));
        stack.add_branch(StackBranch::new("feature-b", Some("feature-a".to_string())));

        // Plan should have feature-a needing rebase (feature-b is still synced with feature-a)
        let plan = create_sync_plan(&rung_repo, &stack, &main_branch).unwrap();
        assert_eq!(plan.branches.len(), 1);
        assert_eq!(plan.branches[0].branch, "feature-a");
    }
}
