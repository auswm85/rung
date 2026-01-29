//! `rung submit` command - Push branches and create/update PRs.

use anyhow::{Context, Result, bail};
use rung_core::{State, stack::Stack, sync};
use rung_git::{RemoteDivergence, Repository};
use rung_github::{Auth, GitHubClient};
use serde::Serialize;

use crate::commands::utils;
use crate::output;
use crate::services::{
    BranchSubmitResult, PlannedBranchAction, SubmitAction, SubmitConfig, SubmitPlan, SubmitService,
};

/// JSON output for submit command.
#[derive(Debug, Serialize)]
struct SubmitOutput {
    prs_created: usize,
    prs_updated: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    branches: Vec<BranchOutputInfo>,
    dry_run: bool,
}

/// Information about a submitted branch (for JSON output).
#[derive(Debug, Serialize)]
struct BranchOutputInfo {
    branch: String,
    pr_number: u64,
    pr_url: String,
    action: OutputAction,
}

impl From<BranchSubmitResult> for BranchOutputInfo {
    fn from(result: BranchSubmitResult) -> Self {
        Self {
            branch: result.branch,
            pr_number: result.pr_number,
            pr_url: result.pr_url,
            action: match result.action {
                SubmitAction::Created => OutputAction::Created,
                SubmitAction::Updated => OutputAction::Updated,
            },
        }
    }
}

/// Information about a planned branch action (for dry-run output).
#[derive(Debug, Serialize)]
struct PlannedBranchInfo {
    branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pr_number: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pr_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_base: Option<String>,
    action: OutputAction,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum OutputAction {
    Created,
    Updated,
}

/// Run the submit command.
#[allow(clippy::fn_params_excessive_bools)]
pub fn run(
    json: bool,
    dry_run: bool,
    draft: bool,
    force: bool,
    custom_title: Option<&str>,
) -> Result<()> {
    let (repo, state, mut stack) = setup_submit()?;

    if stack.is_empty() {
        if json {
            if dry_run {
                return output_dry_run_json(&SubmitPlan::empty());
            }
            return output_json(&SubmitOutput {
                prs_created: 0,
                prs_updated: 0,
                branches: vec![],
                dry_run: false,
            });
        }
        output::info("No branches in stack - nothing to submit");
        return Ok(());
    }

    // Ensure on branch
    utils::ensure_on_branch(&repo)?;

    let config = SubmitConfig {
        draft,
        custom_title,
        current_branch: repo.current_branch().ok(),
        default_branch: state
            .default_branch()
            .context("Failed to load default branch from config")?,
    };

    let (owner, repo_name) = get_remote_info(&repo)?;

    let client = GitHubClient::new(&Auth::auto()).context("Failed to authenticate with GitHub")?;
    let rt = tokio::runtime::Runtime::new()?;

    let service = SubmitService::new(&repo, &client, owner.clone(), repo_name.clone());

    // Phase 0: Sync Protection
    if !force {
        validate_sync_state(&repo, &stack, &config.default_branch, json)?;
    }

    // Phase 1: Create the plan (read-only, checks existing PRs)
    let plan = rt.block_on(service.create_plan(&stack, &config))?;

    // Single dry-run check point
    if dry_run {
        return handle_dry_run_output(&plan, json, &config.default_branch);
    }

    // Phase 2: Execute the plan (mutations only)
    if !json {
        output::info(&format!("Submitting to {owner}/{repo_name}..."));
    }

    // Warn about diverged branches before pushing
    for action in &plan.actions {
        let branch = match action {
            PlannedBranchAction::Update { branch, .. }
            | PlannedBranchAction::Create { branch, .. } => branch,
        };
        warn_if_diverged(&repo, branch, force, json);
    }

    let results = rt.block_on(service.execute(&mut stack, &plan, force))?;

    // Print progress for each result
    if !json {
        for result in &results {
            match result.action {
                SubmitAction::Created => {
                    output::success(&format!(
                        "  Created PR #{}: {}",
                        result.pr_number, result.pr_url
                    ));
                }
                SubmitAction::Updated => {
                    output::info(&format!("  Updated PR #{}", result.pr_number));
                }
            }
        }
    }

    // Save state and update comments (only after real execution)
    state.save_stack(&stack)?;
    if !json {
        output::info("Updating stack comments...");
    }
    rt.block_on(service.update_stack_comments(&stack, &config.default_branch))?;

    let (created, updated) = results
        .iter()
        .fold((0, 0), |(c, u), info| match info.action {
            SubmitAction::Created => (c + 1, u),
            SubmitAction::Updated => (c, u + 1),
        });

    // Output results
    if json {
        return output_json(&SubmitOutput {
            prs_created: created,
            prs_updated: updated,
            branches: results.into_iter().map(Into::into).collect(),
            dry_run: false,
        });
    }

    print_summary(created, updated);

    Ok(())
}

/// Output submit result as JSON.
fn output_json(output: &SubmitOutput) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(output)?);
    Ok(())
}

