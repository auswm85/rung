//! `rung submit` command - Push branches and create/update PRs.

use std::fmt::Write;

use anyhow::{Context, Result, bail};
use rung_core::State;
use rung_core::stack::StackBranch;
use rung_git::Repository;
use rung_github::{Auth, CreatePullRequest, GitHubClient, UpdatePullRequest};

use crate::output;

/// Run the submit command.
pub fn run(draft: bool, force: bool) -> Result<()> {
    let (repo, state, mut stack) = setup_submit()?;

    if stack.is_empty() {
        output::info("No branches in stack - nothing to submit");
        return Ok(());
    }

    let (owner, repo_name) = get_remote_info(&repo)?;
    output::info(&format!("Submitting to {owner}/{repo_name}..."));

    let client = GitHubClient::new(&Auth::auto()).context("Failed to authenticate with GitHub")?;
    let rt = tokio::runtime::Runtime::new()?;

    let (created, updated) = process_branches(
        &repo, &client, &rt, &mut stack, &owner, &repo_name, draft, force,
    )?;

    state.save_stack(&stack)?;
    print_summary(created, updated);

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

/// Process all branches in the stack.
#[allow(clippy::too_many_arguments)]
fn process_branches(
    repo: &Repository,
    client: &GitHubClient,
    rt: &tokio::runtime::Runtime,
    stack: &mut rung_core::stack::Stack,
    owner: &str,
    repo_name: &str,
    draft: bool,
    force: bool,
) -> Result<(usize, usize)> {
    let mut created = 0;
    let mut updated = 0;

    for i in 0..stack.branches.len() {
        let branch = &stack.branches[i];
        let branch_name = branch.name.clone();
        let parent_name = branch.parent.clone();
        let existing_pr = branch.pr;

        output::info(&format!("Processing {branch_name}..."));

        // Push the branch
        output::info(&format!("  Pushing {branch_name}..."));
        repo.push(&branch_name, force)
            .with_context(|| format!("Failed to push {branch_name}"))?;

        let base_branch = parent_name.as_deref().unwrap_or("main");
        let title = generate_title(&branch_name);
        let body = generate_pr_body(&stack.branches, i);

        if let Some(pr_number) = existing_pr {
            update_existing_pr(client, rt, owner, repo_name, pr_number, body, base_branch)?;
            updated += 1;
        } else {
            let result = create_or_find_pr(
                client,
                rt,
                owner,
                repo_name,
                &branch_name,
                base_branch,
                title,
                body,
                draft,
            )?;

            stack.branches[i].pr = Some(result.pr_number);
            if result.was_created {
                created += 1;
            } else {
                updated += 1;
            }
        }
    }

    Ok((created, updated))
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

/// Update an existing PR.
fn update_existing_pr(
    client: &GitHubClient,
    rt: &tokio::runtime::Runtime,
    owner: &str,
    repo_name: &str,
    pr_number: u64,
    body: String,
    base_branch: &str,
) -> Result<()> {
    output::info(&format!("  Updating PR #{pr_number}..."));

    let update = UpdatePullRequest {
        title: None,
        body: Some(body),
        base: Some(base_branch.to_string()),
    };

    rt.block_on(client.update_pr(owner, repo_name, pr_number, update))
        .with_context(|| format!("Failed to update PR #{pr_number}"))?;

    Ok(())
}

/// Result of creating or finding a PR.
struct PrResult {
    pr_number: u64,
    was_created: bool,
}

/// Create a new PR or find an existing one.
#[allow(clippy::too_many_arguments)]
fn create_or_find_pr(
    client: &GitHubClient,
    rt: &tokio::runtime::Runtime,
    owner: &str,
    repo_name: &str,
    branch_name: &str,
    base_branch: &str,
    title: String,
    body: String,
    draft: bool,
) -> Result<PrResult> {
    // Check if PR already exists for this branch
    let existing = rt
        .block_on(client.find_pr_for_branch(owner, repo_name, branch_name))
        .context("Failed to check for existing PR")?;

    if let Some(pr) = existing {
        output::info(&format!("  Found existing PR #{}...", pr.number));

        let update = UpdatePullRequest {
            title: None,
            body: Some(body),
            base: Some(base_branch.to_string()),
        };

        rt.block_on(client.update_pr(owner, repo_name, pr.number, update))
            .with_context(|| format!("Failed to update PR #{}", pr.number))?;

        return Ok(PrResult {
            pr_number: pr.number,
            was_created: false,
        });
    }

    // Create new PR
    output::info(&format!("  Creating PR ({branch_name} → {base_branch})..."));

    let create = CreatePullRequest {
        title,
        body,
        head: branch_name.to_string(),
        base: base_branch.to_string(),
        draft,
    };

    let pr = rt
        .block_on(client.create_pr(owner, repo_name, create))
        .with_context(|| format!("Failed to create PR for {branch_name}"))?;

    output::success(&format!("  Created PR #{}: {}", pr.number, pr.html_url));

    Ok(PrResult {
        pr_number: pr.number,
        was_created: true,
    })
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

/// Generate PR body with stack navigation links.
fn generate_pr_body(branches: &[StackBranch], current_idx: usize) -> String {
    let mut body = String::from("## Stack\n\n");

    for (i, branch) in branches.iter().enumerate() {
        let marker = if i == current_idx { "→" } else { " " };

        if let Some(pr_num) = branch.pr {
            let _ = writeln!(body, "{marker} #{pr_num} - `{}`", branch.name);
        } else {
            let _ = writeln!(body, "{marker} (pending) - `{}`", branch.name);
        }
    }

    body.push_str("\n---\n");
    body.push_str("*Managed by [rung](https://github.com/auswm85/rung)*\n");

    body
}
