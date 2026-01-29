//! `rung restack` command - Move a branch to a different parent.
//!
//! This command reparents a branch by rebasing it onto a new parent branch,
//! updating the stack topology accordingly. Supports interruption recovery
//! via `--continue` and `--abort` flags.

use anyhow::{Context, Result, bail};
use inquire::Select;
use rung_core::{DivergenceRecord, State};
use serde::Serialize;

use crate::commands::utils;
use crate::output;
use crate::services::{DivergenceInfo, RestackConfig, RestackService};

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
    diverged_branches: Vec<DivergenceInfoOutput>,
}

#[derive(Debug, Clone, Serialize)]
struct DivergenceInfoOutput {
    branch: String,
    ahead: usize,
    behind: usize,
}

impl From<&DivergenceRecord> for DivergenceInfoOutput {
    fn from(record: &DivergenceRecord) -> Self {
        Self {
            branch: record.branch.clone(),
            ahead: record.ahead,
            behind: record.behind,
        }
    }
}

impl From<&DivergenceInfo> for DivergenceInfoOutput {
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
pub fn run(opts: &RestackOptions<'_>) -> Result<()> {
    let (repo, state) = utils::open_repo_and_state()?;
    let service = RestackService::new(&repo);

    // Check for conflicting flags
    if opts.continue_ && opts.abort {
        bail!("Cannot use --continue and --abort together");
    }

    // Handle abort
    if opts.abort {
        return handle_abort(&service, &state, opts.json);
    }

    // Handle continue
    if opts.continue_ {
        return handle_continue(&service, &state, opts.json);
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

    // Determine new parent
    let new_parent = match opts.onto {
        Some(parent) => parent.to_string(),
        None => select_new_parent(&stack, target_branch, opts.json)?,
    };

    // Create config for the service
    let config = RestackConfig {
        target_branch: target_branch.to_string(),
        new_parent: new_parent.clone(),
        include_children: opts.include_children,
    };

    // Create plan
    let plan = service.create_plan(&state, &config)?;

    // Handle special cases that don't require a full rebase
    if let Some(result) = handle_early_exit(opts, &state, target_branch, &new_parent, &plan) {
        return result;
    }

    // Ensure working directory is clean
    repo.require_clean()?;

    // Check for divergence
    check_divergence(opts, &plan, target_branch, &new_parent)?;

    print_restack_start(opts, target_branch, &new_parent, &plan);

    // Execute restack
    let _restack_state = service.execute(&state, &plan, &current)?;
    let result = service.execute_restack_loop(&state, &current);

    handle_restack_result(result, opts.json)
}

/// Handle early exit cases: already-based, no-rebase-needed, dry-run.
fn handle_early_exit(
    opts: &RestackOptions<'_>,
    state: &State,
    target_branch: &str,
    new_parent: &str,
    plan: &crate::services::restack::RestackPlan,
) -> Option<Result<()>> {
    // Check if it's a no-op (already has this parent)
    if plan.old_parent.as_deref() == Some(new_parent) && plan.branches_to_rebase.is_empty() {
        return Some(output_already_based(opts, target_branch, plan));
    }

    // Handle no rebase needed (just topology update)
    if !plan.needs_rebase {
        return Some(output_no_rebase_needed(
            opts,
            state,
            target_branch,
            new_parent,
            plan,
        ));
    }

    // Dry run output
    if opts.dry_run {
        return Some(output_dry_run(opts, target_branch, new_parent, plan));
    }

    None
}

/// Output when branch is already based on the target parent.
fn output_already_based(
    opts: &RestackOptions<'_>,
    target_branch: &str,
    plan: &crate::services::restack::RestackPlan,
) -> Result<()> {
    if opts.json {
        let output = RestackOutput {
            status: RestackStatus::AlreadyBased,
            branch: target_branch.to_string(),
            old_parent: plan.old_parent.clone(),
            new_parent: plan.new_parent.clone(),
            branches_rebased: vec![],
            diverged_branches: vec![],
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        output::info(&format!(
            "'{target_branch}' is already a child of '{}'",
            plan.old_parent.as_deref().unwrap_or("(base)")
        ));
    }
    Ok(())
}

/// Output when no rebase is needed (topology-only update).
fn output_no_rebase_needed(
    opts: &RestackOptions<'_>,
    state: &State,
    target_branch: &str,
    new_parent: &str,
    plan: &crate::services::restack::RestackPlan,
) -> Result<()> {
    if !opts.dry_run {
        let mut stack = state.load_stack()?;
        stack.reparent(target_branch, Some(new_parent))?;
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
            old_parent: plan.old_parent.clone(),
            new_parent: new_parent.to_string(),
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
    Ok(())
}

/// Output for dry run mode.
fn output_dry_run(
    opts: &RestackOptions<'_>,
    target_branch: &str,
    new_parent: &str,
    plan: &crate::services::restack::RestackPlan,
) -> Result<()> {
    if opts.json {
        let output = RestackOutput {
            status: RestackStatus::DryRun,
            branch: target_branch.to_string(),
            old_parent: plan.old_parent.clone(),
            new_parent: new_parent.to_string(),
            branches_rebased: plan.branches_to_rebase.clone(),
            diverged_branches: vec![],
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        output::info("Dry run - no changes made");
        output::detail(&format!(
            "Would restack '{}' from '{}' onto '{}'",
            target_branch,
            plan.old_parent.as_deref().unwrap_or("(base)"),
            new_parent
        ));
    }
    Ok(())
}

/// Check for divergence and report if found.
fn check_divergence(
    opts: &RestackOptions<'_>,
    plan: &crate::services::restack::RestackPlan,
    target_branch: &str,
    new_parent: &str,
) -> Result<()> {
    if plan.diverged.is_empty() || opts.force {
        return Ok(());
    }

    if opts.json {
        let diverged_output: Vec<DivergenceInfoOutput> = plan
            .diverged
            .iter()
            .map(DivergenceInfoOutput::from)
            .collect();
        let output = RestackOutput {
            status: RestackStatus::Diverged,
            branch: target_branch.to_string(),
            old_parent: plan.old_parent.clone(),
            new_parent: new_parent.to_string(),
            branches_rebased: vec![],
            diverged_branches: diverged_output,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Err(anyhow::anyhow!("divergence_detected").context(""));
    }

    for info in &plan.diverged {
        output::warn(&format!(
            "{} has diverged from remote ({} ahead, {} behind)",
            info.branch, info.ahead, info.behind
        ));
    }
    output::detail("  Use --force to proceed anyway");
    output::detail("  (rebased branches will need force-push to update remote)");
    bail!("Restack aborted: branches have diverged from remote");
}

/// Print restack start message.
fn print_restack_start(
    opts: &RestackOptions<'_>,
    target_branch: &str,
    new_parent: &str,
    plan: &crate::services::restack::RestackPlan,
) {
    if opts.json {
        return;
    }

    if opts.include_children && plan.branches_to_rebase.len() > 1 {
        output::info(&format!(
            "Restacking '{target_branch}' and {} descendant(s) onto '{new_parent}'...",
            plan.branches_to_rebase.len() - 1
        ));
    } else {
        output::info(&format!(
            "Restacking '{target_branch}' onto '{new_parent}'..."
        ));
    }
}

/// Handle the result of a restack operation.
fn handle_restack_result(
    result: Result<crate::services::restack::RestackResult>,
    json: bool,
) -> Result<()> {
    match result {
        Ok(result) => {
            if json {
                let diverged_output: Vec<DivergenceInfoOutput> = result
                    .diverged_branches
                    .iter()
                    .map(DivergenceInfoOutput::from)
                    .collect();
                let output = RestackOutput {
                    status: RestackStatus::Complete,
                    branch: result.target_branch,
                    old_parent: result.old_parent,
                    new_parent: result.new_parent,
                    branches_rebased: result.branches_rebased,
                    diverged_branches: diverged_output,
                };
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else if result.branches_rebased.len() > 1 {
                output::success(&format!(
                    "Restacked '{}' and {} descendant(s) onto '{}'",
                    result.target_branch,
                    result.branches_rebased.len() - 1,
                    result.new_parent
                ));
            } else {
                output::success(&format!(
                    "Restacked '{}' onto '{}'",
                    result.target_branch, result.new_parent
                ));
            }
            Ok(())
        }
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("Rebase conflict") {
                output_conflict(&[], json)?;
                bail!("Rebase conflict - resolve and run `rung restack --continue`");
            }
            Err(e)
        }
    }
}

/// Handle --abort flag
fn handle_abort<G: rung_git::GitOps>(
    service: &RestackService<'_, G>,
    state: &State,
    json: bool,
) -> Result<()> {
    let result = service.abort(state)?;

    if json {
        let output = RestackOutput {
            status: RestackStatus::Aborted,
            branch: result.target_branch,
            old_parent: result.old_parent,
            new_parent: result.new_parent,
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
fn handle_continue<G: rung_git::GitOps>(
    service: &RestackService<'_, G>,
    state: &State,
    json: bool,
) -> Result<()> {
    if !json {
        output::info("Continuing restack...");
    }

    let result = service.continue_restack(state);

    match result {
        Ok(result) => {
            if json {
                let diverged_output: Vec<DivergenceInfoOutput> = result
                    .diverged_branches
                    .iter()
                    .map(DivergenceInfoOutput::from)
                    .collect();
                let output = RestackOutput {
                    status: RestackStatus::Complete,
                    branch: result.target_branch,
                    old_parent: result.old_parent,
                    new_parent: result.new_parent,
                    branches_rebased: result.branches_rebased,
                    diverged_branches: diverged_output,
                };
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else if result.branches_rebased.len() > 1 {
                output::success(&format!(
                    "Restacked '{}' and {} descendant(s) onto '{}'",
                    result.target_branch,
                    result.branches_rebased.len() - 1,
                    result.new_parent
                ));
            } else {
                output::success(&format!(
                    "Restacked '{}' onto '{}'",
                    result.target_branch, result.new_parent
                ));
            }
            Ok(())
        }
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("Rebase conflict") {
                output_conflict(&[], json)?;
                bail!("Rebase conflict - resolve and run `rung restack --continue`");
            }
            Err(e)
        }
    }
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
        if !files.is_empty() {
            output::hr();
            output::detail("Conflicting files:");
            for file in files {
                output::detail(&format!("  {file}"));
            }
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
