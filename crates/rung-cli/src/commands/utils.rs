use anyhow::{Context, Result, bail};
use rung_core::State;
use rung_git::Repository;

use crate::output;

/// Helper to open repo and state.
pub fn open_repo_and_state() -> Result<(Repository, State)> {
    let repo = Repository::open_current().context("Not inside a git repository")?;
    let workdir = repo.workdir().context("Cannot run in bare repository")?;
    let state = State::new(workdir)?;

    if !state.is_initialized() {
        bail!("Rung not initialized - run `rung init` first");
    }

    Ok((repo, state))
}

/// Ensure the repository is not in detached HEAD state.
/// If detached, prints the detached-HEAD error message and returns an error.
pub fn ensure_on_branch(repo: &Repository) -> Result<()> {
    if repo.head_detached()? {
        output::error_detached_head();
        bail!("");
    }
    Ok(())
}
