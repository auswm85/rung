//! `rung init` command - Initialize rung in the current repository.

use anyhow::{Context, Result};
use rung_core::{Config, State};
use rung_git::Repository;

use crate::output;

/// Run the init command.
pub fn run() -> Result<()> {
    // Open repository
    let repo = Repository::open_current().context("Not inside a git repository")?;

    // Get state manager
    let workdir = repo
        .workdir()
        .context("Cannot initialize in bare repository")?;
    let state = State::new(workdir)?;

    // Check if already initialized
    if state.is_initialized() {
        output::warn("Rung is already initialized in this repository");
        return Ok(());
    }

    // Initialize
    state.init()?;

    // Detect and save default branch
    if let Some(branch) = repo.detect_default_branch() {
        let mut config = Config::default();
        config.general.default_branch = Some(branch.clone());
        state.save_config(&config)?;
        output::info(&format!("Detected default branch: {branch}"));
    } else {
        output::info("Could not detect default branch, using \"main\" as fallback");
    }

    output::success("Initialized rung in this repository");
    output::info(&format!("State stored in: {}", state.rung_dir().display()));

    Ok(())
}
