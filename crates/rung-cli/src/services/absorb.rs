//! Absorb service for determining base branches and executing absorb operations.
//!
//! This module handles base branch detection and orchestrates the absorb
//! workflow, separated from CLI presentation concerns.

use anyhow::{Context, Result};
use rung_core::StateStore;
use rung_core::absorb::{self, AbsorbPlan, AbsorbResult};
use rung_git::{AbsorbOps, Repository};
use rung_github::{Auth, GitHubClient};

/// Service for absorb operations with trait-based dependencies.
pub struct AbsorbService<'a, G: AbsorbOps> {
    repo: &'a G,
}

impl<'a, G: AbsorbOps> AbsorbService<'a, G> {
    /// Create a new absorb service.
    #[must_use]
    pub const fn new(repo: &'a G) -> Self {
        Self { repo }
    }

    /// Check if there are staged changes to absorb.
    pub fn has_staged_changes(&self) -> Result<bool> {
        Ok(self.repo.has_staged_changes()?)
    }

    /// Detect the base branch by querying GitHub for the default branch.
    #[allow(clippy::future_not_send)] // Git operations are sync; future doesn't need to be Send
    pub async fn detect_base_branch(&self) -> Result<String> {
        let origin_url = self
            .repo
            .origin_url()
            .context("No origin remote configured")?;
        let (owner, repo_name) = Repository::parse_github_remote(&origin_url)
            .context("Could not parse GitHub remote URL")?;

        let client = GitHubClient::new(&Auth::auto()).context(
            "GitHub auth required to detect default branch. Use --base <branch> to specify manually.",
        )?;
        client
            .get_default_branch(&owner, &repo_name)
            .await
            .context("Could not fetch default branch. Use --base <branch> to specify manually.")
    }

    /// Create an absorb plan for the given base branch.
    pub fn create_plan<S: StateStore>(&self, state: &S, base_branch: &str) -> Result<AbsorbPlan> {
        Ok(absorb::create_absorb_plan(self.repo, state, base_branch)?)
    }

    /// Execute an absorb plan.
    pub fn execute_plan(&self, plan: &AbsorbPlan) -> Result<AbsorbResult> {
        Ok(absorb::execute_absorb(self.repo, plan)?)
    }
}

#[cfg(test)]
mod tests {
    // Integration tests would require a real git repository with staged changes
    // Unit tests are limited since the service wraps external operations
}
