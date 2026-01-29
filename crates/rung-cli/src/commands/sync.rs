//! `rung sync` command - Sync the stack by rebasing all branches.
//!
//! This command performs a full sync operation:
//! 1. Detects PRs merged externally (via GitHub UI)
//! 2. Updates stack topology for merged branches
//! 3. Rebases remaining branches onto their new parents
//! 4. Updates GitHub PR base branches
//! 5. Pushes all synced branches

use anyhow::{Context, Result, bail};
use rung_core::State;
use rung_core::sync::{self, ReconcileResult, SyncResult};
use rung_git::Repository;
use rung_github::{Auth, GitHubClient};
use serde::Serialize;

use crate::commands::utils;
use crate::output;
use crate::services::SyncService;

/// JSON output for sync command.
#[derive(Debug, Serialize)]
struct SyncOutput {
    status: SyncStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    branches_rebased: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    backup_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    conflict_branch: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    conflict_files: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum SyncStatus {
    AlreadySynced,
    Complete,
    Conflict,
    Aborted,
}

/// Run the sync command.
#[allow(clippy::fn_params_excessive_bools, clippy::too_many_lines)]
pub fn run(
    json: bool,
    dry_run: bool,
    continue_: bool,
    abort: bool,
    no_push: bool,
    base: Option<&str>,
) -> Result<()> {
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
        if json {
            return output_json(&SyncOutput {
                status: SyncStatus::Aborted,
                branches_rebased: None,
                backup_id: None,
                conflict_branch: None,
                conflict_files: vec![],
            });
        }
        output::success("Sync aborted - branches restored from backup");
        return Ok(());
    }

    // Ensure on branch (unless we are continuing/aborting a sync)
    if !continue_ && !abort {
        utils::ensure_on_branch(&repo)?;
    }

    // Handle continue
    if continue_ {
        if !state.is_sync_in_progress() {
            bail!("No sync in progress to continue");
        }
        if !json {
            output::info("Continuing sync...");
        }
        let result = sync::continue_sync(&repo, &state)?;

        // If sync completed successfully, push the branches
        if let SyncResult::Complete { .. } = &result {
            if !no_push {
                push_stack_branches(&repo, &state, json)?;
            }
        }

        return handle_sync_result(result, json);
    }

    // Check for existing sync in progress
    if state.is_sync_in_progress() {
        bail!("Sync already in progress - use --continue to resume or --abort to cancel");
    }

    // Ensure working directory is clean
    repo.require_clean()?;

    // Try to get GitHub remote info (optional - needed for PR operations)
    let github_info = repo
        .origin_url()
        .ok()
        .and_then(|url| Repository::parse_github_remote(&url).ok());

    // Determine base branch: use --base if provided, otherwise query GitHub
    let base_branch = if let Some(b) = base {
        b.to_string()
    } else {
        let (owner, repo_name) = github_info.as_ref().ok_or_else(|| {
            anyhow::anyhow!("No origin remote configured. Use --base <branch> to specify manually.")
        })?;
        let client = GitHubClient::new(&Auth::auto()).context(
            "GitHub auth required to detect default branch. Use --base <branch> to specify manually.",
        )?;
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(client.get_default_branch(owner, repo_name))
            .context("Could not fetch default branch. Use --base <branch> to specify manually.")?
    };

    // === Phase 0: Fetch base branch to ensure we have latest ===
    if !json {
        output::info(&format!("Fetching {base_branch}..."));
    }
    if let Err(e) = repo.fetch(&base_branch) {
        if !json {
            output::warn(&format!("Could not fetch {base_branch}: {e}"));
        }
        // Continue anyway - we'll work with what we have
    }

    // Create GitHub client and service for PR operations (requires origin remote)
    let client_and_info = github_info.as_ref().and_then(|(owner, repo_name)| {
        GitHubClient::new(&Auth::auto()).map_or_else(
            |_| {
                if !json {
                    output::warn("GitHub auth unavailable - skipping merge detection");
                }
                None
            },
            |c| Some((c, owner.clone(), repo_name.clone())),
        )
    });

    let rt = tokio::runtime::Runtime::new()?;

    // === Phase 1: Detect merged PRs and validate PR bases (Active Base Validation) ===
    let reconcile_result = if let Some((ref client, ref owner, ref repo_name)) = client_and_info {
        if !json {
            output::info("Checking PRs and validating bases...");
        }

        let service = SyncService::new(&repo, client, owner.clone(), repo_name.clone());
        let result = rt.block_on(service.detect_and_reconcile_merged(&state, &base_branch))?;

        // Output merged PR notifications
        if !json {
            for merged in &result.merged {
                output::success(&format!(
                    "PR #{} ({}) merged into {}",
                    merged.pr_number, merged.name, merged.merged_into
                ));
            }
            for reparent in &result.reparented {
                output::info(&format!(
                    "Re-parented {} → {} (was {})",
                    reparent.name, reparent.new_parent, reparent.old_parent
                ));
            }
            for repair in &result.repaired {
                output::warn(&format!(
                    "Ghost parent: PR #{} ({}) base was '{}', correcting to '{}'",
                    repair.pr_number.unwrap_or(0),
                    repair.name,
                    repair.old_parent,
                    repair.new_parent
                ));
            }
        }

        result
    } else {
        ReconcileResult::default()
    };

