use super::types::UndoResult;
use crate::error::Result;
use crate::traits::StateStore;

/// Undo the last sync operation.
///
/// Restores all branches to their state before the most recent sync.
///
/// # Errors
/// Returns error if no backup found or undo fails.
pub fn undo_sync(repo: &impl rung_git::GitOps, state: &impl StateStore) -> Result<UndoResult> {
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
