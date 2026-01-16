//! `rung doctor` command - Diagnose issues with the stack and repository.

use anyhow::Result;
use colored::Colorize;
use rung_core::State;
use rung_git::Repository;
use rung_github::{Auth, GitHubClient, PullRequestState};
use serde::Serialize;

use crate::output;

/// Diagnostic issue severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Severity {
    Error,
    Warning,
}

/// A diagnostic issue found by the doctor.
#[derive(Debug, Clone, Serialize)]
struct Issue {
    severity: Severity,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    suggestion: Option<String>,
}

/// JSON output for doctor command.
#[derive(Debug, Serialize)]
struct DoctorOutput {
    healthy: bool,
    errors: usize,
    warnings: usize,
    issues: Vec<Issue>,
}

impl Issue {
    fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            suggestion: None,
        }
    }

    fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            suggestion: None,
        }
    }

    fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }
}

/// Run the doctor command.
pub fn run(json: bool) -> Result<()> {
    let mut issues: Vec<Issue> = Vec::new();

    // Check if we're in a git repo
    let Ok(repo) = Repository::open_current() else {
        if json {
            return output_json(&[Issue::error("Not inside a git repository")]);
        }
        output::error("Not inside a git repository");
        return Ok(());
    };

    let Some(workdir) = repo.workdir() else {
        if json {
            return output_json(&[Issue::error("Cannot run in bare repository")]);
        }
        output::error("Cannot run in bare repository");
        return Ok(());
    };

    let state = State::new(workdir)?;

    // Check initialization
    if !json {
        println!();
        print_check("Checking rung initialization...");
    }
    if !state.is_initialized() {
        issues.push(
            Issue::error("Rung not initialized in this repository")
                .with_suggestion("Run `rung init` to initialize"),
        );
        if json {
            return output_json(&issues);
        }
        print_issues(&issues);
        return Ok(());
    }
    if !json {
        print_ok();
    }

    // Check git state
    if !json {
        print_check("Checking git state...");
    }
    check_git_state(&repo, &mut issues);
    if !json {
        print_status(&issues, "git state");
    }

    // Check stack integrity
    if !json {
        print_check("Checking stack integrity...");
    }
    let stack = state.load_stack()?;
    check_stack_integrity(&repo, &stack, &mut issues);
    if !json {
        print_status(&issues, "stack integrity");
    }

    // Check sync state
    if !json {
        print_check("Checking sync state...");
    }
    check_sync_state(&repo, &state, &stack, &mut issues);
    if !json {
        print_status(&issues, "sync state");
    }

    // Check GitHub connectivity
    if !json {
        print_check("Checking GitHub...");
    }
    check_github(&repo, &stack, &mut issues);
    if !json {
        print_status(&issues, "GitHub");
    }

    // Output
    if json {
        return output_json(&issues);
    }

    println!();
    print_issues(&issues);
    print_summary(&issues);

    Ok(())
}

