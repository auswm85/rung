//! `rung restack` command - Move a branch to a different parent.
//!
//! This command reparents a branch by rebasing it onto a new parent branch,
//! updating the stack topology accordingly. Supports interruption recovery
//! via `--continue` and `--abort` flags.

use anyhow::{Context, Result, bail};
use inquire::Select;
use rung_core::{DivergenceRecord, RestackState, State};
use rung_git::{RemoteDivergence, Repository};
use serde::Serialize;

use crate::commands::utils;
use crate::output;

/// JSON output for restack command.
#[derive(Debug, Serialize)]
struct RestackOutput {
    status: RestackStatus,
    branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_parent: Option<String>,
    new_parent: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    branches_rebased: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    diverged_branches: Vec<DivergenceInfo>,
}

#[derive(Debug, Clone, Serialize)]
struct DivergenceInfo {
    branch: String,
    ahead: usize,
    behind: usize,
}

impl From<&DivergenceRecord> for DivergenceInfo {
    fn from(record: &DivergenceRecord) -> Self {
        Self {
            branch: record.branch.clone(),
            ahead: record.ahead,
            behind: record.behind,
        }
    }
}

impl From<&DivergenceInfo> for DivergenceRecord {
    fn from(info: &DivergenceInfo) -> Self {
        Self {
            branch: info.branch.clone(),
            ahead: info.ahead,
            behind: info.behind,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum RestackStatus {
    Complete,
    DryRun,
    Aborted,
    AlreadyBased,
    Diverged,
}

/// Options for the restack command.
#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)] // CLI options map directly to flags
pub struct RestackOptions<'a> {
    pub json: bool,
    pub branch: Option<&'a str>,
    pub onto: Option<&'a str>,
    pub dry_run: bool,
    pub continue_: bool,
    pub abort: bool,
    pub include_children: bool,
    pub force: bool,
}

/// Run the restack command.
#[allow(clippy::too_many_lines)]
pub fn run(opts: &RestackOptions<'_>) -> Result<()> {
    let (repo, state) = utils::open_repo_and_state()?;

    // Check for conflicting flags
    if opts.continue_ && opts.abort {
        bail!("Cannot use --continue and --abort together");
    }

    // Handle abort
    if opts.abort {
        return handle_abort(&repo, &state, opts.json);
    }

    // Handle continue
    if opts.continue_ {
        return handle_continue(&repo, &state, opts.json);
    }

    // Check for existing restack in progress
    if state.is_restack_in_progress() {
        bail!("Restack already in progress - use --continue to resume or --abort to cancel");
    }

    utils::ensure_on_branch(&repo)?;

    // Determine branch to restack
    let current = repo.current_branch()?;
    let target_branch = opts.branch.unwrap_or(&current);

    // Load stack
    let stack = state.load_stack()?;

    // Verify target branch is in the stack
    let branch_entry = stack
        .find_branch(target_branch)
        .ok_or_else(|| anyhow::anyhow!("Branch '{target_branch}' is not in the stack"))?;
    let old_parent = branch_entry
        .parent
        .as_ref()
        .map(std::string::ToString::to_string);

    // Determine new parent
    let new_parent = match opts.onto {
        Some(parent) => parent.to_string(),
        None => select_new_parent(&stack, target_branch, opts.json)?,
    };

    // Validate new parent exists (either in stack or is a valid branch)
    let new_parent_in_stack = stack.find_branch(&new_parent).is_some();
    if !new_parent_in_stack && !repo.branch_exists(&new_parent) {
        bail!("Branch '{new_parent}' does not exist");
    }

    // Check for cycle
    if stack.would_create_cycle(target_branch, &new_parent) {
        bail!("Cannot restack '{target_branch}' onto '{new_parent}': would create a cycle");
    }

    // Check if it's a no-op (already has this parent)
    if old_parent.as_deref() == Some(&new_parent) {
        if opts.json {
            let output = RestackOutput {
                status: RestackStatus::AlreadyBased,
                branch: target_branch.to_string(),
                old_parent,
                new_parent,
                branches_rebased: vec![],
                diverged_branches: vec![],
            };
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            output::info(&format!(
                "'{target_branch}' is already a child of '{}'",
                old_parent.as_deref().unwrap_or("(base)")
            ));
        }
        return Ok(());
    }

    // === Merge-base analysis: Check if rebase is actually needed ===
    let target_commit = repo.branch_commit(target_branch)?;
    let new_parent_commit = repo.branch_commit(&new_parent)?;
    let merge_base = repo.merge_base(target_commit, new_parent_commit)?;

    let needs_rebase = merge_base != new_parent_commit;

    if !needs_rebase {
        // Branch is already based on new parent's tip, just update the stack
        if !opts.dry_run {
            let mut stack = state.load_stack()?;
            stack.reparent(target_branch, Some(&new_parent))?;
            state.save_stack(&stack)?;
        }

        if opts.json {
            let output = RestackOutput {
                status: if opts.dry_run {
                    RestackStatus::DryRun
                } else {
                    RestackStatus::Complete
                },
                branch: target_branch.to_string(),
                old_parent,
                new_parent,
                branches_rebased: vec![],
                diverged_branches: vec![],
            };
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else if opts.dry_run {
            output::info("Dry run - no changes made");
            output::detail(&format!(
                "'{target_branch}' is already based on '{new_parent}' - only stack topology would be updated"
            ));
        } else {
            output::success(&format!(
                "Updated stack: '{target_branch}' now has parent '{new_parent}'"
            ));
            output::detail("No rebase needed - branch was already based on new parent");
        }
        return Ok(());
    }

    // Dry run output
    if opts.dry_run {
        if opts.json {
            let output = RestackOutput {
                status: RestackStatus::DryRun,
                branch: target_branch.to_string(),
                old_parent,
                new_parent,
                branches_rebased: vec![target_branch.to_string()],
                diverged_branches: vec![],
            };
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            output::info("Dry run - no changes made");
            output::detail(&format!(
                "Would restack '{}' from '{}' onto '{}'",
                target_branch,
                old_parent.as_deref().unwrap_or("(base)"),
                new_parent
            ));
        }
        return Ok(());
    }

    // Ensure working directory is clean
    repo.require_clean()?;

    // === Build list of branches to rebase ===
    let mut branches_to_rebase = vec![target_branch.to_string()];
    if opts.include_children {
        // Add all descendants in topological order (children first, then grandchildren, etc.)
        let descendants = stack.descendants(target_branch);
        for desc in &descendants {
            branches_to_rebase.push(desc.name.to_string());
        }
    }

    // === Check for divergence from remote ===
    let diverged = check_divergence(&repo, &branches_to_rebase);
    if !diverged.is_empty() && !opts.force {
        if opts.json {
            let output = RestackOutput {
                status: RestackStatus::Diverged,
                branch: target_branch.to_string(),
                old_parent,
                new_parent,
                branches_rebased: vec![],
                diverged_branches: diverged,
            };
            println!("{}", serde_json::to_string_pretty(&output)?);
            // Return error to signal non-zero exit
            return Err(anyhow::anyhow!("divergence_detected").context(""));
        }
        for info in &diverged {
            output::warn(&format!(
                "{} has diverged from remote ({} ahead, {} behind)",
                info.branch, info.ahead, info.behind
            ));
        }
        output::detail("  Use --force to proceed anyway");
        output::detail("  (rebased branches will need force-push to update remote)");
        bail!("Restack aborted: branches have diverged from remote");
    }

    // === Create backup of all affected branches ===
    let mut backup_commits: Vec<String> = Vec::with_capacity(branches_to_rebase.len());
    for branch_name in &branches_to_rebase {
        let commit = repo.branch_commit(branch_name)?;
        backup_commits.push(commit.to_string());
    }
    let backup_refs: Vec<(&str, &str)> = branches_to_rebase
        .iter()
        .zip(backup_commits.iter())
        .map(|(name, sha)| (name.as_str(), sha.as_str()))
        .collect();
    let backup_id = state.create_backup(&backup_refs)?;

    // === Create restack state for interruption recovery ===
    let diverged_records: Vec<DivergenceRecord> =
        diverged.iter().map(DivergenceRecord::from).collect();
    let restack_state = RestackState::new(
        backup_id,
        target_branch.to_string(),
        new_parent.clone(),
        old_parent,
        current.clone(),
        branches_to_rebase.clone(),
        diverged_records,
    );
    state.save_restack_state(&restack_state)?;

    if !opts.json {
        if opts.include_children && branches_to_rebase.len() > 1 {
            output::info(&format!(
                "Restacking '{target_branch}' and {} descendant(s) onto '{new_parent}'...",
                branches_to_rebase.len() - 1
            ));
        } else {
            output::info(&format!(
                "Restacking '{target_branch}' onto '{new_parent}'..."
            ));
        }
    }

    // Execute rebase
    execute_restack(&repo, &state, opts.json, &current)
}

/// Handle --abort flag
fn handle_abort(repo: &Repository, state: &State, json: bool) -> Result<()> {
    if !state.is_restack_in_progress() {
        bail!("No restack in progress to abort");
    }

    let restack_state = state.load_restack_state()?;

    // Abort any in-progress rebase
    if repo.is_rebasing() {
        let _ = repo.rebase_abort();
    }

    // Restore all branches from backup
    let refs = state.load_backup(&restack_state.backup_id)?;
    for (branch_name, sha) in refs {
        let oid = rung_git::Oid::from_str(&sha)
            .map_err(|e| anyhow::anyhow!("Invalid backup ref for {branch_name}: {e}"))?;
        repo.reset_branch(&branch_name, oid)?;
    }

    // Restore original branch
    let _ = repo.checkout(&restack_state.original_branch);

    // Clear restack state
    state.clear_restack_state()?;

    if json {
        let output = RestackOutput {
            status: RestackStatus::Aborted,
            branch: restack_state.target_branch,
            old_parent: restack_state.old_parent,
            new_parent: restack_state.new_parent,
            branches_rebased: vec![],
            diverged_branches: vec![],
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        output::success("Restack aborted - branches restored from backup");
    }

    Ok(())
}

/// Handle --continue flag
fn handle_continue(repo: &Repository, state: &State, json: bool) -> Result<()> {
    if !state.is_restack_in_progress() {
        bail!("No restack in progress to continue");
    }

    let mut restack_state = state.load_restack_state()?;

    // Detect stale state from crashed process
    if !repo.is_rebasing() && !restack_state.current_branch.is_empty() {
        bail!(
            "Restack state exists but no rebase in progress (process may have crashed).\n\
             Run `rung restack --abort` to clean up and restore branches."
        );
    }

    if !json {
        output::info("Continuing restack...");
    }

    // Continue the in-progress rebase
    match repo.rebase_continue() {
        Ok(()) => {
            // Success - advance state and continue processing
            restack_state.advance();
            state.save_restack_state(&restack_state)?;
            execute_restack(repo, state, json, &restack_state.original_branch)
        }
        Err(rung_git::Error::RebaseConflict(files)) => {
            output_conflict(&files, json)?;
            bail!("Rebase conflict - resolve and run `rung restack --continue`");
        }
        Err(e) => Err(e.into()),
    }
}

/// Execute the restack operation (initial or continued)
fn execute_restack(
    repo: &Repository,
    state: &State,
    json: bool,
    original_branch: &str,
) -> Result<()> {
    // Load stack once for parent lookups
    let stack = state.load_stack()?;

    loop {
        let mut restack_state = state.load_restack_state()?;

        // Check if complete
        if restack_state.is_complete() {
            return finalize_restack(repo, state, json, original_branch, restack_state);
        }

        // Process current branch
        let current_branch = restack_state.current_branch.clone();
        if current_branch.is_empty() {
            // Shouldn't happen, but handle gracefully
            return finalize_restack(repo, state, json, original_branch, restack_state);
        }

        // Checkout the branch
        repo.checkout(&current_branch)?;

        // Determine the rebase target:
        // - For the target branch: rebase onto new_parent
        // - For child branches: rebase onto their stack parent (which was already rebased)
        let rebase_onto = if current_branch == restack_state.target_branch {
            restack_state.new_parent.clone()
        } else {
            // Child branch - find its parent in the stack
            stack
                .find_branch(&current_branch)
                .and_then(|b| b.parent.as_ref().map(std::string::ToString::to_string))
                .unwrap_or_else(|| restack_state.target_branch.clone())
        };

        // Get the parent's current commit (after it was rebased)
        let parent_commit = repo.branch_commit(&rebase_onto)?;

        // Rebase onto the parent
        match repo.rebase_onto(parent_commit) {
            Ok(()) => {
                restack_state.advance();
                state.save_restack_state(&restack_state)?;
                // Continue loop to process next branch
            }
            Err(rung_git::Error::RebaseConflict(files)) => {
                state.save_restack_state(&restack_state)?;
                output_conflict(&files, json)?;
                bail!("Rebase conflict - resolve and run `rung restack --continue`");
            }
            Err(e) => {
                restore_from_backup(repo, state, &restack_state, original_branch);
                return Err(e.into());
            }
        }
    }
}

/// Finalize a completed restack operation
fn finalize_restack(
    repo: &Repository,
    state: &State,
    json: bool,
    original_branch: &str,
    mut restack_state: RestackState,
) -> Result<()> {
    // Update stack topology (only the target branch's parent changes)
    if !restack_state.stack_updated {
        let mut stack = state.load_stack()?;
        stack.reparent(
            &restack_state.target_branch,
            Some(&restack_state.new_parent),
        )?;
        state.save_stack(&stack)?;
        restack_state.mark_stack_updated();
        state.save_restack_state(&restack_state)?;
    }

    // Clear restack state
    state.clear_restack_state()?;

    // Restore original branch
    if original_branch != restack_state.target_branch {
        let _ = repo.checkout(original_branch);
    }

    // Output results
    if json {
        let diverged_info: Vec<DivergenceInfo> = restack_state
            .diverged_branches
            .iter()
            .map(DivergenceInfo::from)
            .collect();
        let output = RestackOutput {
            status: RestackStatus::Complete,
            branch: restack_state.target_branch.clone(),
            old_parent: restack_state.old_parent.clone(),
            new_parent: restack_state.new_parent.clone(),
            branches_rebased: restack_state.completed.clone(),
            diverged_branches: diverged_info,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        if restack_state.completed.len() > 1 {
            output::success(&format!(
                "Restacked '{}' and {} descendant(s) onto '{}'",
                restack_state.target_branch,
                restack_state.completed.len() - 1,
                restack_state.new_parent
            ));
        } else {
            output::success(&format!(
                "Restacked '{}' onto '{}'",
                restack_state.target_branch, restack_state.new_parent
            ));
        }
        output::detail(&format!("Backup saved: {}", restack_state.backup_id));
    }

    Ok(())
}

/// Restore branches from backup after a failure
fn restore_from_backup(
    repo: &Repository,
    state: &State,
    restack_state: &RestackState,
    original_branch: &str,
) {
    let _ = repo.rebase_abort();
    if let Ok(refs) = state.load_backup(&restack_state.backup_id) {
        for (branch_name, sha) in refs {
            if let Ok(oid) = rung_git::Oid::from_str(&sha) {
                let _ = repo.reset_branch(&branch_name, oid);
            }
        }
    }
    let _ = repo.checkout(original_branch);
    let _ = state.clear_restack_state();
}

/// Output conflict information
fn output_conflict(files: &[String], json: bool) -> Result<()> {
    if json {
        let output = serde_json::json!({
            "status": "conflict",
            "conflict_files": files
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        output::error("Rebase conflict detected");
        output::detail("Resolve conflicts, then run:");
        output::detail("  git add <resolved-files>");
        output::detail("  rung restack --continue");
        output::detail("");
        output::detail("Or abort and restore with:");
        output::detail("  rung restack --abort");
        output::hr();
        output::detail("Conflicting files:");
        for file in files {
            output::detail(&format!("  {file}"));
        }
    }
    Ok(())
}

/// Interactive parent selection.
fn select_new_parent(stack: &rung_core::Stack, target_branch: &str, json: bool) -> Result<String> {
    if json {
        bail!("--onto is required when using --json");
    }

    // Build list of valid parent options
    // Exclude: the target branch itself, and any descendants of target
    let descendants: Vec<_> = stack
        .descendants(target_branch)
        .iter()
        .map(|b| b.name.to_string())
        .collect();

    let options: Vec<String> = stack
        .branches
        .iter()
        .filter(|b| b.name != target_branch && !descendants.contains(&b.name.to_string()))
        .map(|b| {
            let pr = b.pr.map(|n| format!(" #{n}")).unwrap_or_default();
            format!("{}{}", b.name, pr)
        })
        .collect();

    if options.is_empty() {
        bail!("No valid parent branches available in the stack");
    }

    let selection = Select::new("Select new parent:", options)
        .with_page_size(10)
        .prompt()
        .context("Selection cancelled")?;

    // Extract branch name (everything before first space)
    selection
        .split_whitespace()
        .next()
        .map(String::from)
        .context("Invalid selection")
}

/// Check if any branches have diverged from their remote tracking branches.
///
/// Returns a list of branches that have diverged (both local and remote have unique commits).
/// Branches without a remote tracking branch are skipped (not an error).
fn check_divergence(repo: &Repository, branches: &[String]) -> Vec<DivergenceInfo> {
    let mut diverged = Vec::new();

    for branch in branches {
        // Only block on Diverged status (both local and remote have unique commits).
        // Behind: user can pull after restack
        // NoRemote: first push, not an error
        // InSync/Ahead: fine
        // Errors: don't block on remote check failures
        if let Ok(RemoteDivergence::Diverged { ahead, behind }) = repo.remote_divergence(branch) {
            diverged.push(DivergenceInfo {
                branch: branch.clone(),
                ahead,
                behind,
            });
        }
    }

    diverged
}
