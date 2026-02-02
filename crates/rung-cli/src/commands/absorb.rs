//! `rung absorb` command - Absorb staged changes into appropriate commits.

use anyhow::{Context, Result, bail};
use rung_core::State;
use rung_core::absorb::{AbsorbAction, UnmapReason};
use rung_git::Repository;
use std::collections::HashMap;

use crate::commands::utils;
use crate::output;
use crate::services::AbsorbService;

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

    // Ensure on branch
    utils::ensure_on_branch(&repo)?;

    // Create service
    let service = AbsorbService::new(&repo);

    // Check for staged changes
    if !service.has_staged_changes()? {
        bail!("No staged changes to absorb. Stage changes with `git add` first.");
    }

    // Determine base branch
    let base_branch = if let Some(b) = base {
        b.to_string()
    } else {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(service.detect_base_branch())?
    };

    // Create absorb plan
    let plan = service.create_plan(&state, &base_branch)?;

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
    print_absorb_plan(&plan.actions);

    if dry_run {
        output::info("Dry run - no changes made");
        return Ok(());
    }

    // Execute the absorb
    let result = service.execute_plan(&plan)?;

    output::success(&format!(
        "Created {} fixup commit(s)",
        result.fixups_created
    ));

    if result.fixups_created > 0 {
        output::info("Run `git rebase -i --autosquash` to apply the fixups");
    }

    Ok(())
}

/// Print the absorb plan grouped by target commit.
fn print_absorb_plan(actions: &[AbsorbAction]) {
    let mut by_target: HashMap<String, Vec<&AbsorbAction>> = HashMap::new();
    for action in actions {
        let key = action.target_commit.to_string();
        by_target.entry(key).or_default().push(action);
    }

    // Sort by commit SHA for deterministic output
    let mut commit_shas: Vec<_> = by_target.keys().collect();
    commit_shas.sort();

    for commit_sha in commit_shas {
        let actions = &by_target[commit_sha];
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
}