    // === Phase 2: Remove stale branches ===
    let stale_result = sync::remove_stale_branches(&repo, &state)?;
    if !json && !stale_result.removed.is_empty() {
        output::warn(&format!(
            "Removed {} stale branch(es) from stack:",
            stale_result.removed.len()
        ));
        for branch in &stale_result.removed {
            println!("  → {branch}");
        }
    }

    // Load stack (after reconcile and stale branch cleanup)
    let stack = state.load_stack()?;

    if stack.is_empty() {
        if json {
            return output_json(&SyncOutput {
                status: SyncStatus::AlreadySynced,
                branches_rebased: Some(0),
                backup_id: None,
                conflict_branch: None,
                conflict_files: vec![],
            });
        }
        output::info("No branches in stack - nothing to sync");
        return Ok(());
    }

    // === Phase 3: Create and execute sync plan ===
    let plan = sync::create_sync_plan(&repo, &stack, &base_branch)?;

    if dry_run {
        if !json {
            output::info("Dry run - would perform the following:");
            if !reconcile_result.merged.is_empty() {
                println!("  Merged PRs detected: {}", reconcile_result.merged.len());
            }
            if !plan.is_empty() {
                println!("  Branches to rebase:");
                for action in &plan.branches {
                    println!(
                        "    → {} (onto {})",
                        action.branch,
                        &action.new_base[..8.min(action.new_base.len())]
                    );
                }
            }
        }
        return Ok(());
    }

    let sync_result = if plan.is_empty() {
        SyncResult::AlreadySynced
    } else {
        if !json {
            output::info(&format!("Syncing {} branches...", plan.branches.len()));
        }
        sync::execute_sync(&repo, &state, plan)?
    };

    // If sync paused on conflict, don't proceed with push/update
    if let SyncResult::Paused { .. } = &sync_result {
        return handle_sync_result(sync_result, json);
    }

    // === Phase 4: Update GitHub PR base branches (reparented + repaired) ===
    if let Some((ref client, ref owner, ref repo_name)) = client_and_info {
        if !reconcile_result.reparented.is_empty() || !reconcile_result.repaired.is_empty() {
            if !json {
                output::info("Updating PR base branches on GitHub...");
            }

            let service = SyncService::new(&repo, client, owner.clone(), repo_name.clone());
            rt.block_on(service.update_pr_bases(&reconcile_result))?;

            if !json {
                for reparent in reconcile_result
                    .reparented
                    .iter()
                    .chain(reconcile_result.repaired.iter())
                {
                    if let Some(pr_num) = reparent.pr_number {
                        output::success(&format!(
                            "Updated PR #{pr_num} base: {} → {}",
                            reparent.old_parent, reparent.new_parent
                        ));
                    }
                }
            }
        }
    }

    // === Phase 5: Push all branches ===
    if !no_push {
        push_stack_branches(&repo, &state, json)?;
    }

    handle_sync_result(sync_result, json)
}

/// Push all branches in the stack to remote.
fn push_stack_branches(repo: &Repository, state: &State, json: bool) -> Result<()> {
    let stack = state.load_stack()?;

    if stack.is_empty() {
        return Ok(());
    }

    if !json {
        output::info("Pushing to remote...");
    }

    let mut pushed = 0;
    for branch in &stack.branches {
        if repo.branch_exists(&branch.name) {
            match repo.push(&branch.name, true) {
                Ok(()) => {
                    pushed += 1;
                }
                Err(e) => {
                    if !json {
                        output::warn(&format!("Could not push {}: {e}", branch.name));
                    }
                }
            }
        }
    }

    if !json && pushed > 0 {
        output::success(&format!("Pushed {pushed} branch(es)"));
    }

    Ok(())
}

#[allow(clippy::unnecessary_wraps)]
fn handle_sync_result(result: SyncResult, json: bool) -> Result<()> {
    match result {
        SyncResult::AlreadySynced => {
            if json {
                return output_json(&SyncOutput {
                    status: SyncStatus::AlreadySynced,
                    branches_rebased: Some(0),
                    backup_id: None,
                    conflict_branch: None,
                    conflict_files: vec![],
                });
            }
            output::success("Stack is already up-to-date");
        }
        SyncResult::Complete {
            branches_rebased,
            backup_id,
        } => {
            if json {
                return output_json(&SyncOutput {
                    status: SyncStatus::Complete,
                    branches_rebased: Some(branches_rebased),
                    backup_id: Some(backup_id),
                    conflict_branch: None,
                    conflict_files: vec![],
                });
            }
            output::success(&format!(
                "Synced {branches_rebased} branches (backup: {})",
                &backup_id[..8.min(backup_id.len())]
            ));
        }
        SyncResult::Paused {
            at_branch,
            conflict_files,
            backup_id,
        } => {
            if json {
                return output_json(&SyncOutput {
                    status: SyncStatus::Conflict,
                    branches_rebased: None,
                    backup_id: Some(backup_id),
                    conflict_branch: Some(at_branch),
                    conflict_files,
                });
            }
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

/// Output sync result as JSON.
fn output_json(output: &SyncOutput) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(output)?);
    Ok(())
}