/// Output issues as JSON.
fn output_json(issues: &[Issue]) -> Result<()> {
    let errors = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .count();
    let warnings = issues
        .iter()
        .filter(|i| i.severity == Severity::Warning)
        .count();

    let output = DoctorOutput {
        healthy: errors == 0 && warnings == 0,
        errors,
        warnings,
        issues: issues.to_vec(),
    };

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn print_check(message: &str) {
    print!("  {message}");
}

fn print_ok() {
    println!(" {}", "✓".green());
}

fn print_status(issues: &[Issue], _category: &str) {
    let has_errors = issues.iter().any(|i| i.severity == Severity::Error);
    let has_warnings = issues.iter().any(|i| i.severity == Severity::Warning);

    if has_errors {
        println!(" {}", "✗".red());
    } else if has_warnings {
        println!(" {}", "⚠".yellow());
    } else {
        println!(" {}", "✓".green());
    }
}

fn print_issues(issues: &[Issue]) {
    if issues.is_empty() {
        return;
    }

    for issue in issues {
        let icon = match issue.severity {
            Severity::Error => "✗".red(),
            Severity::Warning => "⚠".yellow(),
        };

        println!("  {icon} {}", issue.message);

        if let Some(suggestion) = &issue.suggestion {
            println!("    {} {suggestion}", "→".dimmed());
        }
    }
    println!();
}

fn print_summary(issues: &[Issue]) {
    let errors = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .count();
    let warnings = issues
        .iter()
        .filter(|i| i.severity == Severity::Warning)
        .count();

    if errors == 0 && warnings == 0 {
        output::success("No issues found!");
    } else {
        let summary = format!(
            "Found {} issue(s) ({} error(s), {} warning(s))",
            errors + warnings,
            errors,
            warnings
        );
        if errors > 0 {
            output::error(&summary);
        } else {
            output::warn(&summary);
        }
    }
    println!();
}

/// Check git repository state.
fn check_git_state(repo: &Repository, issues: &mut Vec<Issue>) {
    // Check for dirty working directory
    if !repo.is_clean().unwrap_or(false) {
        issues.push(
            Issue::warning("Working directory has uncommitted changes")
                .with_suggestion("Commit or stash changes before running rung commands"),
        );
    }

    // Check for detached HEAD
    if repo.current_branch().is_err() {
        issues.push(
            Issue::error("HEAD is detached (not on a branch)")
                .with_suggestion("Checkout a branch with `git checkout <branch>`"),
        );
    }

    // Check for rebase in progress
    if repo.is_rebasing() {
        issues.push(
            Issue::error("Rebase in progress")
                .with_suggestion("Complete or abort the rebase before running rung commands"),
        );
    }

    // Check for sync in progress
    // This is handled by State, so we skip it here
}

/// Check stack integrity.
fn check_stack_integrity(repo: &Repository, stack: &rung_core::Stack, issues: &mut Vec<Issue>) {
    for branch in &stack.branches {
        // Check if branch exists locally
        if !repo.branch_exists(&branch.name) {
            issues.push(
                Issue::warning(format!("Branch '{}' in stack but not in git", branch.name))
                    .with_suggestion("Run `rung sync` to clean up stale branches"),
            );
            continue;
        }

        // Check if parent exists (for non-root branches)
        if let Some(parent) = &branch.parent {
            if !repo.branch_exists(parent) && stack.find_branch(parent).is_none() {
                issues.push(
                    Issue::error(format!(
                        "Branch '{}' has missing parent '{}'",
                        branch.name, parent
                    ))
                    .with_suggestion("Run `rung sync` to re-parent orphaned branches"),
                );
            }
        }
    }

    // Check for circular dependencies
    for branch in &stack.branches {
        if has_circular_dependency(stack, &branch.name, &mut vec![]) {
            issues.push(Issue::error(format!(
                "Circular dependency detected involving '{}'",
                branch.name
            )));
        }
    }
}

/// Check if a branch has a circular dependency.
fn has_circular_dependency(
    stack: &rung_core::Stack,
    branch_name: &str,
    visited: &mut Vec<String>,
) -> bool {
    if visited.contains(&branch_name.to_string()) {
        return true;
    }

    visited.push(branch_name.to_string());

    if let Some(branch) = stack.find_branch(branch_name) {
        if let Some(parent) = &branch.parent {
            if stack.find_branch(parent).is_some() {
                return has_circular_dependency(stack, parent, visited);
            }
        }
    }

    false
}

/// Check sync state of branches.
fn check_sync_state(
    repo: &Repository,
    state: &State,
    stack: &rung_core::Stack,
    issues: &mut Vec<Issue>,
) {
    // Check if sync is in progress
    if state.is_sync_in_progress() {
        issues.push(
            Issue::warning("Sync operation in progress")
                .with_suggestion("Run `rung sync --continue` or `rung sync --abort`"),
        );
    }

    // Check each branch's sync state
    let mut needs_sync = 0;
    for branch in &stack.branches {
        if !repo.branch_exists(&branch.name) {
            continue;
        }

        let parent_name = branch.parent.as_deref().unwrap_or("main");
        if !repo.branch_exists(parent_name) {
            continue;
        }

        // Check if branch needs rebasing
        if let (Ok(branch_commit), Ok(parent_commit)) = (
            repo.branch_commit(&branch.name),
            repo.branch_commit(parent_name),
        ) {
            if let Ok(merge_base) = repo.merge_base(branch_commit, parent_commit) {
                if merge_base != parent_commit {
                    needs_sync += 1;
                }
            }
        }
    }

    if needs_sync > 0 {
        issues.push(
            Issue::warning(format!("{needs_sync} branch(es) behind their parent"))
                .with_suggestion("Run `rung sync` to rebase"),
        );
    }
}

/// Check GitHub connectivity and PR state.
fn check_github(repo: &Repository, stack: &rung_core::Stack, issues: &mut Vec<Issue>) {
    // Check auth
    let auth = Auth::auto();
    let Ok(client) = GitHubClient::new(&auth) else {
        issues.push(
            Issue::error("GitHub authentication failed")
                .with_suggestion("Set GITHUB_TOKEN or authenticate with `gh auth login`"),
        );
        return;
    };

    // Get repo info
    let Ok(origin_url) = repo.origin_url() else {
        issues.push(Issue::warning("No origin remote configured"));
        return;
    };

    let Ok((owner, repo_name)) = Repository::parse_github_remote(&origin_url) else {
        issues.push(Issue::warning("Origin is not a GitHub repository"));
        return;
    };

    // Check PRs for branches that have them
    let Ok(rt) = tokio::runtime::Runtime::new() else {
        return;
    };

    for branch in &stack.branches {
        let Some(pr_number) = branch.pr else {
            continue;
        };

        // Check if PR is still open
        match rt.block_on(client.get_pr(&owner, &repo_name, pr_number)) {
            Ok(pr) => {
                if pr.state != PullRequestState::Open {
                    let state_str = match pr.state {
                        PullRequestState::Closed => "closed",
                        PullRequestState::Merged => "merged",
                        PullRequestState::Open => "open",
                    };
                    issues.push(
                        Issue::warning(format!(
                            "PR #{} for '{}' is {} (not open)",
                            pr_number, branch.name, state_str
                        ))
                        .with_suggestion("Run `rung sync` to clean up or merge the branch"),
                    );
                }
            }
            Err(_) => {
                issues.push(Issue::warning(format!(
                    "Could not fetch PR #{} for '{}'",
                    pr_number, branch.name
                )));
            }
        }
    }
}
