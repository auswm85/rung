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
use rung_core::sync::{self, ExternalMergeInfo, ReconcileResult, ReparentedBranch, SyncResult};
use rung_git::Repository;
use rung_github::{Auth, GitHubClient, PullRequestState, UpdatePullRequest};
use serde::Serialize;

use crate::output;

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

    // Determine base branch: use --base if provided, otherwise query GitHub
    let base_branch = if let Some(b) = base {
        b.to_string()
    } else {
        let origin_url = repo.origin_url().context("No origin remote configured")?;
        let (owner, repo_name) = Repository::parse_github_remote(&origin_url)
            .context("Could not parse GitHub remote URL")?;

        let client = GitHubClient::new(&Auth::auto()).context(
            "GitHub auth required to detect default branch. Use --base <branch> to specify manually.",
        )?;
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(client.get_default_branch(&owner, &repo_name))
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

    // === Phase 1: Detect merged PRs and validate PR bases (Active Base Validation) ===
    let reconcile_result = detect_and_reconcile_merged(&repo, &state, json, &base_branch)?;

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
    if !reconcile_result.reparented.is_empty() || !reconcile_result.repaired.is_empty() {
        update_pr_bases(&repo, &reconcile_result, json)?;
    }

    // === Phase 5: Push all branches ===
    if !no_push {
        push_stack_branches(&repo, &state, json)?;
    }

    handle_sync_result(sync_result, json)
}

/// Threshold for switching from individual REST calls to batched GraphQL query.
/// For stacks with more than this many PRs, we use a single GraphQL call instead
/// of N individual REST calls to reduce API usage.
const BATCH_THRESHOLD: usize = 5;

/// Detect merged PRs via GitHub API, validate PR bases, and reconcile the stack.
///
/// This function performs two key operations:
/// 1. Detects PRs that were merged externally (via GitHub UI)
/// 2. Validates that each PR's base branch matches what stack.json expects ("Active Base Validation")
///
/// The second check is a "self-healing" mechanism that detects "ghost parents" - PRs whose
/// base branch on GitHub points to a deleted branch or doesn't match the stack's expectation.
///
/// For efficiency, uses GraphQL batch fetching when there are more than 5 PRs to check.
fn detect_and_reconcile_merged(
    repo: &Repository,
    state: &State,
    json: bool,
    base_branch: &str,
) -> Result<ReconcileResult> {
    let stack = state.load_stack()?;

    // Collect branches with PRs to check
    let branches_with_prs: Vec<_> = stack
        .branches
        .iter()
        .filter_map(|b| b.pr.map(|pr| (b.name.to_string(), b.parent.clone(), pr)))
        .collect();

    if branches_with_prs.is_empty() {
        return Ok(ReconcileResult::default());
    }

    // Get GitHub client
    let origin_url = repo.origin_url().context("No origin remote configured")?;
    let (owner, repo_name) = Repository::parse_github_remote(&origin_url)
        .context("Could not parse GitHub remote URL")?;

    let Ok(client) = GitHubClient::new(&Auth::auto()) else {
        // If GitHub auth fails, skip merge detection but continue with sync
        if !json {
            output::warn("GitHub auth unavailable - skipping merge detection");
        }
        return Ok(ReconcileResult::default());
    };

    let rt = tokio::runtime::Runtime::new()?;

    if !json {
        output::info("Checking PRs and validating bases...");
    }

    // Check each PR's status and validate base branches
    let mut merged_prs = Vec::new();
    let mut ghost_parents = Vec::new();

    // Use batch fetch for larger stacks to reduce API calls
    if branches_with_prs.len() > BATCH_THRESHOLD {
        // Batch fetch all PRs in a single GraphQL call
        let pr_numbers: Vec<u64> = branches_with_prs.iter().map(|(_, _, pr)| *pr).collect();

        let batch_result = rt.block_on(client.get_prs_batch(&owner, &repo_name, &pr_numbers));

        match batch_result {
            Ok(pr_map) => {
                // Process the batch results
                for (branch_name, stack_parent, pr_number) in &branches_with_prs {
                    if let Some(pr) = pr_map.get(pr_number) {
                        process_pr_result(
                            pr,
                            branch_name,
                            stack_parent.as_ref(),
                            *pr_number,
                            base_branch,
                            json,
                            &mut merged_prs,
                            &mut ghost_parents,
                        );
                    } else if !json {
                        output::warn(&format!("Could not fetch PR #{pr_number}"));
                    }
                }
            }
            Err(e) => {
                if !json {
                    output::warn(&format!(
                        "Batch PR fetch failed, falling back to individual: {e}"
                    ));
                }
                // Fall back to individual fetches on actual failure
                fetch_prs_individually(
                    &rt,
                    &client,
                    &owner,
                    &repo_name,
                    &branches_with_prs,
                    base_branch,
                    json,
                    &mut merged_prs,
                    &mut ghost_parents,
                );
            }
        }
    } else {
        // Small stack: use individual REST calls
        fetch_prs_individually(
            &rt,
            &client,
            &owner,
            &repo_name,
            &branches_with_prs,
            base_branch,
            json,
            &mut merged_prs,
            &mut ghost_parents,
        );
    }

    // If no merged PRs, just return with ghost parent repairs
    if merged_prs.is_empty() {
        return Ok(ReconcileResult {
            merged: vec![],
            reparented: vec![],
            repaired: ghost_parents,
        });
    }

    // Reconcile the stack for merged PRs
    let mut result = sync::reconcile_merged(state, &merged_prs)?;

    // Add ghost parent repairs
    result.repaired = ghost_parents;

    // Report re-parented branches
    if !json {
        for reparent in &result.reparented {
            output::info(&format!(
                "Re-parented {} → {} (was {})",
                reparent.name, reparent.new_parent, reparent.old_parent
            ));
        }
    }

    Ok(result)
}

