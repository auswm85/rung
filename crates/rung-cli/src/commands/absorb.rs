//! `rung absorb` command - Absorb staged changes into appropriate commits.

use anyhow::{Context, Result, bail};
use rung_core::State;
use rung_core::absorb::{self, UnmapReason};
use rung_git::Repository;
use rung_github::{Auth, GitHubClient};

use crate::output;

/// Run the absorb command.
pub fn run(dry_run: bool, base: Option<&str>) -> Result<()> {
    // Open repository
    let repo = Repository::open_current().context("Not inside a git repository")?;

    // Get state manager
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    // Ensure initialized
    if !state.is_initialized() {
        bail!("Rung not initialized - run `rung init` first");
    }

    // Check for staged changes
    if !repo.has_staged_changes()? {
        bail!("No staged changes to absorb. Stage changes with `git add` first.");
    }

    // Determine base branch
    let base_branch = if let Some(b) = base {
        b.to_string()
    } else {
        detect_base_branch(&repo)?
    };

    // Create absorb plan
    let plan = absorb::create_absorb_plan(&repo, &state, &base_branch)?;

    if plan.actions.is_empty() && plan.unmapped.is_empty() {
        output::info("Staged changes present but no absorbable hunks found");
        return Ok(());
    }

    // Report unmapped hunks
    if !plan.unmapped.is_empty() {
        output::warn(&format!(
            "{} hunk(s) could not be absorbed:",
            plan.unmapped.len()
        ));
        for unmapped in &plan.unmapped {
            let reason = match &unmapped.reason {
                UnmapReason::NewFile => "new file".to_string(),
                UnmapReason::InsertOnly => "insert-only (no lines to blame)".to_string(),
                UnmapReason::MultipleCommits => "multiple commits touched these lines".to_string(),
                UnmapReason::CommitNotInStack => "target commit not in stack".to_string(),
                UnmapReason::CommitOnBaseBranch => {
                    "target commit already on base branch".to_string()
                }
                UnmapReason::BlameError(e) => format!("blame error: {e}"),
            };
            output::detail(&format!("  {} ({})", unmapped.hunk.file_path, reason));
        }
        output::detail("");
    }

    if plan.actions.is_empty() {
        if !plan.unmapped.is_empty() {
            bail!("All staged hunks could not be mapped to target commits");
        }
        return Ok(());
    }

    // Show what will be absorbed
    output::info(&format!("{} hunk(s) will be absorbed:", plan.actions.len()));

    // Group by target commit for cleaner output
    let mut by_target: std::collections::HashMap<String, Vec<&absorb::AbsorbAction>> =
        std::collections::HashMap::new();
    for action in &plan.actions {
        let key = action.target_commit.to_string();
        by_target.entry(key).or_default().push(action);
    }

    for (commit_sha, actions) in &by_target {
        let short_sha = &commit_sha[..8.min(commit_sha.len())];
        let message = &actions[0].target_message;
        output::detail(&format!(
            "  {} {} ({} hunk(s))",
            short_sha,
            message,
            actions.len()
        ));
        for action in actions {
            output::detail(&format!("    → {}", action.hunk.file_path));
        }
    }

    if dry_run {
        output::info("Dry run - no changes made");
        return Ok(());
    }

    // Execute the absorb
    let result = absorb::execute_absorb(&repo, &plan)?;

    output::success(&format!(
        "Created {} fixup commit(s)",
        result.fixups_created
    ));

    if result.fixups_created > 0 {
        output::info("Run `git rebase -i --autosquash` to apply the fixups");
    }

    Ok(())
}

/// Auto-detect the base branch by querying GitHub for the default branch.
fn detect_base_branch(repo: &Repository) -> Result<String> {
    let origin_url = repo.origin_url().context("No origin remote configured")?;
    let (owner, repo_name) = Repository::parse_github_remote(&origin_url)
        .context("Could not parse GitHub remote URL")?;

    let client = GitHubClient::new(&Auth::auto()).context(
        "GitHub auth required to detect default branch. Use --base <branch> to specify manually.",
    )?;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client.get_default_branch(&owner, &repo_name))
        .context("Could not fetch default branch. Use --base <branch> to specify manually.")
}
