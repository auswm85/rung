use super::types::{SyncPlan, SyncResult};
use crate::error::Result;
use crate::state::SyncState;
use crate::traits::StateStore;
/// Execute a sync operation.
///
/// Rebases all branches in the plan onto their new bases. If a conflict occurs,
/// the sync is paused and can be continued with `continue_sync` after resolution.
///
/// # Errors
/// Returns error if sync fails.
pub fn execute_sync(
    repo: &impl rung_git::GitOps,
    state: &impl StateStore,
    plan: SyncPlan,
) -> Result<SyncResult> {
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
/// If the user already ran `git rebase --continue` manually, this will detect
/// that no rebase is in progress and proceed with remaining branches.
///
/// # Errors
/// Returns error if no sync in progress or continuation fails.
pub fn continue_sync(repo: &impl rung_git::GitOps, state: &impl StateStore) -> Result<SyncResult> {
    // Load sync state
    let mut sync_state = state.load_sync_state()?;
    let backup_id = sync_state.backup_id.clone();

    // Check if a rebase is actually in progress
    // If user ran `git rebase --continue` manually, there won't be one
    if repo.is_rebasing() {
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
                let _ = repo.rebase_abort(); // Best effort
                state.clear_sync_state()?;
                return Err(e.into());
            }
        }
    } else {
        // No rebase in progress - user may have completed it manually
        // Verify the rebase actually succeeded before advancing
        let current_branch = &sync_state.current_branch;
        let stack = state.load_stack()?;
        let branch = stack
            .find_branch(current_branch)
            .ok_or_else(|| crate::error::Error::NotInStack(current_branch.clone()))?;

        let default_branch = state.default_branch()?;
        let parent_name = branch.parent.as_deref().unwrap_or(&default_branch);

        let parent_commit = repo.branch_commit(parent_name)?;
        let current_commit = repo.branch_commit(current_branch)?;

        // Verify parent is an ancestor of current (meaning rebase succeeded)
        let merge_base = repo.merge_base(parent_commit, current_commit)?;
        if merge_base != parent_commit {
            return Err(crate::error::Error::SyncFailed(format!(
                "Rebase verification failed for '{current_branch}': parent '{parent_name}' \
                 is not an ancestor. The rebase may not have completed correctly. \
                 Please run `rung sync --abort` and try again."
            )));
        }

        // Verification passed - advance to next branch
        sync_state.advance();
        state.save_sync_state(&sync_state)?;
    }

    // Process remaining branches (including the one moved to current_branch by advance())
    // Use while loop since advance() moves next branch from remaining to current_branch
    while !sync_state.current_branch.is_empty() {
        let branch_name = sync_state.current_branch.clone();

        // Checkout the branch
        repo.checkout(&branch_name)?;

        // Get parent's current tip (we need to look this up from the stack)
        let stack = state.load_stack()?;
        let branch = stack
            .find_branch(&branch_name)
            .ok_or_else(|| crate::error::Error::NotInStack(branch_name.clone()))?;

        let default_branch = state.default_branch()?;
        let parent_name = branch.parent.as_deref().unwrap_or(&default_branch);
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
pub fn abort_sync(repo: &impl rung_git::GitOps, state: &impl StateStore) -> Result<()> {
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