/// Fetch PRs individually using REST API (for small stacks or as fallback).
#[allow(clippy::too_many_arguments)]
fn fetch_prs_individually(
    rt: &tokio::runtime::Runtime,
    client: &GitHubClient,
    owner: &str,
    repo_name: &str,
    branches_with_prs: &[(String, Option<rung_core::BranchName>, u64)],
    base_branch: &str,
    json: bool,
    merged_prs: &mut Vec<ExternalMergeInfo>,
    ghost_parents: &mut Vec<ReparentedBranch>,
) {
    for (branch_name, stack_parent, pr_number) in branches_with_prs {
        let pr_result = rt.block_on(client.get_pr(owner, repo_name, *pr_number));

        match pr_result {
            Ok(pr) => {
                process_pr_result(
                    &pr,
                    branch_name,
                    stack_parent.as_ref(),
                    *pr_number,
                    base_branch,
                    json,
                    merged_prs,
                    ghost_parents,
                );
            }
            Err(e) => {
                // Log but don't fail - PR might have been deleted
                if !json {
                    output::warn(&format!("Could not check PR #{pr_number}: {e}"));
                }
            }
        }
    }
}

/// Process a fetched PR: detect merges and ghost parents.
#[allow(clippy::too_many_arguments)]
fn process_pr_result(
    pr: &rung_github::PullRequest,
    branch_name: &str,
    stack_parent: Option<&rung_core::BranchName>,
    pr_number: u64,
    base_branch: &str,
    json: bool,
    merged_prs: &mut Vec<ExternalMergeInfo>,
    ghost_parents: &mut Vec<ReparentedBranch>,
) {
    if pr.state == PullRequestState::Merged {
        // PR was merged externally
        if !json {
            output::success(&format!(
                "PR #{} ({}) merged into {}",
                pr_number, branch_name, pr.base_branch
            ));
        }

        merged_prs.push(ExternalMergeInfo {
            branch_name: branch_name.to_string(),
            pr_number,
            merged_into: pr.base_branch.clone(),
        });
    } else {
        // PR is still open - validate its base matches our expectation
        let expected_base = stack_parent.map_or(base_branch, |p| p.as_str());

        if pr.base_branch != expected_base {
            // Ghost parent detected! PR base doesn't match stack.json
            if !json {
                output::warn(&format!(
                    "Ghost parent: PR #{} ({}) base is '{}' but should be '{}'",
                    pr_number, branch_name, pr.base_branch, expected_base
                ));
            }

            ghost_parents.push(ReparentedBranch {
                name: branch_name.to_string(),
                old_parent: pr.base_branch.clone(),
                new_parent: expected_base.to_string(),
                pr_number: Some(pr_number),
            });
        }
    }
}

