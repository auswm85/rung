//! `rung sync` command - Sync the stack by rebasing all branches.

use anyhow::{Context, Result, bail};
use rung_core::State;
use rung_git::Repository;

use crate::output;

/// Run the sync command.
pub fn run(dry_run: bool, continue_: bool, abort: bool) -> Result<()> {
    // Open repository
    let repo = Repository::open_current().context("Not inside a git repository")?;

    // Get state manager
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    // Ensure initialized
    if !state.is_initialized() {
        bail!("Rung not initialized - run `rung init` first");
    }

    // Check for conflicting flags
    if continue_ && abort {
        bail!("Cannot use --continue and --abort together");
    }

    // Handle abort
    if abort {
        if !state.is_sync_in_progress() {
            bail!("No sync in progress to abort");
        }
        // TODO: Implement abort logic
        rung_core::sync::abort_sync(&repo, &state)?;
        output::success("Sync aborted - branches restored from backup");
        return Ok(());
    }

    // Handle continue
    if continue_ {
        if !state.is_sync_in_progress() {
            bail!("No sync in progress to continue");
        }
        // TODO: Implement continue logic
        output::info("Continuing sync...");
        output::warn("Sync continuation not yet implemented");
        return Ok(());
    }

    // Check for existing sync in progress
    if state.is_sync_in_progress() {
        bail!("Sync already in progress - use --continue to resume or --abort to cancel");
    }

    // Ensure working directory is clean
    repo.require_clean()?;

    // Load stack
    let stack = state.load_stack()?;

    if stack.is_empty() {
        output::info("No branches in stack - nothing to sync");
        return Ok(());
    }

    if dry_run {
        output::info("Dry run - would sync the following branches:");
        for branch in &stack.branches {
            println!("  → {}", branch.name);
        }
        return Ok(());
    }

    // TODO: Implement actual sync
    output::warn("Sync not yet fully implemented");
    output::info("Would sync branches in order:");
    for branch in &stack.branches {
        println!("  → {}", branch.name);
    }

    Ok(())
}
