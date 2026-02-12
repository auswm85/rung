//! `rung fold` command - Combine adjacent branches into one.

use anyhow::{Context, Result, bail};
use inquire::{Confirm, MultiSelect};
use rung_core::State;
use rung_git::Repository;
use serde::Serialize;

use crate::commands::utils;
use crate::output;
use crate::services::fold::{FoldConfig, FoldService};

/// JSON output for fold operation.
#[derive(Serialize)]
struct FoldJsonOutput {
    success: bool,
    target_branch: String,
    branches_folded: Vec<String>,
    total_commits: usize,
    prs_to_close: Vec<u64>,
}

/// JSON output for dry-run.
#[derive(Serialize)]
struct FoldDryRunOutput {
    dry_run: bool,
    target_branch: String,
    branches_to_fold: Vec<String>,
}

/// JSON output for abort.
#[derive(Serialize)]
struct FoldAbortOutput {
    aborted: bool,
    message: String,
}

/// Options for the fold command.
#[allow(clippy::struct_excessive_bools)]
pub struct FoldOptions<'a> {
    /// Show what would be done without making changes.
    pub dry_run: bool,
    /// Abort the current fold and restore from backup.
    pub abort: bool,
    /// Output as JSON.
    pub json: bool,
    /// Fold current branch into its parent (upward fold).
    pub into_parent: bool,
    /// Fold children into current branch (downward fold).
    pub include_children: bool,
    /// Branches to fold (must be adjacent).
    pub branches: Vec<&'a str>,
}

/// Run the fold command.
pub fn run(opts: &FoldOptions<'_>) -> Result<()> {
    let repo = Repository::open_current().context("Not inside a git repository")?;
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    if !state.is_initialized() {
        bail!("Rung not initialized - run `rung init` first");
    }

    let service = FoldService::new(&repo);

    // Handle --abort
    if opts.abort {
        return handle_abort(&service, &state, opts.json);
    }

    check_in_progress_operations(&state)?;
    utils::ensure_on_branch(&repo)?;

    let current_branch = repo.current_branch()?;
    let analysis = service.analyze(&state, &current_branch)?;

    let fold_config = resolve_fold_config(opts, &state, &analysis, &current_branch)?;

    let Some(config) = fold_config else {
        if opts.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "success": true,
                    "message": "No branches selected for folding"
                }))?
            );
        } else {
            output::info("No branches selected for folding");
        }
        return Ok(());
    };

    if opts.dry_run {
        return handle_dry_run(&config, opts.json);
    }

    if !opts.json && !confirm_fold(&config)? {
        return Ok(());
    }

    let result = service.execute(&state, &config)?;
    print_fold_result(&result, opts.json)
}

/// Check for in-progress operations that would block fold.
fn check_in_progress_operations(state: &State) -> Result<()> {
    if state.is_fold_in_progress() {
        bail!("A fold is already in progress.\nUse --abort to cancel.");
    }
    if state.is_sync_in_progress() {
        bail!("A sync is in progress. Complete or abort it first.");
    }
    if state.is_restack_in_progress() {
        bail!("A restack is in progress. Complete or abort it first.");
    }
    if state.is_split_in_progress() {
        bail!("A split is in progress. Complete or abort it first.");
    }
    Ok(())
}

/// Resolve fold configuration based on options.
fn resolve_fold_config(
    opts: &FoldOptions<'_>,
    state: &State,
    analysis: &crate::services::fold::FoldAnalysis,
    current_branch: &str,
) -> Result<Option<FoldConfig>> {
    if opts.into_parent {
        create_into_parent_config(state, analysis, current_branch)
    } else if opts.include_children {
        create_include_children_config(state, analysis, current_branch)
    } else if !opts.branches.is_empty() {
        create_specified_branches_config(state, &opts.branches)
    } else {
        interactive_fold_selection(state, analysis, current_branch)
    }
}

