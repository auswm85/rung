//! `rung submit` command - Push branches and create/update PRs.

use std::fmt::Write;

use anyhow::{Context, Result, bail};
use rung_core::State;
use rung_core::stack::StackBranch;
use rung_git::Repository;
use rung_github::{
    Auth, CreateComment, CreatePullRequest, GitHubClient, UpdateComment, UpdatePullRequest,
};
use serde::Serialize;

use crate::output;

/// JSON output for submit command.
#[derive(Debug, Serialize)]
struct SubmitOutput {
    prs_created: usize,
    prs_updated: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    branches: Vec<BranchSubmitInfo>,
}

/// Information about a submitted branch.
#[derive(Debug, Serialize)]
struct BranchSubmitInfo {
    branch: String,
    pr_number: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pr_url: Option<String>,
    action: SubmitAction,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum SubmitAction {
    Created,
    Updated,
}

/// Configuration options for the submit command.
struct SubmitConfig<'a> {
    /// Output as JSON.
    json: bool,
    /// Create PRs as drafts.
    draft: bool,
    /// Force push branches.
    force: bool,
    /// Custom title for the current branch's PR.
    custom_title: Option<&'a str>,
    /// Current branch name (for custom title matching).
    current_branch: Option<String>,
}

/// Context for GitHub API operations.
struct GitHubContext<'a> {
    client: &'a GitHubClient,
    rt: &'a tokio::runtime::Runtime,
    owner: &'a str,
    repo_name: &'a str,
}