/// Set up repository, state, and stack for submit.
fn setup_submit() -> Result<(Repository, State, rung_core::stack::Stack)> {
    let repo = Repository::open_current().context("Not inside a git repository")?;
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    if !state.is_initialized() {
        bail!("Rung not initialized - run `rung init` first");
    }

    repo.require_clean()?;
    let stack = state.load_stack()?;

    Ok((repo, state, stack))
}

/// Get owner and repo name from remote.
fn get_remote_info(repo: &Repository) -> Result<(String, String)> {
    let origin_url = repo.origin_url().context("No origin remote configured")?;
    Repository::parse_github_remote(&origin_url).context("Could not parse GitHub remote URL")
}

/// Warn if a branch has diverged from its remote and force is not enabled.
fn warn_if_diverged(repo: &Repository, branch: &str, force: bool, json: bool) {
    if force || json {
        return;
    }
    if let Ok(RemoteDivergence::Diverged { ahead, behind }) = repo.remote_divergence(branch) {
        output::warn(&format!(
            "{branch} has diverged from remote ({ahead} ahead, {behind} behind)"
        ));
        output::detail("  Use --force to safely update (uses --force-with-lease)");
    }
}

// ============================================================================
// Dry-Run Output
// ============================================================================

/// JSON output for dry-run mode.
#[derive(Debug, Serialize)]
struct DryRunOutput {
    prs_would_create: usize,
    prs_would_update: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    branches: Vec<PlannedBranchInfo>,
    dry_run: bool,
}

/// Handle dry-run output (both JSON and human-readable).
fn handle_dry_run_output(plan: &SubmitPlan, json: bool, default_branch: &str) -> Result<()> {
    if json {
        return output_dry_run_json(plan);
    }

    print_dry_run_summary(plan, default_branch);
    Ok(())
}

