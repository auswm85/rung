//! `rung undo` command - Undo the last sync operation.

use anyhow::{Context, Result};
use rung_core::State;
use rung_git::Repository;

use crate::output;

/// Run the undo command.
pub fn run() -> Result<()> {
    // Open repository
    let repo = Repository::open_current().context("Not inside a git repository")?;

    // Get state manager
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    // Find latest backup
    let backup_id = state.latest_backup()?;
    let refs = state.load_backup(&backup_id)?;

    output::info(&format!("Restoring from backup {backup_id}"));

    // Restore each branch
    for (branch_name, sha) in &refs {
        let oid =
            rung_git::Oid::from_str(sha).map_err(|e| anyhow::anyhow!("Invalid SHA {sha}: {e}"))?;
        repo.reset_branch(branch_name, oid)?;
        output::success(&format!("Restored {branch_name} to {}", &sha[..8]));
    }

    // Delete the backup
    state.delete_backup(&backup_id)?;

    output::success("Undo complete");

    Ok(())
}
