//! `rung submit` command - Push branches and create/update PRs.

use std::fmt::Write;

use anyhow::{Context, Result, bail};
use rung_core::{State, stack::Stack};
use rung_git::{RemoteDivergence, Repository};
use rung_github::{
    Auth, CreateComment, CreatePullRequest, GitHubClient, UpdateComment, UpdatePullRequest,
};
use serde::Serialize;

use crate::commands::utils;
use crate::output;

/// A planned action for a single branch.
#[derive(Debug)]
enum PlannedBranchAction {
    /// Update an existing PR (push branch, update base).
    Update {
        branch: String,
        pr_number: u64,
        pr_url: String,
        base: String,
    },
    /// Create a new PR.
    Create {
        branch: String,
        title: String,
        body: String,
        base: String,
        draft: bool,
    },
}

/// The complete submit plan describing what will happen.
#[derive(Debug)]
struct SubmitPlan {
    actions: Vec<PlannedBranchAction>,
}

impl SubmitPlan {
    fn count_creates(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| matches!(a, PlannedBranchAction::Create { .. }))
            .count()
    }

    fn count_updates(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| matches!(a, PlannedBranchAction::Update { .. }))
            .count()
    }
}

/// JSON output for submit command.
#[derive(Debug, Serialize)]
struct SubmitOutput {
    prs_created: usize,
    prs_updated: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    branches: Vec<BranchSubmitInfo>,
    dry_run: bool,
}

/// Information about a submitted branch (after execution).
#[derive(Debug, Serialize)]
struct BranchSubmitInfo {
    branch: String,
    pr_number: u64,
    pr_url: String,
    action: SubmitAction,
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
    action: SubmitAction,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum SubmitAction {
    Created,
    Updated,
}

/// Configuration options for the submit command (planning phase).
struct SubmitConfig<'a> {
    /// Create PRs as drafts.
    draft: bool,
    /// Custom title for the current branch's PR.
    custom_title: Option<&'a str>,
    /// Current branch name (for custom title matching).
    current_branch: Option<String>,
    /// Default base branch (from config, falls back to "main").
    default_branch: String,
}

/// Context for GitHub API operations.
struct GitHubContext<'a> {
    client: &'a GitHubClient,
    rt: &'a tokio::runtime::Runtime,
    owner: &'a str,
    repo_name: &'a str,
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
                return output_dry_run_json(&SubmitPlan { actions: vec![] });
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

    let gh = GitHubContext {
        client: &client,
        rt: &rt,
        owner: &owner,
        repo_name: &repo_name,
    };

    // Phase 1: Create the plan (read-only, checks existing PRs)
    let plan = create_submit_plan(&repo, &gh, &stack, &config)?;

    // Single dry-run check point
    if dry_run {
        return handle_dry_run_output(&plan, json, &config.default_branch);
    }

    // Phase 2: Execute the plan (mutations only)
    if !json {
        output::info(&format!("Submitting to {owner}/{repo_name}..."));
    }
    let branch_infos = execute_submit(&repo, &gh, &mut stack, &plan, force, json)?;

    // Save state and update comments (only after real execution)
    state.save_stack(&stack)?;
    update_stack_comments(&gh, &stack, json, &config.default_branch)?;

    let (created, updated) = branch_infos
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
            branches: branch_infos,
            dry_run: false,
        });
    }

    print_summary(created, updated);

    // Output PR URLs for piping (essential output, not suppressed by --quiet)
    for info in &branch_infos {
        output::essential(&info.pr_url);
    }

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

