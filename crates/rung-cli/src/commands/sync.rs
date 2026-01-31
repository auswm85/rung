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
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    github_auth_unavailable: bool,
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
#[allow(clippy::fn_params_excessive_bools)]
pub fn run(
    json: bool,
    dry_run: bool,
    continue_: bool,
    abort: bool,
    no_push: bool,
    base: Option<&str>,
) -> Result<()> {
    let repo = Repository::open_current().context("Not inside a git repository")?;
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    if !state.is_initialized() {
        bail!("Rung not initialized - run `rung init` first");
    }

    if continue_ && abort {
        bail!("Cannot use --continue and --abort together");
    }

    // Handle abort (no GitHub needed)
    if abort {
        return handle_abort(&repo, &state, json);
    }

    // Ensure on branch (unless continuing)
    if !continue_ {
        utils::ensure_on_branch(&repo)?;
    }

    // Handle continue (no GitHub needed)
    if continue_ {
        return handle_continue(&repo, &state, json, no_push);
    }

    // Check for existing sync in progress
    if state.is_sync_in_progress() {
        bail!("Sync already in progress - use --continue to resume or --abort to cancel");
    }

    repo.require_clean()?;

    // Try to get GitHub remote info (optional - needed for PR operations)
    let github_info = repo
        .origin_url()
        .ok()
        .and_then(|url| Repository::parse_github_remote(&url).ok());

    // Determine base branch
    let base_branch = determine_base_branch(base, github_info.as_ref())?;

    // Fetch base branch
    if !json {
        output::info(&format!("Fetching {base_branch}..."));
    }
    if let Err(e) = repo.fetch(&base_branch)
        && !json
    {
        output::warn(&format!("Could not fetch {base_branch}: {e}"));
    }

    // Create GitHub client (if available)
    let mut github_auth_unavailable = false;
    let client = github_info.as_ref().and_then(|_| {
        GitHubClient::new(&Auth::auto())
            .map_err(|_| {
                github_auth_unavailable = true;
                if !json {
                    output::warn("GitHub auth unavailable - skipping merge detection");
                }
            })
            .ok()
    });

    let rt = tokio::runtime::Runtime::new()?;

    // Run the main sync phases
    run_sync_phases(
        &repo,
        &state,
        &base_branch,
        github_info.as_ref(),
        client.as_ref(),
        &rt,
        json,
        dry_run,
        no_push,
        github_auth_unavailable,
    )
}

/// Handle --abort flag.
fn handle_abort(repo: &Repository, state: &State, json: bool) -> Result<()> {
    if !state.is_sync_in_progress() {
        bail!("No sync in progress to abort");
    }
    sync::abort_sync(repo, state)?;
    if json {
        return output_json(&SyncOutput {
            status: SyncStatus::Aborted,
            branches_rebased: None,
            backup_id: None,
            conflict_branch: None,
            conflict_files: vec![],
            github_auth_unavailable: false,
        });
    }
    output::success("Sync aborted - branches restored from backup");
    Ok(())
}

/// Handle --continue flag.
fn handle_continue(repo: &Repository, state: &State, json: bool, no_push: bool) -> Result<()> {
    if !state.is_sync_in_progress() {
        bail!("No sync in progress to continue");
    }
    if !json {
        output::info("Continuing sync...");
    }
    let result = sync::continue_sync(repo, state)?;

    // If sync completed successfully, push the branches
    if let SyncResult::Complete { .. } = &result
        && !no_push
    {
        push_stack_branches(repo, state, json)?;
    }

    // Check GitHub auth availability for accurate JSON output
    let github_auth_unavailable = GitHubClient::new(&Auth::auto()).is_err();

    handle_sync_result(result, json, github_auth_unavailable)
}

/// Determine base branch from --base flag or GitHub API.
fn determine_base_branch(
    base: Option<&str>,
    github_info: Option<&(String, String)>,
) -> Result<String> {
    if let Some(b) = base {
        return Ok(b.to_string());
    }

    let (owner, repo_name) = github_info.ok_or_else(|| {
        anyhow::anyhow!(
            "No GitHub origin remote detected. Use --base <branch> to specify manually."
        )
    })?;
    let client = GitHubClient::new(&Auth::auto()).context(
        "GitHub auth required to detect default branch. Use --base <branch> to specify manually.",
    )?;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client.get_default_branch(owner, repo_name))
        .context("Could not fetch default branch. Use --base <branch> to specify manually.")
}