/// Output dry-run result as JSON.
fn output_dry_run_json(plan: &SubmitPlan) -> Result<()> {
    let branches: Vec<PlannedBranchInfo> = plan
        .actions
        .iter()
        .map(|action| match action {
            PlannedBranchAction::Update {
                branch,
                pr_number,
                pr_url,
                ..
            } => PlannedBranchInfo {
                branch: branch.clone(),
                pr_number: Some(*pr_number),
                pr_url: Some(pr_url.clone()),
                target_base: None,
                action: OutputAction::Updated,
            },
            PlannedBranchAction::Create { branch, base, .. } => PlannedBranchInfo {
                branch: branch.clone(),
                pr_number: None,
                pr_url: None,
                target_base: Some(base.clone()),
                action: OutputAction::Created,
            },
        })
        .collect();

    let output = DryRunOutput {
        prs_would_create: plan.count_creates(),
        prs_would_update: plan.count_updates(),
        branches,
        dry_run: true,
    };

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// Print human-readable summary for dry-run mode.
fn print_dry_run_summary(plan: &SubmitPlan, default_branch: &str) {
    if plan.is_empty() {
        output::info("No branches to submit");
        return;
    }

    let updates: Vec<_> = plan
        .actions
        .iter()
        .filter_map(|a| match a {
            PlannedBranchAction::Update {
                branch, pr_number, ..
            } => Some((branch, pr_number)),
            PlannedBranchAction::Create { .. } => None,
        })
        .collect();

    let creates: Vec<_> = plan
        .actions
        .iter()
        .filter_map(|a| match a {
            PlannedBranchAction::Create { branch, base, .. } => Some((branch, base)),
            PlannedBranchAction::Update { .. } => None,
        })
        .collect();

    let mut parts = vec![];

    if !updates.is_empty() {
        parts.push(format!("→ Would push {} branches:", updates.len()));
        for (branch, pr_number) in &updates {
            parts.push(format!("  - {branch} (PR #{pr_number})"));
        }
        parts.push(String::new());
    }

    if !creates.is_empty() {
        parts.push(format!(
            "→ Would create {} new PRs for branches:",
            creates.len()
        ));
        for (branch, base) in &creates {
            let target = if base.is_empty() {
                default_branch
            } else {
                base
            };
            parts.push(format!("  - {branch} → {target}"));
        }
        parts.push(String::new());
    }

    parts.push("(dry run - no changes made)".into());
    output::essential(&parts.join("\n"));
}

/// Validate that the stack is in sync with the base branch.
fn validate_sync_state(
    repo: &Repository,
    stack: &Stack,
    base_branch: &str,
    json: bool,
) -> Result<()> {
    if !json {
        output::info(&format!("Checking sync status against {base_branch}..."));
    }

    // 1. Fetch latest from remote (updates local tracking branch)
    if let Err(e) = repo.fetch(base_branch) {
        if !json {
            output::warn(&format!("Could not fetch {base_branch}: {e}"));
        }
    }

    // 2. Check if stack needs syncing
    let sync_plan = sync::create_sync_plan(repo, stack, base_branch)?;

    if !sync_plan.is_empty() {
        let affected: Vec<&str> = sync_plan
            .branches
            .iter()
            .map(|a| a.branch.as_str())
            .collect();
        let message = format!(
            "Stack is out of sync with {base_branch}. {} branch(es) need rebasing: {}\n\n\
             → Run `rung sync` to update the stack\n\
             → Use `--force` to push without syncing (may create conflicts)",
            affected.len(),
            affected.join(", ")
        );

        if json {
            bail!(message);
        }
        // For CLI users, use the colored error output then exit
        bail!(message);
    }
    Ok(())
}
/// Print summary of submit operation.
fn print_summary(created: usize, updated: usize) {
    if created > 0 || updated > 0 {
        let mut parts = vec![];
        if created > 0 {
            parts.push(format!("{created} created"));
        }
        if updated > 0 {
            parts.push(format!("{updated} updated"));
        }
        output::success(&format!("Done! PRs: {}", parts.join(", ")));
    } else {
        output::info("No changes to submit");
    }
}
#[cfg(test)]
mod test {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;
    use rung_core::stack::{Stack, StackBranch};
    use rung_git::Repository;
    use std::fs;
    use std::process::Command as StdCommand;
    use tempfile::TempDir;

    /// Helper to create a test git repository.
    fn setup_test_repo() -> (TempDir, Repository) {
        let temp = TempDir::new().expect("Failed to create temp dir");

        // Initialize git repo
        StdCommand::new("git")
            .args(["init"])
            .current_dir(&temp)
            .output()
            .expect("Failed to init git repo");
        StdCommand::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&temp)
            .output()
            .expect("Failed to set git user email");
        StdCommand::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(&temp)
            .output()
            .expect("Failed to set git user name");

        // Create initial commit
        let readme = temp.path().join("README.md");
        fs::write(&readme, "# Test Repo\n").expect("Failed to write README");

        StdCommand::new("git")
            .args(["add", "."])
            .current_dir(&temp)
            .output()
            .expect("Failed to git add");

        StdCommand::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(&temp)
            .output()
            .expect("Failed to git commit");

        StdCommand::new("git")
            .args(["branch", "-M", "main"])
            .current_dir(&temp)
            .output()
            .expect("Failed to set main branch");

        let repo = Repository::open(temp.path()).expect("Failed to open repo");
        (temp, repo)
    }

    // Helper to create a branch with commits.
    fn create_branch_with_commits(temp: &TempDir, branch_name: &str, commit_msg: &str) {
        StdCommand::new("git")
            .args(["checkout", "-b", branch_name])
            .current_dir(temp)
            .output()
            .expect("Failed to create branch");

        let file = temp.path().join("feature.txt");
        fs::write(&file, "Feature content").expect("Failed to write file");

        StdCommand::new("git")
            .args(["add", "."])
            .current_dir(temp)
            .output()
            .expect("Failed to git add");

        StdCommand::new("git")
            .args(["commit", "-m", commit_msg])
            .current_dir(temp)
            .output()
            .expect("Failed to git commit");
    }

    #[test]
    fn test_validate_sync_state_up_to_date() {
        let (temp, repo) = setup_test_repo();

        // Create a simple branch off main
        create_branch_with_commits(&temp, "feature-1", "Add feature");

        let mut stack = Stack::new();
        let branch = rung_core::stack::StackBranch::try_new("feature-1", Some("main"))
            .expect("Failed to create stack branch");
        stack.add_branch(branch);

        // Should pass validate (branch is base on latest main)
        let result = validate_sync_state(&repo, &stack, "main", false);
        assert!(result.is_ok(), "Stack should be up to date");
    }

    #[test]
    fn test_validate_sync_state_needs_sync() {
        let (temp, repo) = setup_test_repo();

        // Create feature branch
        create_branch_with_commits(&temp, "feature-1", "Add feature");

        // Go back to main and add another commit (simulate remote change)
        StdCommand::new("git")
            .args(["checkout", "main"])
            .current_dir(&temp)
            .output()
            .expect("Failed to checkout main");

        let file = temp.path().join("main-change.txt");
        fs::write(&file, "Main branch change").expect("Failed to write file");

        StdCommand::new("git")
            .args(["add", "."])
            .current_dir(&temp)
            .output()
            .expect("Failed to git add");

        StdCommand::new("git")
            .args(["commit", "-m", "Main branch update"])
            .current_dir(&temp)
            .output()
            .expect("Failed to git commit");

        // Go back to feature branch
        StdCommand::new("git")
            .args(["checkout", "feature-1"])
            .current_dir(&temp)
            .output()
            .expect("Failed to checkout feature-1");

        let mut stack = Stack::new();
        let branch =
            StackBranch::try_new("feature-1", Some("main")).expect("Failed to create stack branch");
        stack.add_branch(branch);

        // Should fail validate (feature branch is behind main which has new commit)
        let result = validate_sync_state(&repo, &stack, "main", true);
        assert!(result.is_err(), "Stack should need syncing");

        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("out of sync"),
            "Error should mention sync status"
        );
        assert!(
            error_msg.contains("feature-1"),
            "Error should mention the branch"
        );
    }

    #[test]
    fn test_validate_sync_state_empty_stack() {
        let (_temp, repo) = setup_test_repo();
        let stack = Stack::new(); // Empty stack

        // Should pass validate (no branches to check)
        let result = validate_sync_state(&repo, &stack, "main", false);
        assert!(result.is_ok(), "Empty stack should be valid");
    }

    #[test]
    fn test_validate_sync_state_fetch_error_continues() {
        let (_temp, repo) = setup_test_repo();
        let stack = Stack::new();

        // Should handle fetch errors gracefully and continue with local check
        let result = validate_sync_state(&repo, &stack, "nonexistent-branch", false);
        assert!(result.is_ok(), "Should handle fetch errors gracefully");
    }
}
