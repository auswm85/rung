//! `rung status` command - Display the current stack status.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use colored::Colorize;
use rung_core::State;
use rung_git::Repository;
use rung_github::{Auth, GitHubClient, PullRequestState};
use serde::Serialize;

use crate::output::{self, PrStatus};
use crate::services::{BranchStatusInfo, RemoteDivergenceInfo, StatusService};

/// Run the status command.
pub fn run(json: bool, fetch: bool) -> Result<()> {
    // Open repository
    let repo = Repository::open_current().context("Not inside a git repository")?;

    // Get state manager
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    // Ensure initialized
    if !state.is_initialized() {
        bail!("Rung not initialized - run `rung init` first");
    }

    // Load stack
    let stack = state.load_stack()?;

    // Create service
    let service = StatusService::new(&repo, &stack);

    // Fetch latest from remote if requested
    if fetch {
        if !json {
            output::info("Fetching from remote...");
        }
        service
            .fetch_remote()
            .context("Failed to fetch from remote")?;
    }

    // Compute status
    let status = service.compute_status()?;

    if status.is_empty() {
        if json {
            println!("{}", serde_json::to_string_pretty(&JsonOutput::empty())?);
        } else {
            output::info("No branches in stack yet. Use `rung create <name>` to add one.");
        }
        return Ok(());
    }

    // Fetch PR statuses if requested (best-effort - don't fail status command on GitHub errors)
    let mut pr_cache = HashMap::new();
    if fetch
        && let Err(e) = fetch_pr_statuses(&repo, &stack, &mut pr_cache, json)
        && !json
    {
        output::warn(&format!("Could not fetch PR statuses: {e}"));
    }

    // Enrich branches with PR status info
    let branches_with_pr_status: Vec<BranchWithPrStatus> = status
        .branches
        .into_iter()
        .map(|branch| {
            let (pr_state, display_status) = branch.pr.map_or((None, None), |pr_num| {
                pr_cache.get(&pr_num).map_or((None, None), |pr| {
                    let status = match (pr.state, pr.draft) {
                        (PullRequestState::Merged, _) => PrStatus::Merged,
                        (PullRequestState::Closed, _) => PrStatus::Closed,
                        (_, true) => PrStatus::Draft,
                        _ => PrStatus::Open,
                    };
                    let pr_state = match status {
                        PrStatus::Open => "open",
                        PrStatus::Draft => "draft",
                        PrStatus::Merged => "merged",
                        PrStatus::Closed => "closed",
                    };
                    (Some(pr_state.to_string()), Some(status))
                })
            });
            BranchWithPrStatus {
                info: branch,
                pr_state,
                display_status,
            }
        })
        .collect();

    // Output
    if json {
        let output = JsonOutput::from_branches(&branches_with_pr_status, status.current_branch);
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print_tree(&branches_with_pr_status);
    }

    Ok(())
}

/// Fetch PR statuses from GitHub (best-effort).
fn fetch_pr_statuses(
    repo: &Repository,
    stack: &rung_core::Stack,
    pr_cache: &mut HashMap<u64, rung_github::PullRequest>,
    json: bool,
) -> Result<()> {
    // Early return if no PRs to fetch
    let pr_numbers: Vec<u64> = stack.branches.iter().filter_map(|b| b.pr).collect();
    if pr_numbers.is_empty() {
        return Ok(());
    }

    let origin_url = repo.origin_url().context("No origin remote configured")?;
    let (owner, repo_name) = Repository::parse_github_remote(&origin_url)
        .context("Could not parse GitHub remote URL")?;

    let client = GitHubClient::new(&Auth::auto()).context("Failed to authenticate with GitHub")?;
    let rt = tokio::runtime::Runtime::new()?;

    if !json {
        let label = if pr_numbers.len() == 1 { "PR" } else { "PRs" };
        output::info(&format!(
            "Fetching status for {} {label}...",
            pr_numbers.len(),
        ));
    }
    *pr_cache = rt.block_on(client.get_prs_batch(&owner, &repo_name, &pr_numbers))?;
    Ok(())
}

/// Print a tree view of the stack.
fn print_tree(branches: &[BranchWithPrStatus]) {
    println!();
    println!("  {}", "Stack".bold());
    output::hr();

    for branch in branches {
        let state_icon = output::state_indicator(&branch.info.state);
        let name = output::branch_name(&branch.info.name, branch.info.is_current);
        let pr = output::pr_ref(branch.info.pr, branch.display_status);

        let parent_info = branch
            .info
            .parent
            .as_ref()
            .map(|p| format!(" ← {}", p.dimmed()))
            .unwrap_or_default();

        // Add remote divergence indicator if present
        let divergence = branch
            .info
            .remote_divergence
            .as_ref()
            .and_then(remote_divergence_indicator)
            .map(|s| format!(" {s}"))
            .unwrap_or_default();

        println!("  {state_icon} {name} {pr}{parent_info}{divergence}");
    }

    output::hr();
    println!();

    // Legend
    println!(
        "  {} synced  {} needs sync  {} conflict",
        "●".green(),
        "●".yellow(),
        "●".red()
    );
    println!();

    // Collect branches that need force push and print warnings
    let diverged: Vec<_> = branches
        .iter()
        .filter(|b| {
            matches!(
                b.info.remote_divergence,
                Some(RemoteDivergenceInfo::Diverged { .. })
            )
        })
        .collect();

    if !diverged.is_empty() {
        for b in &diverged {
            if let Some(RemoteDivergenceInfo::Diverged { ahead, behind }) =
                &b.info.remote_divergence
            {
                output::warn(&format!(
                    "{} has diverged from remote ({} ahead, {} behind)",
                    b.info.name, ahead, behind
                ));
            }
        }
        output::detail("  Run `rung submit --force` to safely update (uses --force-with-lease)");
        println!();
    }
}

/// Format remote divergence info as a compact indicator.
fn remote_divergence_indicator(divergence: &RemoteDivergenceInfo) -> Option<String> {
    match divergence {
        RemoteDivergenceInfo::InSync | RemoteDivergenceInfo::NoRemote => None,
        RemoteDivergenceInfo::Ahead { commits } => {
            Some(format!("({commits}↑)").dimmed().to_string())
        }
        RemoteDivergenceInfo::Behind { commits } => {
            Some(format!("({commits}↓)").yellow().to_string())
        }
        RemoteDivergenceInfo::Diverged { ahead, behind } => {
            Some(format!("({ahead}↑ {behind}↓)").yellow().to_string())
        }
    }
}

/// Branch info with PR status for display.
struct BranchWithPrStatus {
    info: BranchStatusInfo,
    #[allow(dead_code)]
    pr_state: Option<String>,
    display_status: Option<PrStatus>,
}

/// JSON output wrapper (preserves existing JSON structure).
#[derive(Debug, Serialize)]
struct JsonOutput {
    branches: Vec<JsonBranchInfo>,
    current: Option<String>,
}

#[derive(Debug, Serialize)]
struct JsonBranchInfo {
    #[serde(flatten)]
    info: BranchStatusInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pr_state: Option<String>,
}

impl JsonOutput {
    const fn empty() -> Self {
        Self {
            branches: vec![],
            current: None,
        }
    }

    fn from_branches(branches: &[BranchWithPrStatus], current: Option<String>) -> Self {
        Self {
            branches: branches
                .iter()
                .map(|b| JsonBranchInfo {
                    info: b.info.clone(),
                    pr_state: b.pr_state.clone(),
                })
                .collect(),
            current,
        }
    }
}
