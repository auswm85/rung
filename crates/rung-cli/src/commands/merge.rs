//! `rung merge` command - Merge PR and clean up stack.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use rung_core::State;
use rung_core::stack::Stack;
use rung_git::{Oid, Repository};
use rung_github::{Auth, GitHubClient, MergeMethod};
use serde::Serialize;

use crate::commands::utils;
use crate::output;
use crate::services::MergeService;

/// JSON output for merge command.
#[derive(Debug, Serialize)]
struct MergeOutput {
    merged_branch: String,
    pr_number: u64,
    merge_method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    checked_out: Option<String>,
    descendants_rebased: usize,
}

/// Context gathered during merge setup.
struct MergeContext {
    current_branch: String,
    pr_number: u64,
    stack_parent_branch: Option<String>,
    owner: String,
    repo_name: String,
    descendants: Vec<String>,
    old_commits: HashMap<String, Oid>,
}

/// Parse merge method from string.
fn parse_merge_method(method: &str) -> Result<MergeMethod> {
    match method.to_lowercase().as_str() {
        "squash" => Ok(MergeMethod::Squash),
        "merge" => Ok(MergeMethod::Merge),
        "rebase" => Ok(MergeMethod::Rebase),
        _ => bail!("Invalid merge method: {method}. Use squash, merge, or rebase."),
    }
}

/// Set up merge context: validate state and gather required info.
fn setup_merge_context(repo: &Repository, state: &State) -> Result<(MergeContext, Stack)> {
    if !state.is_initialized() {
        bail!("Rung not initialized - run `rung init` first");
    }

    utils::ensure_on_branch(repo)?;
    let current_branch = repo.current_branch()?;

    let stack = state.load_stack()?;
    let branch = stack
        .find_branch(&current_branch)
        .ok_or_else(|| anyhow::anyhow!("Branch '{current_branch}' not in stack"))?;

    let pr_number = branch.pr.ok_or_else(|| {
        anyhow::anyhow!("No PR associated with branch '{current_branch}'. Run `rung submit` first.")
    })?;

    let stack_parent_branch = branch.parent.as_ref().map(ToString::to_string);

    let origin_url = repo.origin_url()?;
    let (owner, repo_name) = Repository::parse_github_remote(&origin_url)?;

    let descendants =
        MergeService::<Repository, GitHubClient>::collect_descendants(&stack, &current_branch);

    // Capture old commits before any rebasing (needed for --onto)
    let mut old_commits: HashMap<String, Oid> = HashMap::new();
    old_commits.insert(current_branch.clone(), repo.branch_commit(&current_branch)?);
    for branch_name in &descendants {
        old_commits.insert(branch_name.clone(), repo.branch_commit(branch_name)?);
    }

    Ok((
        MergeContext {
            current_branch,
            pr_number,
            stack_parent_branch,
            owner,
            repo_name,
            descendants,
            old_commits,
        },
        stack,
    ))
}

/// Clean up local state after merge: checkout parent, delete local branch, pull.
/// Checkout failures are non-fatal since the merge itself succeeded.
fn cleanup_after_merge(
    repo: &Repository,
    current_branch: &str,
    parent_branch: &str,
    json: bool,
) -> Option<String> {
    // Checkout is non-fatal - the merge succeeded, so we continue with cleanup
    let checked_out = if let Err(e) = repo.checkout(parent_branch) {
        if !json {
            output::warn(&format!("Could not checkout '{parent_branch}': {e}"));
            output::info("You may need to manually checkout the desired branch.");
        }
        None
    } else {
        Some(parent_branch.to_string())
    };

    if let Err(e) = repo.delete_branch(current_branch) {
        if !json {
            output::warn(&format!("Could not delete local branch: {e}"));
        }
    } else if !json {
        output::info(&format!("Deleted local branch '{current_branch}'"));
    }

    if let Err(e) = repo.pull_ff()
        && !json
    {
        output::warn(&format!("Could not pull latest {parent_branch}: {e}"));
    }

    checked_out
}