/// Create a submit plan by checking existing PRs (read-only).
///
/// This function iterates through all branches in the stack and determines
/// what action would be taken for each branch (create new PR or update existing).
///
/// # Errors
/// Returns error if any GitHub API calls fail.
fn create_submit_plan(
    repo: &Repository,
    gh: &GitHubContext<'_>,
    stack: &rung_core::stack::Stack,
    config: &SubmitConfig<'_>,
) -> Result<SubmitPlan> {
    let mut actions = Vec::new();

    for branch in &stack.branches {
        let branch_name = &branch.name;
        let base_branch = branch
            .parent
            .as_deref()
            .unwrap_or(&config.default_branch)
            .to_string();

        // Get title and body from commit message, with custom title override for current branch
        let (mut title, body) = get_pr_title_and_body(repo, branch_name);
        if config.current_branch.as_deref() == Some(branch_name.as_str()) {
            if let Some(custom) = config.custom_title {
                title = custom.to_string();
            }
        }

        // Check if PR already exists (either from saved state or by querying GitHub)
        if let Some(pr_number) = branch.pr {
            // PR number is already known from saved state
            let pr_url = format!(
                "https://github.com/{}/{}/pull/{pr_number}",
                gh.owner, gh.repo_name
            );
            actions.push(PlannedBranchAction::Update {
                branch: branch_name.to_string(),
                pr_number,
                pr_url,
                base: base_branch,
            });
        } else {
            let existing = gh
                .rt
                .block_on(
                    gh.client
                        .find_pr_for_branch(gh.owner, gh.repo_name, branch_name),
                )
                .context("Failed to check for existing PR")?;

            if let Some(pr) = existing {
                actions.push(PlannedBranchAction::Update {
                    branch: branch_name.to_string(),
                    pr_number: pr.number,
                    pr_url: pr.html_url,
                    base: base_branch,
                });
            } else {
                actions.push(PlannedBranchAction::Create {
                    branch: branch_name.to_string(),
                    title,
                    body,
                    base: base_branch,
                    draft: config.draft,
                });
            }
        }
    }

    Ok(SubmitPlan { actions })
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

/// Push a branch with optional progress output.
fn push_branch(repo: &Repository, branch: &str, force: bool, json: bool) -> Result<()> {
    if !json {
        output::info(&format!("  Pushing {branch}..."));
    }
    repo.push(branch, force)
        .with_context(|| format!("Failed to push {branch}"))
}

/// Execute the submit plan (mutations only).
///
/// This function pushes branches and creates/updates PRs according to the plan.
/// It also updates the stack state with new PR numbers.
///
/// # Errors
/// Returns error if any GitHub API calls or git operations fail.
fn execute_submit(
    repo: &Repository,
    gh: &GitHubContext<'_>,
    stack: &mut rung_core::stack::Stack,
    plan: &SubmitPlan,
    force: bool,
    json: bool,
) -> Result<Vec<BranchSubmitInfo>> {
    let mut branch_infos = Vec::new();

    for action in &plan.actions {
        match action {
            PlannedBranchAction::Update {
                branch,
                pr_number,
                pr_url,
                base,
            } => {
                if !json {
                    output::info(&format!("Processing {branch}..."));
                }
                warn_if_diverged(repo, branch, force, json);
                push_branch(repo, branch, force, json)?;
                update_existing_pr(gh, *pr_number, base, json)?;

                // Persist PR number if it was discovered during planning
                if let Some(stack_branch) = stack.branches.iter_mut().find(|b| &b.name == branch) {
                    if stack_branch.pr.is_none() {
                        stack_branch.pr = Some(*pr_number);
                    }
                }

                branch_infos.push(BranchSubmitInfo {
                    branch: branch.clone(),
                    pr_number: *pr_number,
                    pr_url: pr_url.clone(),
                    action: SubmitAction::Updated,
                });
            }
            PlannedBranchAction::Create {
                branch,
                title,
                body,
                base,
                draft,
            } => {
                if !json {
                    output::info(&format!("Processing {branch}..."));
                }
                warn_if_diverged(repo, branch, force, json);
                push_branch(repo, branch, force, json)?;

                // Check if a PR was created between planning and execution
                let existing = gh
                    .rt
                    .block_on(gh.client.find_pr_for_branch(gh.owner, gh.repo_name, branch))
                    .context("Failed to check for existing PR")?;

                let (pr_number, pr_url, was_created) = if let Some(pr) = existing {
                    // PR was created between planning and execution - update it instead
                    if !json {
                        output::info(&format!("  Found existing PR #{}...", pr.number));
                    }

                    let update = UpdatePullRequest {
                        title: None,
                        body: None,
                        base: Some(base.clone()),
                    };

                    gh.rt
                        .block_on(
                            gh.client
                                .update_pr(gh.owner, gh.repo_name, pr.number, update),
                        )
                        .with_context(|| format!("Failed to update PR #{}", pr.number))?;

                    (pr.number, pr.html_url, false)
                } else {
                    // Create new PR
                    if !json {
                        output::info(&format!("  Creating PR ({branch} → {base})..."));
                    }

                    let create = CreatePullRequest {
                        title: title.clone(),
                        body: body.clone(),
                        head: branch.clone(),
                        base: base.clone(),
                        draft: *draft,
                    };

                    let pr = gh
                        .rt
                        .block_on(gh.client.create_pr(gh.owner, gh.repo_name, create))
                        .with_context(|| format!("Failed to create PR for {branch}"))?;

                    if !json {
                        output::success(&format!("  Created PR #{}: {}", pr.number, pr.html_url));
                    }

                    (pr.number, pr.html_url, true)
                };

                // Update stack state with the PR number
                if let Some(stack_branch) = stack.branches.iter_mut().find(|b| &b.name == branch) {
                    stack_branch.pr = Some(pr_number);
                }

                branch_infos.push(BranchSubmitInfo {
                    branch: branch.clone(),
                    pr_number,
                    pr_url,
                    action: if was_created {
                        SubmitAction::Created
                    } else {
                        SubmitAction::Updated
                    },
                });
            }
        }
    }

    Ok(branch_infos)
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
                action: SubmitAction::Updated,
            },
            PlannedBranchAction::Create { branch, base, .. } => PlannedBranchInfo {
                branch: branch.clone(),
                pr_number: None,
                pr_url: None,
                target_base: Some(base.clone()),
                action: SubmitAction::Created,
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
    if plan.actions.is_empty() {
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

/// Generate PR title from branch name.
fn generate_title(branch_name: &str) -> String {
    let base = branch_name
        .split('/')
        .next_back()
        .unwrap_or(branch_name)
        .replace(['-', '_'], " ");

    base.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |c| {
                c.to_uppercase().collect::<String>() + chars.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Get PR title and body from the branch's tip commit message.
///
/// Returns (title, body) where:
/// - title is the first line of the commit message
/// - body is the remaining lines (after the first blank line), or empty string if none
///
/// Falls back to generated title from branch name if commit message can't be read.
fn get_pr_title_and_body(repo: &Repository, branch_name: &str) -> (String, String) {
    if let Ok(message) = repo.branch_commit_message(branch_name) {
        let mut lines = message.lines();
        let title = lines.next().unwrap_or("").trim().to_string();

        // Skip blank lines after title, then collect the rest as body
        // Use trim_end() to preserve leading indentation for markdown formatting
        let body: String = lines
            .skip_while(|line| line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_string();

        // Only use commit message if title is non-empty
        if !title.is_empty() {
            return (title, body);
        }
    }

    // Fallback to slugified branch name
    (generate_title(branch_name), String::new())
}

/// Update an existing PR (only updates base branch, preserves description).
fn update_existing_pr(
    gh: &GitHubContext<'_>,
    pr_number: u64,
    base_branch: &str,
    json: bool,
) -> Result<()> {
    if !json {
        output::info(&format!("  Updating PR #{pr_number}..."));
    }

    let update = UpdatePullRequest {
        title: None,
        body: None, // Preserve existing description
        base: Some(base_branch.to_string()),
    };

    gh.rt
        .block_on(
            gh.client
                .update_pr(gh.owner, gh.repo_name, pr_number, update),
        )
        .with_context(|| format!("Failed to update PR #{pr_number}"))?;

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

/// Marker to identify rung stack comments.
const STACK_COMMENT_MARKER: &str = "<!-- rung-stack -->";

/// Generate stack comment for a PR.
fn generate_stack_comment(stack: &Stack, current_pr: u64, default_branch: &str) -> String {
    let mut comment = String::from(STACK_COMMENT_MARKER);
    comment.push('\n');

    let branches = &stack.branches;

    // Find the current branch
    let current_branch = branches.iter().find(|b| b.pr == Some(current_pr));
    let current_name = current_branch.map_or("", |b| b.name.as_str());

    // Build the chain for this branch (includes merged branches)
    let chain = build_branch_chain(stack, current_name);

    // Build stack list in markdown format (newest at top, so iterate in reverse)
    for branch_name in chain.iter().rev() {
        let is_current = branch_name == current_name;
        let pointer = if is_current { " 👈" } else { "" };

        // Check if this is a merged branch
        if let Some(merged) = stack.find_merged(branch_name) {
            // Show merged branches with strikethrough
            let _ = writeln!(comment, "* ~~**#{}**~~ ✓{pointer}", merged.pr);
        } else if let Some(b) = branches.iter().find(|b| &b.name == branch_name) {
            if let Some(pr_num) = b.pr {
                // GitHub auto-links and expands #number to show PR title
                let _ = writeln!(comment, "* **#{pr_num}**{pointer}");
            } else {
                let _ = writeln!(comment, "* *(pending)* `{branch_name}`{pointer}");
            }
        }
    }

    // Add base branch (main)
    let base = current_branch
        .and_then(|b| {
            // Walk up to find the root's parent
            let mut current = b;
            loop {
                if let Some(ref parent) = current.parent {
                    if let Some(p) = branches.iter().find(|br| &br.name == parent) {
                        current = p;
                    } else {
                        // Check if parent is a merged branch
                        if stack.find_merged(parent).is_some() {
                            // Continue walking up - the merged branch's parent info is lost
                            // so we use the original parent name
                            return Some(parent.as_str());
                        }
                        return Some(parent.as_str());
                    }
                } else {
                    return Some(default_branch);
                }
            }
        })
        .unwrap_or(default_branch);

    let _ = writeln!(comment, "* `{base}`");
    comment.push_str("\n---\n*Managed by [rung](https://github.com/auswm85/rung)*");

    comment
}

/// Update stack comments on all PRs in the stack.
fn update_stack_comments(
    gh: &GitHubContext<'_>,
    stack: &Stack,
    json: bool,
    default_branch: &str,
) -> Result<()> {
    if !json {
        output::info("Updating stack comments...");
    }

    for branch in &stack.branches {
        let Some(pr_number) = branch.pr else {
            continue;
        };

        let comment_body = generate_stack_comment(stack, pr_number, default_branch);

        // Find existing rung comment
        let comments = gh
            .rt
            .block_on(
                gh.client
                    .list_pr_comments(gh.owner, gh.repo_name, pr_number),
            )
            .with_context(|| format!("Failed to list comments on PR #{pr_number}"))?;

        let existing_comment = comments.iter().find(|c| {
            c.body
                .as_ref()
                .is_some_and(|b| b.contains(STACK_COMMENT_MARKER))
        });

        if let Some(comment) = existing_comment {
            // Update existing comment
            let update = UpdateComment { body: comment_body };
            gh.rt
                .block_on(
                    gh.client
                        .update_pr_comment(gh.owner, gh.repo_name, comment.id, update),
                )
                .with_context(|| format!("Failed to update comment on PR #{pr_number}"))?;
        } else {
            // Create new comment
            let create = CreateComment { body: comment_body };
            gh.rt
                .block_on(
                    gh.client
                        .create_pr_comment(gh.owner, gh.repo_name, pr_number, create),
                )
                .with_context(|| format!("Failed to create comment on PR #{pr_number}"))?;
        }
    }

    Ok(())
}

/// Build a chain of branches from root ancestor to all descendants.
///
/// Returns branch names in order from oldest ancestor to newest descendant.
/// Includes merged branches in the chain for history preservation.
fn build_branch_chain(stack: &Stack, current_name: &str) -> Vec<String> {
    let branches = &stack.branches;

    // Find all ancestors (walk up the parent chain, including merged branches)
    let mut ancestors: Vec<String> = vec![];
    let mut current = current_name.to_string();

    loop {
        // Get the parent of current (from active or merged branch)
        let parent = branches
            .iter()
            .find(|b| b.name == current)
            .and_then(|b| b.parent.as_ref())
            .or_else(|| stack.find_merged(&current).and_then(|m| m.parent.as_ref()))
            .map(ToString::to_string);

        let Some(parent_name) = parent else {
            break;
        };

        // Check if parent is in active stack or merged list
        let in_active = branches.iter().any(|b| b.name == parent_name);
        let in_merged = stack.find_merged(&parent_name).is_some();

        if in_active || in_merged {
            ancestors.push(parent_name.clone());
            current = parent_name;
        } else {
            // Parent not in stack (reached base like main)
            break;
        }
    }

    // Reverse to get oldest ancestor first
    ancestors.reverse();

    // Start chain with ancestors, then current
    let mut chain = ancestors;
    chain.push(current_name.to_string());

    // Find all descendants (branches whose parent is in our chain)
    let mut i = 0;
    while i < chain.len() {
        let parent_name = chain[i].clone();
        for branch in branches {
            if branch.parent.as_ref().is_some_and(|p| p == &parent_name)
                && !chain.contains(&branch.name.to_string())
            {
                chain.push(branch.name.to_string());
            }
        }
        i += 1;
    }

    chain
}
