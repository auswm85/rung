//! `rung split` command - Split a branch into multiple stacked branches.

use anyhow::{Context, Result, bail};
use inquire::{MultiSelect, Text};
use rung_core::{SplitPoint, State};
use rung_git::Repository;

use crate::commands::utils;
use crate::output;
use crate::services::SplitService;
use crate::services::split::{SplitAnalysis, SplitConfig};

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

    // Phase 3: Interactive commit selection UI
    let split_config = select_split_points(&analysis, branch_name)?;

    if split_config.split_points.is_empty() {
        output::info("No split points selected - nothing to do");
        return Ok(());
    }

    output::info(&format!(
        "Will create {} new branch(es) from '{}'",
        split_config.split_points.len(),
        branch_name
    ));

    for point in &split_config.split_points {
        output::detail(&format!(
            "  {} → branch '{}'",
            &point.commit_sha[..8.min(point.commit_sha.len())],
            point.branch_name
        ));
    }

    // TODO: Phase 4 - Split execution engine
    output::warn("Split execution not yet implemented");

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

/// Interactive UI for selecting split points.
fn select_split_points(analysis: &SplitAnalysis, source_branch: &str) -> Result<SplitConfig> {
    // Build display options for each commit
    let options: Vec<String> = analysis
        .commits
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let position = format!("[{}/{}]", i + 1, analysis.commits.len());
            format!("{position} {} {}", c.short_sha, c.summary)
        })
        .collect();

    output::info("Select commits to split after (each becomes a new branch):");
    output::detail("Use SPACE to select, ENTER to confirm, ESC to cancel");

    // Let user select split points
    let selections = MultiSelect::new("Split after commits:", options.clone())
        .with_page_size(15)
        .prompt()
        .context("Selection cancelled")?;

    if selections.is_empty() {
        return Ok(SplitConfig {
            source_branch: source_branch.to_string(),
            parent_branch: analysis.parent_branch.clone(),
            split_points: vec![],
        });
    }

    // Convert selections to indices (sorted to maintain commit order)
    let mut selected_indices: Vec<usize> = selections
        .iter()
        .filter_map(|s| options.iter().position(|o| o == s))
        .collect();
    selected_indices.sort_unstable();

    // For each selected commit, ask for the new branch name
    let mut split_points = Vec::new();

    for (point_idx, &commit_idx) in selected_indices.iter().enumerate() {
        let commit = &analysis.commits[commit_idx];
        let default_name =
            SplitService::suggest_branch_name(&commit.summary, source_branch, point_idx);

        let name = Text::new(&format!(
            "Branch name for commits up to {} '{}':",
            commit.short_sha,
            truncate(&commit.summary, 40)
        ))
        .with_default(&default_name)
        .with_help_message("Press ENTER to accept default, or type a new name")
        .prompt()
        .context("Branch name input cancelled")?;

        split_points.push(SplitPoint {
            commit_sha: commit.oid.clone(),
            message: commit.summary.clone(),
            branch_name: name,
        });
    }

    Ok(SplitConfig {
        source_branch: source_branch.to_string(),
        parent_branch: analysis.parent_branch.clone(),
        split_points,
    })
}

/// Truncate a string to a maximum length, adding "..." if truncated.
fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}