/// Run the merge command.
pub fn run(json: bool, method: &str, no_delete: bool) -> Result<()> {
    let merge_method = parse_merge_method(method)?;

    let repo = Repository::open_current().context("Not inside a git repository")?;
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    let (ctx, stack) = setup_merge_context(&repo, &state)?;

    if !json {
        output::info(&format!(
            "Merging PR #{} for {}...",
            ctx.pr_number, ctx.current_branch
        ));
    }

    let rt = tokio::runtime::Runtime::new()?;
    let (parent_branch, descendants_rebased) = rt.block_on(execute_merge(
        &repo,
        &state,
        &stack,
        &ctx,
        merge_method,
        no_delete,
        json,
    ))?;

    let checked_out = cleanup_after_merge(&repo, &ctx.current_branch, &parent_branch, json);

    if json {
        return output_json(&MergeOutput {
            merged_branch: ctx.current_branch,
            pr_number: ctx.pr_number,
            merge_method: method.to_string(),
            checked_out,
            descendants_rebased,
        });
    }

    if checked_out.is_some() {
        output::info(&format!("Checked out '{parent_branch}'"));
    }
    output::success("Merge complete!");

    Ok(())
}

/// Execute the GitHub merge operation.
/// Returns (`parent_branch`, `descendants_rebased_count`).
#[allow(clippy::too_many_arguments, clippy::future_not_send)]
async fn execute_merge(
    repo: &Repository,
    state: &State,
    stack: &Stack,
    ctx: &MergeContext,
    merge_method: MergeMethod,
    no_delete: bool,
    json: bool,
) -> Result<(String, usize)> {
    let auth = Auth::auto();
    let client = GitHubClient::new(&auth)?;
    let service = MergeService::new(repo, &client, ctx.owner.clone(), ctx.repo_name.clone());

    // Step 1: Validate PR is mergeable
    let pr = service.validate_mergeable(ctx.pr_number).await?;

    let parent_branch = ctx
        .stack_parent_branch
        .clone()
        .unwrap_or_else(|| pr.base_branch.clone());

    // Step 2: Shift child PR bases before merge
    print_child_relinks(stack, ctx, &parent_branch, json);

    let shifted_prs = service
        .shift_child_pr_bases(stack, &ctx.current_branch, &parent_branch, &ctx.descendants)
        .await?;

    // Step 3: Merge the PR
    if let Err(merge_err) = service.merge_pr(ctx.pr_number, merge_method).await {
        rollback_on_failure(&service, &shifted_prs, json).await;
        return Err(merge_err);
    }

    if !json {
        output::success(&format!("Merged PR #{}", ctx.pr_number));
    }

    // NOTE: After merge_pr succeeds, the PR is merged on GitHub.
    // Subsequent failures should NOT abort - we log warnings and continue.

    // Step 4: Update stack after merge (non-fatal after merge)
    match service.update_stack_after_merge(state, &ctx.current_branch, &parent_branch) {
        Ok(children_count) => {
            if !json && children_count > 0 {
                output::info(&format!(
                    "Re-parented {children_count} child branch(es) to '{parent_branch}'"
                ));
            }
        }
        Err(e) => {
            if !json {
                output::error(&format!("Failed to update stack after merge: {e}"));
                output::warn(
                    "PR was merged successfully, but local stack state may be inconsistent.",
                );
                output::info("To fix, run: rung sync");
            }
        }
    }

    // Step 5: Rebase descendants (non-fatal after merge)
    // rebase_descendants_after_merge already handles its own error messaging
    // We explicitly match but don't propagate since the merge itself succeeded
    let descendants_rebased =
        rebase_descendants_after_merge(&service, state, stack, ctx, &parent_branch, json)
            .await
            .unwrap_or(0);

    // Step 6: Delete remote branch
    if !no_delete {
        delete_remote_branch(&service, &ctx.current_branch, json).await;
    }

    Ok((parent_branch, descendants_rebased))
}

