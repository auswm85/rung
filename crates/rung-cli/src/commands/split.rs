//! `rung split` command - Split a branch into multiple stacked branches.

use anyhow::{Context, Result, bail};
use rung_core::State;
use rung_git::Repository;

use crate::commands::utils;
use crate::output;
use crate::services::SplitService;

/// Options for the split command.
#[allow(clippy::struct_excessive_bools)]
pub struct SplitOptions<'a> {
    /// Show what would be done without making changes.
    pub dry_run: bool,
    /// Continue a paused split after resolving conflicts.
    pub continue_: bool,
    /// Abort the current split and restore from backup.
    pub abort: bool,
    /// Output as JSON.
    pub json: bool,
    /// Branch to split. Defaults to current branch.
    pub branch: Option<&'a str>,
}

/// Run the split command.
pub fn run(opts: &SplitOptions<'_>) -> Result<()> {
    let repo = Repository::open_current().context("Not inside a git repository")?;
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    if !state.is_initialized() {
        bail!("Rung not initialized - run `rung init` first");
    }

    let service = SplitService::new(&repo);

    // Handle --continue
    if opts.continue_ {
        return handle_continue(&service, &state, opts.json);
    }

    // Handle --abort
    if opts.abort {
        return handle_abort(&service, &state, opts.json);
    }

    // Check for in-progress operations
    if state.is_split_in_progress() {
        bail!(
            "A split is already in progress.\n\
             Use --continue to resume or --abort to cancel."
        );
    }

    if state.is_sync_in_progress() {
        bail!("A sync is in progress. Complete or abort it first.");
    }

    if state.is_restack_in_progress() {
        bail!("A restack is in progress. Complete or abort it first.");
    }

    // Ensure on a branch
    utils::ensure_on_branch(&repo)?;

    // Get the branch to split
    let current_branch = repo.current_branch()?;
    let branch_name = opts.branch.unwrap_or(&current_branch);

    // Analyze the branch
    let analysis = service.analyze(&state, branch_name)?;

    if analysis.commits.is_empty() {
        bail!("No commits to split - branch is already at parent");
    }

    if analysis.commits.len() == 1 {
        bail!("Only one commit on branch - nothing to split");
    }

    if opts.dry_run {
        output::info(&format!(
            "Would split '{}' ({} commits) into multiple branches",
            branch_name,
            analysis.commits.len()
        ));
        output::detail("Commits:");
        for commit in &analysis.commits {
            output::detail(&format!("  {} {}", commit.short_sha, commit.summary));
        }
        return Ok(());
    }

    // TODO: Phase 3 - Interactive commit selection UI
    // TODO: Phase 4 - Split execution engine

    output::warn("Interactive split UI not yet implemented");

    bail!(
        "Split not yet implemented for branch '{}' ({} commits)",
        branch_name,
        analysis.commits.len()
    )
}

/// Handle --continue flag.
fn handle_continue(service: &SplitService<'_>, state: &State, _json: bool) -> Result<()> {
    service.continue_split(state)?;
    Ok(())
}

/// Handle --abort flag.
fn handle_abort(service: &SplitService<'_>, state: &State, _json: bool) -> Result<()> {
    service.abort(state)?;
    output::success("Split aborted - branches restored from backup");
    Ok(())
}
