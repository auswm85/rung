//! Forge client dispatch.
//!
//! [`ForgeApi`] uses `impl Future` return types and is therefore not
//! dyn-compatible. The [`Forge`] enum provides static dispatch across the
//! supported forge backends, selected from a git remote URL via
//! [`rung_forge::ForgeKind::detect`]. Adding a backend means adding a variant
//! here — call sites stay backend-agnostic.

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use rung_forge::{
    CheckRun, CreateComment, CreatePullRequest, ForgeApi, ForgeKind, IssueComment,
    MergePullRequest, MergeResult, PullRequest, RepoId, Result as ForgeResult, UpdateComment,
    UpdatePullRequest,
};
use rung_github::{Auth, GitHubClient};

/// A forge client, statically dispatched by backend kind.
pub enum Forge {
    /// GitHub backend.
    GitHub(GitHubClient),
}

impl Forge {
    /// Build a forge client for a git remote, dispatching on the detected forge.
    ///
    /// # Errors
    /// Returns an error if the remote is not a recognized forge, or if
    /// authentication for the detected forge fails.
    pub fn for_remote(remote_url: &str, auth: &Auth) -> Result<Self> {
        match ForgeKind::detect(remote_url) {
            Some(kind @ ForgeKind::GitHub) => {
                let client = GitHubClient::new(auth).with_context(|| {
                    format!(
                        "Failed to authenticate with {} - {}",
                        kind.display_name(),
                        kind.auth_hint()
                    )
                })?;
                Ok(Self::GitHub(client))
            }
            None => Err(anyhow!(
                "unsupported forge: remote is not a recognized forge repository (supported: {})",
                ForgeKind::supported_label()
            )),
        }
    }
}

// `GitHubClient` has inherent `(owner, repo, …)` methods that shadow the
// trait's `(&RepoId, …)` methods under normal method-call resolution, so each
// arm dispatches through `ForgeApi` explicitly to reach the trait impl.
impl ForgeApi for Forge {
    async fn get_pr(&self, repo: &RepoId, number: u64) -> ForgeResult<PullRequest> {
        match self {
            Self::GitHub(c) => ForgeApi::get_pr(c, repo, number).await,
        }
    }

    async fn get_prs_batch(
        &self,
        repo: &RepoId,
        numbers: &[u64],
    ) -> ForgeResult<HashMap<u64, PullRequest>> {
        match self {
            Self::GitHub(c) => ForgeApi::get_prs_batch(c, repo, numbers).await,
        }
    }

    async fn find_pr_for_branch(
        &self,
        repo: &RepoId,
        branch: &str,
    ) -> ForgeResult<Option<PullRequest>> {
        match self {
            Self::GitHub(c) => ForgeApi::find_pr_for_branch(c, repo, branch).await,
        }
    }

    async fn create_pr(&self, repo: &RepoId, pr: CreatePullRequest) -> ForgeResult<PullRequest> {
        match self {
            Self::GitHub(c) => ForgeApi::create_pr(c, repo, pr).await,
        }
    }

    async fn update_pr(
        &self,
        repo: &RepoId,
        number: u64,
        update: UpdatePullRequest,
    ) -> ForgeResult<PullRequest> {
        match self {
            Self::GitHub(c) => ForgeApi::update_pr(c, repo, number, update).await,
        }
    }

    async fn get_check_runs(&self, repo: &RepoId, commit_sha: &str) -> ForgeResult<Vec<CheckRun>> {
        match self {
            Self::GitHub(c) => ForgeApi::get_check_runs(c, repo, commit_sha).await,
        }
    }

    async fn merge_pr(
        &self,
        repo: &RepoId,
        number: u64,
        merge: MergePullRequest,
    ) -> ForgeResult<MergeResult> {
        match self {
            Self::GitHub(c) => ForgeApi::merge_pr(c, repo, number, merge).await,
        }
    }

    async fn delete_ref(&self, repo: &RepoId, ref_name: &str) -> ForgeResult<()> {
        match self {
            Self::GitHub(c) => ForgeApi::delete_ref(c, repo, ref_name).await,
        }
    }

    async fn get_default_branch(&self, repo: &RepoId) -> ForgeResult<String> {
        match self {
            Self::GitHub(c) => ForgeApi::get_default_branch(c, repo).await,
        }
    }

    async fn list_pr_comments(
        &self,
        repo: &RepoId,
        pr_number: u64,
    ) -> ForgeResult<Vec<IssueComment>> {
        match self {
            Self::GitHub(c) => ForgeApi::list_pr_comments(c, repo, pr_number).await,
        }
    }

    async fn create_pr_comment(
        &self,
        repo: &RepoId,
        pr_number: u64,
        comment: CreateComment,
    ) -> ForgeResult<IssueComment> {
        match self {
            Self::GitHub(c) => ForgeApi::create_pr_comment(c, repo, pr_number, comment).await,
        }
    }

    async fn update_pr_comment(
        &self,
        repo: &RepoId,
        comment_id: u64,
        comment: UpdateComment,
    ) -> ForgeResult<IssueComment> {
        match self {
            Self::GitHub(c) => ForgeApi::update_pr_comment(c, repo, comment_id, comment).await,
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use rung_github::SecretString;

    fn test_auth() -> Auth {
        Auth::Token(SecretString::from("test_token"))
    }

    #[test]
    fn test_for_remote_github_https() {
        let forge = Forge::for_remote("https://github.com/octocat/hello-world.git", &test_auth())
            .expect("github remote should resolve");
        assert!(matches!(forge, Forge::GitHub(_)));
    }

    #[test]
    fn test_for_remote_github_ssh() {
        let forge = Forge::for_remote("git@github.com:octocat/hello-world.git", &test_auth())
            .expect("github ssh remote should resolve");
        assert!(matches!(forge, Forge::GitHub(_)));
    }

    #[test]
    fn test_for_remote_unsupported_forge_errors() {
        // A recognizable-but-unsupported forge must not silently fall back to GitHub.
        assert!(Forge::for_remote("https://gitlab.com/owner/repo.git", &test_auth()).is_err());
        assert!(Forge::for_remote("not a url", &test_auth()).is_err());
    }
}