/// Run the submit command.
pub fn run(json: bool, draft: bool, force: bool, custom_title: Option<&str>) -> Result<()> {
    let (repo, state, mut stack) = setup_submit()?;

    if stack.is_empty() {
        if json {
            return output_json(&SubmitOutput {
                prs_created: 0,
                prs_updated: 0,
                branches: vec![],
            });
        }
        output::info("No branches in stack - nothing to submit");
        return Ok(());
    }

    let config = SubmitConfig {
        json,
        draft,
        force,
        custom_title,
        current_branch: repo.current_branch().ok(),
    };

    let (owner, repo_name) = get_remote_info(&repo)?;
    if !json {
        output::info(&format!("Submitting to {owner}/{repo_name}..."));
    }

    let client = GitHubClient::new(&Auth::auto()).context("Failed to authenticate with GitHub")?;
    let rt = tokio::runtime::Runtime::new()?;

    let gh = GitHubContext {
        client: &client,
        rt: &rt,
        owner: &owner,
        repo_name: &repo_name,
    };

    let (created, updated, branch_infos) = process_branches(&repo, &gh, &mut stack, &config)?;

    state.save_stack(&stack)?;

    // Update stack comments on all PRs
    update_stack_comments(&gh, &stack.branches, json)?;

    if json {
        return output_json(&SubmitOutput {
            prs_created: created,
            prs_updated: updated,
            branches: branch_infos,
        });
    }

    print_summary(created, updated);

    // Output PR URLs for piping (essential output, not suppressed by --quiet)
    for info in &branch_infos {
        if let Some(url) = &info.pr_url {
            output::essential(url);
        }
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

/// Process all branches in the stack.
fn process_branches(
    repo: &Repository,
    gh: &GitHubContext<'_>,
    stack: &mut rung_core::stack::Stack,
    config: &SubmitConfig<'_>,
) -> Result<(usize, usize, Vec<BranchSubmitInfo>)> {
    let mut created = 0;
    let mut updated = 0;
    let mut branch_infos = Vec::new();

    for i in 0..stack.branches.len() {
        let branch = &stack.branches[i];
        let branch_name = branch.name.clone();
        let parent_name = branch.parent.clone();
        let existing_pr = branch.pr;

        if !config.json {
            output::info(&format!("Processing {branch_name}..."));
            output::info(&format!("  Pushing {branch_name}..."));
        }

        // Push the branch
        repo.push(&branch_name, config.force)
            .with_context(|| format!("Failed to push {branch_name}"))?;

        let base_branch = parent_name.as_deref().unwrap_or("main");

        // Get title and body from commit message, with custom title override for current branch
        let (mut title, body) = get_pr_title_and_body(repo, &branch_name);
        if config.current_branch.as_deref() == Some(branch_name.as_str()) {
            if let Some(custom) = config.custom_title {
                title = custom.to_string();
            }
        }

        if let Some(pr_number) = existing_pr {
            update_existing_pr(gh, pr_number, base_branch, config.json)?;
            updated += 1;
            branch_infos.push(BranchSubmitInfo {
                branch: branch_name.to_string(),
                pr_number,
                pr_url: Some(format!(
                    "https://github.com/{}/{}/pull/{pr_number}",
                    gh.owner, gh.repo_name
                )),
                action: SubmitAction::Updated,
            });
        } else {
            let result = create_or_find_pr(
                gh,
                &branch_name,
                base_branch,
                title,
                body,
                config.draft,
                config.json,
            )?;

            stack.branches[i].pr = Some(result.pr_number);
            let action = if result.was_created {
                created += 1;
                SubmitAction::Created
            } else {
                updated += 1;
                SubmitAction::Updated
            };
            branch_infos.push(BranchSubmitInfo {
                branch: branch_name.to_string(),
                pr_number: result.pr_number,
                pr_url: result.pr_url,
                action,
            });
        }
    }

    Ok((created, updated, branch_infos))
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

/// Result of creating or finding a PR.
struct PrResult {
    pr_number: u64,
    was_created: bool,
    pr_url: Option<String>,
}

/// Create a new PR or find an existing one.
fn create_or_find_pr(
    gh: &GitHubContext<'_>,
    branch_name: &str,
    base_branch: &str,
    title: String,
    body: String,
    draft: bool,
    json: bool,
) -> Result<PrResult> {
    // Check if PR already exists for this branch
    let existing = gh
        .rt
        .block_on(
            gh.client
                .find_pr_for_branch(gh.owner, gh.repo_name, branch_name),
        )
        .context("Failed to check for existing PR")?;

    if let Some(pr) = existing {
        if !json {
            output::info(&format!("  Found existing PR #{}...", pr.number));
        }

        // Only update base branch, preserve existing description
        let update = UpdatePullRequest {
            title: None,
            body: None,
            base: Some(base_branch.to_string()),
        };

        gh.rt
            .block_on(
                gh.client
                    .update_pr(gh.owner, gh.repo_name, pr.number, update),
            )
            .with_context(|| format!("Failed to update PR #{}", pr.number))?;

        return Ok(PrResult {
            pr_number: pr.number,
            was_created: false,
            pr_url: Some(pr.html_url),
        });
    }

    // Create new PR
    if !json {
        output::info(&format!("  Creating PR ({branch_name} → {base_branch})..."));
    }

    let create = CreatePullRequest {
        title,
        body,
        head: branch_name.to_string(),
        base: base_branch.to_string(),
        draft,
    };

    let pr = gh
        .rt
        .block_on(gh.client.create_pr(gh.owner, gh.repo_name, create))
        .with_context(|| format!("Failed to create PR for {branch_name}"))?;

    if !json {
        output::success(&format!("  Created PR #{}: {}", pr.number, pr.html_url));
    }

    Ok(PrResult {
        pr_number: pr.number,
        was_created: true,
        pr_url: Some(pr.html_url),
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

/// Marker to identify rung stack comments.
const STACK_COMMENT_MARKER: &str = "<!-- rung-stack -->";

/// Generate stack comment for a PR.
fn generate_stack_comment(branches: &[StackBranch], current_pr: u64) -> String {
    let mut comment = String::from(STACK_COMMENT_MARKER);
    comment.push_str("\n### Stack\n\n");

    // Find the current branch
    let current_branch = branches.iter().find(|b| b.pr == Some(current_pr));
    let current_name = current_branch.map_or("", |b| b.name.as_str());

    // Build the chain for this branch
    let chain = build_branch_chain(branches, current_name);

    // Build stack list in markdown format (newest at top, so iterate in reverse)
    for branch_name in chain.iter().rev() {
        let branch = branches.iter().find(|b| &b.name == branch_name);
        let is_current = branch_name == current_name;

        if let Some(b) = branch {
            let pointer = if is_current { " 👈" } else { "" };

            if let Some(pr_num) = b.pr {
                let title = generate_title(&b.name);
                if is_current {
                    let _ = writeln!(comment, "- **{title}** #{pr_num}{pointer}");
                } else {
                    let _ = writeln!(comment, "- {title} #{pr_num}");
                }
            } else {
                let _ = writeln!(comment, "- *(pending)* `{branch_name}`{pointer}");
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
                        return Some(parent.as_str());
                    }
                } else {
                    return Some("main");
                }
            }
        })
        .unwrap_or("main");

    let _ = writeln!(comment, "- `{base}`");
    comment.push_str("\n---\n*Managed by [rung](https://github.com/auswm85/rung)*");

    comment
}

/// Update stack comments on all PRs in the stack.
fn update_stack_comments(
    gh: &GitHubContext<'_>,
    branches: &[StackBranch],
    json: bool,
) -> Result<()> {
    if !json {
        output::info("Updating stack comments...");
    }

    for branch in branches {
        let Some(pr_number) = branch.pr else {
            continue;
        };

        let comment_body = generate_stack_comment(branches, pr_number);

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
fn build_branch_chain(branches: &[StackBranch], current_name: &str) -> Vec<String> {
    // Find all ancestors (walk up the parent chain)
    let mut ancestors: Vec<String> = vec![];
    let mut current = current_name.to_string();

    loop {
        if let Some(branch) = branches.iter().find(|b| b.name == current) {
            if let Some(ref parent) = branch.parent {
                // Check if parent is in the stack
                if branches.iter().any(|b| b.name == *parent) {
                    ancestors.push(parent.to_string());
                    current = parent.to_string();
                    continue;
                }
            }
        }
        break;
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
