//! `rung sync` command - Sync the stack by rebasing all branches.

use anyhow::{Context, Result, bail};
use rung_core::State;
use rung_core::sync::{self, SyncResult};
use rung_git::Repository;

use crate::output;

/// Run the sync command.
pub fn run(dry_run: bool, continue_: bool, abort: bool, base: Option<&str>) -> Result<()> {
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
        sync::abort_sync(&repo, &state)?;
        output::success("Sync aborted - branches restored from backup");
        return Ok(());
    }

    // Handle continue
    if continue_ {
        if !state.is_sync_in_progress() {
            bail!("No sync in progress to continue");
        }
        output::info("Continuing sync...");
        return handle_sync_result(sync::continue_sync(&repo, &state)?);
    }

    // Check for existing sync in progress
    if state.is_sync_in_progress() {
        bail!("Sync already in progress - use --continue to resume or --abort to cancel");
    }

    // Ensure working directory is clean
    repo.require_clean()?;

    // Check for and remove stale branches (branches in stack but not in git)
    let stale_result = sync::remove_stale_branches(&repo, &state)?;
    if !stale_result.removed.is_empty() {
        output::warn(&format!(
            "Removed {} stale branch(es) from stack:",
            stale_result.removed.len()
        ));
        for branch in &stale_result.removed {
            println!("  → {branch}");
        }
    }

    // Load stack (after stale branch cleanup)
    let stack = state.load_stack()?;

    if stack.is_empty() {
        output::info("No branches in stack - nothing to sync");
        return Ok(());
    }

    // Determine base branch
    let base_branch = base.unwrap_or("main");

    // Create sync plan
    let plan = sync::create_sync_plan(&repo, &stack, base_branch)?;

    if plan.is_empty() {
        output::success("Stack is already up-to-date");
        return Ok(());
    }

    if dry_run {
        output::info("Dry run - would rebase the following branches:");
        for action in &plan.branches {
            println!(
                "  → {} (onto {})",
                action.branch,
                &action.new_base[..8.min(action.new_base.len())]
            );
        }
        return Ok(());
    }

    // Execute sync
    output::info(&format!("Syncing {} branches...", plan.branches.len()));
    handle_sync_result(sync::execute_sync(&repo, &state, plan)?)
}

#[allow(clippy::unnecessary_wraps)]
fn handle_sync_result(result: SyncResult) -> Result<()> {
    match result {
        SyncResult::AlreadySynced => {
            output::success("Stack is already up-to-date");
        }
        SyncResult::Complete {
            branches_rebased,
            backup_id,
        } => {
            output::success(&format!(
                "Synced {} branches (backup: {})",
                branches_rebased,
                &backup_id[..8.min(backup_id.len())]
            ));
        }
        SyncResult::Paused {
            at_branch,
            conflict_files,
            ..
        } => {
            output::warn(&format!("Conflict in branch '{at_branch}'"));
            output::info("Conflicting files:");
            for file in &conflict_files {
                println!("  → {file}");
            }
            println!();
            output::info("Resolve conflicts, then run: rung sync --continue");
            output::info("Or abort with: rung sync --abort");
        }
    }
    Ok(())
}
