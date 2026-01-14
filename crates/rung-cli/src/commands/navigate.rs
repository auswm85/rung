//! `rung nxt` and `rung prv` commands - Navigate the stack.

use anyhow::{bail, Context, Result};
use rung_core::State;
use rung_git::Repository;

use crate::output;

/// Navigate to the next (child) branch in the stack.
pub fn run_next() -> Result<()> {
    let (repo, state) = open_repo_and_state()?;

    let current = repo.current_branch()?;
    let stack = state.load_stack()?;

    // Find children of current branch
    let children = stack.children_of(&current);

    match children.len() {
        0 => {
            output::info(&format!("'{}' has no children in the stack", current));
            Ok(())
        }
        1 => {
            let child = &children[0].name;
            repo.checkout(child)?;
            output::success(&format!("Switched to '{}'", child));
            Ok(())
        }
        _ => {
            output::warn(&format!(
                "'{}' has multiple children. Choose one:",
                current
            ));
            for child in children {
                println!("  → {}", child.name);
            }
            bail!("Use `git checkout <branch>` to switch to the desired branch");
        }
    }
}

/// Navigate to the previous (parent) branch in the stack.
pub fn run_prev() -> Result<()> {
    let (repo, state) = open_repo_and_state()?;

    let current = repo.current_branch()?;
    let stack = state.load_stack()?;

    // Find current branch in stack
    let branch = stack.find_branch(&current);

    match branch.and_then(|b| b.parent.as_ref()) {
        Some(parent) => {
            repo.checkout(parent)?;
            output::success(&format!("Switched to '{}'", parent));
            Ok(())
        }
        None => {
            output::info(&format!(
                "'{}' has no parent in the stack (it's a root branch)",
                current
            ));
            Ok(())
        }
    }
}

/// Helper to open repo and state.
fn open_repo_and_state() -> Result<(Repository, State)> {
    let repo = Repository::open_current().context("Not inside a git repository")?;
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    if !state.is_initialized() {
        bail!("Rung not initialized - run `rung init` first");
    }

    Ok((repo, state))
}