/// Handle dry-run output.
fn handle_dry_run(config: &FoldConfig, json: bool) -> Result<()> {
    if json {
        let output = FoldDryRunOutput {
            dry_run: true,
            target_branch: config.target_branch.clone(),
            branches_to_fold: config.branches_to_fold.clone(),
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        output::info(&format!(
            "Would fold {} branch(es) into '{}'",
            config.branches_to_fold.len(),
            config.target_branch
        ));
        output::detail("Branches to fold:");
        for branch in &config.branches_to_fold {
            output::detail(&format!("  {branch}"));
        }
    }
    Ok(())
}

/// Confirm fold operation with user.
fn confirm_fold(config: &FoldConfig) -> Result<bool> {
    let branches_str = config.branches_to_fold.join(", ");
    output::info(&format!(
        "Will fold [{}] into '{}'",
        branches_str, config.target_branch
    ));

    let confirmed = Confirm::new("Proceed with fold?")
        .with_default(true)
        .prompt()
        .context("Confirmation cancelled")?;

    if !confirmed {
        output::info("Fold cancelled");
    }
    Ok(confirmed)
}

/// Print fold result.
fn print_fold_result(result: &crate::services::fold::FoldResult, json: bool) -> Result<()> {
    if json {
        let output = FoldJsonOutput {
            success: true,
            target_branch: result.target_branch.clone(),
            branches_folded: result.branches_folded.clone(),
            total_commits: result.total_commits,
            prs_to_close: result.prs_to_close.clone(),
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        output::success(&format!(
            "Folded {} branch(es) into '{}' ({} commits)",
            result.branches_folded.len(),
            result.target_branch,
            result.total_commits
        ));

        for branch in &result.branches_folded {
            output::detail(&format!("  • removed {branch}"));
        }

        if !result.prs_to_close.is_empty() {
            output::info(&format!(
                "PRs to close: {}",
                result
                    .prs_to_close
                    .iter()
                    .map(|pr| format!("#{pr}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            output::detail("Run `rung submit` to update PRs");
        }
    }
    Ok(())
}

/// Handle --abort flag.
fn handle_abort(service: &FoldService<'_>, state: &State, json: bool) -> Result<()> {
    service.abort(state)?;
    if json {
        let output = FoldAbortOutput {
            aborted: true,
            message: "Fold aborted - branches restored from backup".to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        output::success("Fold aborted - branches restored from backup");
    }
    Ok(())
}

/// Create config for folding current branch into its parent.
fn create_into_parent_config(
    state: &State,
    analysis: &crate::services::fold::FoldAnalysis,
    current_branch: &str,
) -> Result<Option<FoldConfig>> {
    let Some(parent) = &analysis.parent_branch else {
        bail!("Current branch has no parent in the stack");
    };

    // The parent is the target, current branch gets folded into it
    // CURRENT is what gets folded/removed into PARENT

    // Get the actual grandparent (parent's parent)
    let stack = state.load_stack()?;
    let parent_branch = stack
        .find_branch(parent)
        .ok_or_else(|| anyhow::anyhow!("Parent branch not found in stack"))?;

    let default_branch = state
        .default_branch()
        .unwrap_or_else(|_| "main".to_string());
    let new_parent = parent_branch
        .parent
        .as_ref()
        .map_or(default_branch, ToString::to_string);

    Ok(Some(FoldConfig {
        target_branch: parent.clone(),
        branches_to_fold: vec![current_branch.to_string()],
        new_parent,
    }))
}

/// Create config for folding children into current branch.
fn create_include_children_config(
    state: &State,
    analysis: &crate::services::fold::FoldAnalysis,
    current_branch: &str,
) -> Result<Option<FoldConfig>> {
    if analysis.children.is_empty() {
        bail!("Current branch has no children to fold");
    }

    // Collect all children in order
    let branches_to_fold: Vec<String> = analysis.children.iter().map(|c| c.name.clone()).collect();

    // The new parent stays the same (current branch's parent)
    let default_branch = state
        .default_branch()
        .unwrap_or_else(|_| "main".to_string());
    let new_parent = analysis.parent_branch.clone().unwrap_or(default_branch);

    Ok(Some(FoldConfig {
        target_branch: current_branch.to_string(),
        branches_to_fold,
        new_parent,
    }))
}

/// Create config for folding specified branches.
fn create_specified_branches_config(
    state: &State,
    branches: &[&str],
) -> Result<Option<FoldConfig>> {
    if branches.len() < 2 {
        bail!("At least two branches must be specified for folding");
    }

    let stack = state.load_stack()?;

    // Verify all branches exist and are adjacent (form a parent-child chain)
    let mut ordered_branches = Vec::new();

    // Find the root of the chain (branch whose parent is not in the list)
    let mut root = None;
    for &branch in branches {
        let stack_branch = stack
            .find_branch(branch)
            .ok_or_else(|| anyhow::anyhow!("Branch '{branch}' not found in stack"))?;

        let parent_in_list = stack_branch
            .parent
            .as_ref()
            .is_some_and(|p| branches.contains(&p.as_str()));

        if !parent_in_list {
            if root.is_some() {
                bail!("Branches must form a single parent-child chain");
            }
            root = Some(branch);
        }
    }

    let root = root.ok_or_else(|| anyhow::anyhow!("Could not determine root of branch chain"))?;

    // Build the ordered chain
    let mut current = root.to_string();
    while ordered_branches.len() < branches.len() {
        ordered_branches.push(current.clone());
        // Find child in the list
        let children: Vec<String> = stack
            .children_of(&current)
            .iter()
            .filter(|c| branches.contains(&c.name.as_str()))
            .map(|c| c.name.to_string())
            .collect();

        if children.len() > 1 {
            bail!("Branches must form a linear chain (no branching)");
        }

        if let Some(child) = children.first() {
            current.clone_from(child);
        } else {
            break;
        }
    }

    if ordered_branches.len() != branches.len() {
        bail!("Branches must form a connected chain");
    }

    // The first branch is the target, rest are folded
    let target_branch = ordered_branches.remove(0);
    let target_stack_branch = stack
        .find_branch(&target_branch)
        .ok_or_else(|| anyhow::anyhow!("Target branch '{target_branch}' not found"))?;

    let new_parent = target_stack_branch.parent.as_ref().map_or_else(
        || {
            state
                .default_branch()
                .unwrap_or_else(|_| "main".to_string())
        },
        std::string::ToString::to_string,
    );

    Ok(Some(FoldConfig {
        target_branch,
        branches_to_fold: ordered_branches,
        new_parent,
    }))
}

/// Interactive fold selection.
fn interactive_fold_selection(
    state: &State,
    analysis: &crate::services::fold::FoldAnalysis,
    current_branch: &str,
) -> Result<Option<FoldConfig>> {
    // Build options
    let mut options = Vec::new();

    // Option to fold into parent
    if analysis.parent_branch.is_some() {
        options.push(format!(
            "Fold into parent (merge {current_branch} into parent)"
        ));
    }

    // Option to fold children
    if !analysis.children.is_empty() {
        let child_names: Vec<_> = analysis.children.iter().map(|c| c.name.as_str()).collect();
        options.push(format!(
            "Fold children ({}) into {}",
            child_names.join(", "),
            current_branch
        ));
    }

    if options.is_empty() {
        bail!("No branches available to fold (need parent or children in stack)");
    }

    // Add cancel option
    options.push("Cancel".to_string());

    output::info("Select fold operation:");

    let selection = inquire::Select::new("Fold operation:", options.clone())
        .prompt()
        .context("Selection cancelled")?;

    if selection == "Cancel" {
        return Ok(None);
    }

    if selection.starts_with("Fold into parent") {
        create_into_parent_config(state, analysis, current_branch)
    } else if selection.starts_with("Fold children") {
        // If multiple children, let user select which ones
        if analysis.children.len() > 1 {
            let child_options: Vec<String> = analysis
                .children
                .iter()
                .map(|c| format!("{} ({} commits)", c.name, c.commit_count))
                .collect();

            let selections = MultiSelect::new("Select children to fold:", child_options.clone())
                .with_all_selected_by_default()
                .prompt()
                .context("Selection cancelled")?;

            if selections.is_empty() {
                return Ok(None);
            }

            // Get selected branch names
            let selected_indices: Vec<usize> = selections
                .iter()
                .filter_map(|s| child_options.iter().position(|o| o == s))
                .collect();

            let branches_to_fold: Vec<String> = selected_indices
                .iter()
                .map(|&i| analysis.children[i].name.clone())
                .collect();

            let default_branch = state
                .default_branch()
                .unwrap_or_else(|_| "main".to_string());
            let new_parent = analysis.parent_branch.clone().unwrap_or(default_branch);

            Ok(Some(FoldConfig {
                target_branch: current_branch.to_string(),
                branches_to_fold,
                new_parent,
            }))
        } else {
            create_include_children_config(state, analysis, current_branch)
        }
    } else {
        Ok(None)
    }
}