/// Print info about child PR relinking.
fn print_child_relinks(stack: &Stack, ctx: &MergeContext, parent_branch: &str, json: bool) {
    if json || ctx.descendants.is_empty() {
        return;
    }

    for branch_name in &ctx.descendants {
        if let Some(branch_info) = stack.find_branch(branch_name) {
            let stack_parent = branch_info
                .parent
                .as_ref()
                .map_or(parent_branch, |p| p.as_str());
            if stack_parent == ctx.current_branch
                && let Some(child_pr_num) = branch_info.pr
            {
                output::info(&format!(
                    "Relinking PR #{child_pr_num} to '{parent_branch}' before merge..."
                ));
            }
        }
    }
}

/// Rollback PR base changes on merge failure.
#[allow(clippy::future_not_send)]
async fn rollback_on_failure(
    service: &MergeService<'_, Repository, GitHubClient>,
    shifted_prs: &[(u64, String)],
    json: bool,
) {
    if shifted_prs.is_empty() {
        return;
    }

    if !json {
        output::warn("Merge failed, rolling back PR base changes...");
    }
    let failures = service.rollback_pr_bases(shifted_prs).await;
    if !json {
        for (child_pr_num, original_base) in shifted_prs {
            // Check if this PR failed to rollback
            if let Some((_, err)) = failures.iter().find(|(pr, _)| pr == child_pr_num) {
                output::warn(&format!(
                    "  Failed to restore PR #{child_pr_num} base to '{original_base}': {err}"
                ));
            } else {
                output::info(&format!(
                    "  Restored PR #{child_pr_num} base to '{original_base}'"
                ));
            }
        }
    }
}

/// Rebase descendants after a successful merge.
/// Returns the count of successfully rebased descendants.
#[allow(clippy::future_not_send)]
async fn rebase_descendants_after_merge(
    service: &MergeService<'_, Repository, GitHubClient>,
    state: &State,
    stack: &Stack,
    ctx: &MergeContext,
    parent_branch: &str,
    json: bool,
) -> Result<usize> {
    if ctx.descendants.is_empty() {
        return Ok(0);
    }

    let results = service
        .rebase_descendants(
            state,
            stack,
            &ctx.current_branch,
            parent_branch,
            &ctx.descendants,
            &ctx.old_commits,
        )
        .await;

    match results {
        Ok(results) => {
            let success_count = if json {
                results.iter().filter(|r| r.rebased).count()
            } else {
                let mut count = 0;
                for result in &results {
                    if result.rebased {
                        output::info(&format!("  Rebased and pushed {}", result.branch));
                        count += 1;
                    } else if let Some(err) = &result.error {
                        output::warn(&format!("  Failed to rebase {}: {err}", result.branch));
                    }
                    if result.pr_updated {
                        output::info(&format!("  Updated PR base for {}", result.branch));
                    }
                }
                count
            };
            Ok(success_count)
        }
        Err(e) => {
            if !json {
                output::error(&format!("Merged parent, but descendant has conflicts: {e}"));
                output::warn("Manual intervention required. After resolving conflicts:");
                output::info("  1. git rebase --continue");
                output::info("  2. git push --force-with-lease");
                output::info("  3. rung sync");
                output::info("");
                output::info("Note: 'rung sync' will rebase any remaining descendant branches.");
            }
            Err(e)
        }
    }
}

/// Delete remote branch after merge.
#[allow(clippy::future_not_send)]
async fn delete_remote_branch(
    service: &MergeService<'_, Repository, GitHubClient>,
    branch: &str,
    json: bool,
) {
    match service.delete_remote_branch(branch).await {
        Ok(()) => {
            if !json {
                output::info(&format!("Deleted remote branch '{branch}'"));
            }
        }
        Err(e) => {
            if !json {
                output::warn(&format!("Failed to delete remote branch: {e}"));
            }
        }
    }
}

/// Output merge result as JSON.
fn output_json(output: &MergeOutput) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(output)?);
    Ok(())
}
