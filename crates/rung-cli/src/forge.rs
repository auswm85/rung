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
    MergePullRequest, MergeResult, PullRequest, Result as ForgeResult, UpdateComment,
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
            Some(ForgeKind::GitHub) => {
                let client = GitHubClient::new(auth).context(
                    "Failed to authenticate with GitHub - run `gh auth login` or set GITHUB_TOKEN",
                )?;
                Ok(Self::GitHub(client))
            }
            None => Err(anyhow!(
                "unsupported forge: remote is not a recognized GitHub repository"
            )),
        }
    }
}

impl ForgeApi for Forge {
    async fn get_pr(&self, owner: &str, repo: &str, number: u64) -> ForgeResult<PullRequest> {
        match self {
            Self::GitHub(c) => c.get_pr(owner, repo, number).await,
        }
    }

    async fn get_prs_batch(
        &self,
        owner: &str,
        repo: &str,
        numbers: &[u64],
    ) -> ForgeResult<HashMap<u64, PullRequest>> {
        match self {
            Self::GitHub(c) => c.get_prs_batch(owner, repo, numbers).await,
        }
    }

    async fn find_pr_for_branch(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> ForgeResult<Option<PullRequest>> {
        match self {
            Self::GitHub(c) => c.find_pr_for_branch(owner, repo, branch).await,
        }
    }

    async fn create_pr(
        &self,
        owner: &str,
        repo: &str,
        pr: CreatePullRequest,
    ) -> ForgeResult<PullRequest> {
        match self {
            Self::GitHub(c) => c.create_pr(owner, repo, pr).await,
        }
    }

    async fn update_pr(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        update: UpdatePullRequest,
    ) -> ForgeResult<PullRequest> {
        match self {
            Self::GitHub(c) => c.update_pr(owner, repo, number, update).await,
        }
    }

    async fn get_check_runs(
        &self,
        owner: &str,
        repo: &str,
        commit_sha: &str,
    ) -> ForgeResult<Vec<CheckRun>> {
        match self {
            Self::GitHub(c) => c.get_check_runs(owner, repo, commit_sha).await,
        }
    }

    async fn merge_pr(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        merge: MergePullRequest,
    ) -> ForgeResult<MergeResult> {
        match self {
            Self::GitHub(c) => c.merge_pr(owner, repo, number, merge).await,
        }
    }

    async fn delete_ref(&self, owner: &str, repo: &str, ref_name: &str) -> ForgeResult<()> {
        match self {
            Self::GitHub(c) => c.delete_ref(owner, repo, ref_name).await,
        }
    }

    async fn get_default_branch(&self, owner: &str, repo: &str) -> ForgeResult<String> {
        match self {
            Self::GitHub(c) => c.get_default_branch(owner, repo).await,
        }
    }

    async fn list_pr_comments(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> ForgeResult<Vec<IssueComment>> {
        match self {
            Self::GitHub(c) => c.list_pr_comments(owner, repo, pr_number).await,
        }
    }

    async fn create_pr_comment(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        comment: CreateComment,
    ) -> ForgeResult<IssueComment> {
        match self {
            Self::GitHub(c) => c.create_pr_comment(owner, repo, pr_number, comment).await,
        }
    }

    async fn update_pr_comment(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        comment: UpdateComment,
    ) -> ForgeResult<IssueComment> {
        match self {
            Self::GitHub(c) => c.update_pr_comment(owner, repo, comment_id, comment).await,
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