/// Run the main sync phases.
#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn run_sync_phases(
    repo: &Repository,
    state: &State,
    base_branch: &str,
    github_info: Option<&(String, String)>,
    client: Option<&GitHubClient>,
    rt: &tokio::runtime::Runtime,
    json: bool,
    dry_run: bool,
    no_push: bool,
    github_auth_unavailable: bool,
) -> Result<()> {
    // Phase 1: Detect merged PRs (if GitHub available)
    let reconcile_result = match (client, github_info) {
        (Some(client), Some((owner, repo_name))) => {
            if !json {
                output::info("Checking PRs and validating bases...");
            }
            let service = SyncService::new(repo, client, owner.clone(), repo_name.clone());
            let result = rt.block_on(service.detect_and_reconcile_merged(state, base_branch))?;
            print_reconcile_results(&result, json);
            result
        }
        _ => ReconcileResult::default(),
    };

    // Phase 2: Remove stale branches (via service if available, else direct)
    let stale_result = if let (Some(client), Some((owner, repo_name))) = (client, github_info) {
        let service = SyncService::new(repo, client, owner.clone(), repo_name.clone());
        service.remove_stale_branches(state)?
    } else {
        sync::remove_stale_branches(repo, state)?
    };

    if !json && !stale_result.removed.is_empty() {
        output::warn(&format!(
            "Removed {} stale branch(es) from stack:",
            stale_result.removed.len()
        ));
        for branch in &stale_result.removed {
            println!("  → {branch}");
        }
    }

    // Load stack after cleanup
    let stack = state.load_stack()?;

    if stack.is_empty() {
        if json {
            return output_json(&SyncOutput {
                status: SyncStatus::AlreadySynced,
                branches_rebased: Some(0),
                backup_id: None,
                conflict_branch: None,
                conflict_files: vec![],
                github_auth_unavailable,
            });
        }
        output::info("No branches in stack - nothing to sync");
        return Ok(());
    }

    // Phase 3: Create and execute sync plan (via service if available)
    let plan = if let (Some(client), Some((owner, repo_name))) = (client, github_info) {
        let service = SyncService::new(repo, client, owner.clone(), repo_name.clone());
        service.create_sync_plan(&stack, base_branch)?
    } else {
        sync::create_sync_plan(repo, &stack, base_branch)?
    };

    if dry_run {
        print_dry_run(&plan, &reconcile_result, json);
        return Ok(());
    }

    let sync_result = if plan.is_empty() {
        SyncResult::AlreadySynced
    } else {
        if !json {
            output::info(&format!("Syncing {} branches...", plan.branches.len()));
        }
        if let (Some(client), Some((owner, repo_name))) = (client, github_info) {
            let service = SyncService::new(repo, client, owner.clone(), repo_name.clone());
            service.execute_sync(state, plan)?
        } else {
            sync::execute_sync(repo, state, plan)?
        }
    };

    // If paused on conflict, don't proceed
    if let SyncResult::Paused { .. } = &sync_result {
        return handle_sync_result(sync_result, json, github_auth_unavailable);
    }

    // Phase 4: Update GitHub PR bases
    if let (Some(client), Some((owner, repo_name))) = (client, github_info)
        && (!reconcile_result.reparented.is_empty() || !reconcile_result.repaired.is_empty())
    {
        if !json {
            output::info("Updating PR base branches on GitHub...");
        }
        let service = SyncService::new(repo, client, owner.clone(), repo_name.clone());
        rt.block_on(service.update_pr_bases(&reconcile_result))?;
        print_pr_updates(&reconcile_result, json);
    }

    // Phase 5: Push all branches
    if !no_push {
        if let (Some(client), Some((owner, repo_name))) = (client, github_info) {
            let service = SyncService::new(repo, client, owner.clone(), repo_name.clone());
            let push_results = service.push_stack_branches(state)?;
            if !json {
                let pushed = push_results.iter().filter(|p| p.success).count();
                for result in push_results.iter().filter(|p| !p.success) {
                    output::warn(&format!("Could not push {}", result.branch));
                }
                if pushed > 0 {
                    output::success(&format!("Pushed {pushed} branch(es)"));
                }
            }
        } else {
            push_stack_branches(repo, state, json)?;
        }
    }

    handle_sync_result(sync_result, json, github_auth_unavailable)
}

/// Print reconcile results.
fn print_reconcile_results(result: &ReconcileResult, json: bool) {
    if json {
        return;
    }
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
        let pr_display = repair
            .pr_number
            .map_or_else(|| "no PR".to_string(), |n| format!("PR #{n}"));
        output::warn(&format!(
            "Ghost parent: {pr_display} ({}) base was '{}', correcting to '{}'",
            repair.name, repair.old_parent, repair.new_parent
        ));
    }
}

/// Print dry run info.
fn print_dry_run(plan: &rung_core::sync::SyncPlan, reconcile_result: &ReconcileResult, json: bool) {
    if json {
        return;
    }
    output::info("Dry run - would perform the following:");
    if !reconcile_result.merged.is_empty() {
        println!("  Merged PRs detected: {}", reconcile_result.merged.len());
    }
    if !plan.is_empty() {
        println!("  Branches to rebase:");
        for action in &plan.branches {
            // Use char-safe truncation to avoid UTF-8 boundary panic
            let base_short: String = action.new_base.chars().take(8).collect();
            println!("    → {} (onto {base_short})", action.branch);
        }
    }
}

/// Print PR update results.
fn print_pr_updates(reconcile_result: &ReconcileResult, json: bool) {
    if json {
        return;
    }
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
                Ok(()) => pushed += 1,
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
fn handle_sync_result(result: SyncResult, json: bool, github_auth_unavailable: bool) -> Result<()> {
    match result {
        SyncResult::AlreadySynced => {
            if json {
                return output_json(&SyncOutput {
                    status: SyncStatus::AlreadySynced,
                    branches_rebased: Some(0),
                    backup_id: None,
                    conflict_branch: None,
                    conflict_files: vec![],
                    github_auth_unavailable,
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
                    github_auth_unavailable,
                });
            }
            // Use char-safe truncation for backup_id display
            let backup_short: String = backup_id.chars().take(8).collect();
            output::success(&format!(
                "Synced {branches_rebased} branches (backup: {backup_short})"
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
                    github_auth_unavailable,
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