/// Update GitHub PR base branches for re-parented and repaired branches.
///
/// Implements a no-op check: re-fetches current PR state before PATCH to avoid
/// redundant updates that would trigger unnecessary CI builds and PR timeline noise.
fn update_pr_bases(
    repo: &Repository,
    reconcile_result: &ReconcileResult,
    json: bool,
) -> Result<()> {
    // Collect all PRs that need updating
    let updates_needed: Vec<_> = reconcile_result
        .reparented
        .iter()
        .chain(reconcile_result.repaired.iter())
        .filter_map(|r| {
            r.pr_number
                .map(|pr| (pr, r.new_parent.clone(), r.old_parent.clone()))
        })
        .collect();

    if updates_needed.is_empty() {
        return Ok(());
    }

    let origin_url = repo.origin_url().context("No origin remote configured")?;
    let (owner, repo_name) = Repository::parse_github_remote(&origin_url)
        .context("Could not parse GitHub remote URL")?;

    let client = GitHubClient::new(&Auth::auto()).context("Failed to authenticate with GitHub")?;
    let rt = tokio::runtime::Runtime::new()?;

    if !json {
        output::info("Updating PR base branches on GitHub...");
    }

    // Re-fetch current PR states to implement no-op check
    // This prevents redundant PATCH requests that would trigger CI builds
    let pr_numbers: Vec<u64> = updates_needed.iter().map(|(pr, _, _)| *pr).collect();

    let current_states: std::collections::HashMap<u64, String> =
        if pr_numbers.len() > BATCH_THRESHOLD {
            // Use batch fetch for efficiency
            rt.block_on(client.get_prs_batch(&owner, &repo_name, &pr_numbers))
                .map_or_else(
                    |_| fetch_current_bases(&rt, &client, &owner, &repo_name, &pr_numbers),
                    |prs| {
                        prs.into_iter()
                            .map(|(num, pr)| (num, pr.base_branch))
                            .collect()
                    },
                )
        } else {
            fetch_current_bases(&rt, &client, &owner, &repo_name, &pr_numbers)
        };

    // Apply updates with no-op check
    for (pr_number, new_base, old_base) in updates_needed {
        // No-op check: skip if PR base is already what we want
        if let Some(current_base) = current_states.get(&pr_number) {
            if current_base == &new_base {
                if !json {
                    output::info(&format!(
                        "PR #{pr_number} base already '{new_base}' - skipping"
                    ));
                }
                continue;
            }
        }

        let update = UpdatePullRequest {
            title: None,
            body: None,
            base: Some(new_base.clone()),
        };

        match rt.block_on(client.update_pr(&owner, &repo_name, pr_number, update)) {
            Ok(_) => {
                if !json {
                    output::success(&format!(
                        "Updated PR #{pr_number} base: {old_base} → {new_base}"
                    ));
                }
            }
            Err(e) => {
                if !json {
                    output::warn(&format!("Could not update PR #{pr_number}: {e}"));
                }
            }
        }
    }

    Ok(())
}

/// Fetch current base branches for a list of PRs individually.
fn fetch_current_bases(
    rt: &tokio::runtime::Runtime,
    client: &GitHubClient,
    owner: &str,
    repo_name: &str,
    pr_numbers: &[u64],
) -> std::collections::HashMap<u64, String> {
    let mut result = std::collections::HashMap::new();
    for &pr_number in pr_numbers {
        if let Ok(pr) = rt.block_on(client.get_pr(owner, repo_name, pr_number)) {
            result.insert(pr_number, pr.base_branch);
        }
    }
    result
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
