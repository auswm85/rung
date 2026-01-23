//! `rung status` command - Display the current stack status.

use anyhow::{Context, Result, bail};
use rung_core::{BranchState, State};
use rung_git::{RemoteDivergence, Repository};
use serde::Serialize;

use crate::output;

/// Run the status command.
pub fn run(json: bool, _fetch: bool) -> Result<()> {
    // Open repository
    let repo = Repository::open_current().context("Not inside a git repository")?;

    // Get state manager
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    // Ensure initialized
    if !state.is_initialized() {
        bail!("Rung not initialized - run `rung init` first");
    }

    // Get current branch
    let current = repo.current_branch().ok();

    // Load stack
    let stack = state.load_stack()?;

    if stack.is_empty() {
        if json {
            println!("{}", serde_json::to_string_pretty(&JsonOutput::empty())?);
        } else {
            output::info("No branches in stack yet. Use `rung create <name>` to add one.");
        }
        return Ok(());
    }

    // Compute branch states
    let mut branches_with_state: Vec<BranchInfo> = vec![];

    for branch in &stack.branches {
        let branch_state = compute_branch_state(&repo, branch, &stack)?;
        let remote_divergence = repo
            .remote_divergence(&branch.name)
            .ok()
            .map(|d| RemoteDivergenceInfo::from(&d));
        branches_with_state.push(BranchInfo {
            name: branch.name.to_string(),
            parent: branch.parent.as_ref().map(ToString::to_string),
            state: branch_state,
            pr: branch.pr,
            is_current: current.as_deref() == Some(branch.name.as_str()),
            remote_divergence,
        });
    }

    if json {
        let output = JsonOutput {
            branches: branches_with_state,
            current,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print_tree(&branches_with_state);
    }

    Ok(())
}

/// Compute the sync state of a branch relative to its parent.
fn compute_branch_state(
    repo: &Repository,
    branch: &rung_core::stack::StackBranch,
    stack: &rung_core::Stack,
) -> Result<BranchState> {
    let Some(parent_name) = &branch.parent else {
        // Root branch, always synced
        return Ok(BranchState::Synced);
    };

    // Check if parent exists in repo
    if !repo.branch_exists(parent_name) {
        // Check if parent is in stack (might be deleted)
        if stack.find_branch(parent_name).is_some() {
            return Ok(BranchState::Detached);
        }
        // Parent is external (like main), check if it exists
        if !repo.branch_exists(parent_name) {
            return Ok(BranchState::Detached);
        }
    }

    // Get commits
    let branch_commit = repo.branch_commit(&branch.name)?;
    let parent_commit = repo.branch_commit(parent_name)?;

    // Find merge base
    let merge_base = repo.merge_base(branch_commit, parent_commit)?;

    // If merge base is the parent commit, we're synced
    if merge_base == parent_commit {
        return Ok(BranchState::Synced);
    }

    // Count how many commits behind
    let commits_behind = repo.count_commits_between(merge_base, parent_commit)?;

    Ok(BranchState::Diverged { commits_behind })
}

/// Print a tree view of the stack.
fn print_tree(branches: &[BranchInfo]) {
    println!();
    println!("  {}", "Stack".bold());
    output::hr();

    for branch in branches {
        let state_icon = output::state_indicator(&branch.state);
        let name = output::branch_name(&branch.name, branch.is_current);
        let pr = output::pr_ref(branch.pr);

        let parent_info = branch
            .parent
            .as_ref()
            .map(|p| format!(" ← {}", p.dimmed()))
            .unwrap_or_default();

        // Add remote divergence indicator if present
        let divergence = branch
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
                b.remote_divergence,
                Some(RemoteDivergenceInfo::Diverged { .. })
            )
        })
        .collect();

    if !diverged.is_empty() {
        for b in &diverged {
            if let Some(RemoteDivergenceInfo::Diverged { ahead, behind }) = &b.remote_divergence {
                output::warn(&format!(
                    "{} has diverged from remote ({} ahead, {} behind)",
                    b.name, ahead, behind
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

use colored::Colorize;

#[derive(Debug, Serialize)]
struct JsonOutput {
    branches: Vec<BranchInfo>,
    current: Option<String>,
}

impl JsonOutput {
    const fn empty() -> Self {
        Self {
            branches: vec![],
            current: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct BranchInfo {
    name: String,
    parent: Option<String>,
    state: BranchState,
    pr: Option<u64>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    is_current: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote_divergence: Option<RemoteDivergenceInfo>,
}

/// Serializable remote divergence info for JSON output.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum RemoteDivergenceInfo {
    InSync,
    Ahead { commits: usize },
    Behind { commits: usize },
    Diverged { ahead: usize, behind: usize },
    NoRemote,
}

impl From<&RemoteDivergence> for RemoteDivergenceInfo {
    fn from(d: &RemoteDivergence) -> Self {
        match d {
            RemoteDivergence::InSync => Self::InSync,
            RemoteDivergence::Ahead { commits } => Self::Ahead { commits: *commits },
            RemoteDivergence::Behind { commits } => Self::Behind { commits: *commits },
            RemoteDivergence::Diverged { ahead, behind } => Self::Diverged {
                ahead: *ahead,
                behind: *behind,
            },
            RemoteDivergence::NoRemote => Self::NoRemote,
        }
    }
}
