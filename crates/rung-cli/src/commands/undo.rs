//! `rung undo` command - Undo the last sync operation.

use anyhow::{Context, Result, bail};
use rung_core::State;
use rung_core::sync;
use rung_git::Repository;

use crate::output;

/// Run the undo command.
pub fn run() -> Result<()> {
    // Open repository
    let repo = Repository::open_current().context("Not inside a git repository")?;

    // Get state manager
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    // Ensure initialized
    if !state.is_initialized() {
        bail!("Rung not initialized - run `rung init` first");
    }

    // Perform undo
    let result = sync::undo_sync(&repo, &state)?;

    output::success(&format!(
        "Restored {} branches from backup {}",
        result.branches_restored,
        &result.backup_id[..8.min(result.backup_id.len())]
    ));

    Ok(())
}
