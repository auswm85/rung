//! Forge client dispatch.
//!
//! [`ForgeApi`] uses `impl Future` return types and is therefore not
//! dyn-compatible. The [`Forge`] enum provides static dispatch across the
//! supported forge backends, selected from a git remote URL via
//! [`rung_forge::ForgeKind::detect`]. Adding a backend means adding a variant
//! here — call sites stay backend-agnostic.

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use rung_core::Config;
use rung_forge::{
    CheckRun, CreateComment, CreatePullRequest, ForgeApi, ForgeKind, IssueComment,
    MergePullRequest, MergeResult, PullRequest, RemoteInfo, RepoId, Result as ForgeResult,
    UpdateComment, UpdatePullRequest,
};
use rung_github::{Auth as GitHubAuth, GitHubClient};
use rung_gitlab::{Auth as GitLabAuth, GitLabClient};

/// The configured self-hosted GitLab API base URL, normalized to include a
/// scheme.
///
/// A scheme-less value (`gitlab.example.com/api/v4`) is treated as `https` so it
/// still yields a usable URL for both host detection and API requests, rather
/// than being silently dropped.
fn gitlab_api_url(config: &Config) -> Option<String> {
    config.gitlab.api_url.as_deref().map(|url| {
        if url.starts_with("http://") || url.starts_with("https://") {
            url.to_owned()
        } else {
            format!("https://{url}")
        }
    })
}

/// Self-hosted GitLab host derived from `gitlab.api_url`, if configured.
///
/// The host cannot be inferred from a self-hosted remote URL alone, so it is
/// resolved from the configured API base URL and used to extend forge detection
/// and credential lookup beyond `gitlab.com`.
fn gitlab_host(config: &Config) -> Option<String> {
    gitlab_api_url(config).and_then(|url| rung_forge::host_from_url(&url).map(str::to_owned))
}

/// Parse a git remote URL into forge and repository, honoring a configured
/// self-hosted GitLab host from `config`.
///
/// # Errors
/// Returns an error if the remote is not a recognized forge repository.
pub fn parse_remote(remote_url: &str, config: &Config) -> ForgeResult<RemoteInfo> {
    let host = gitlab_host(config);
    let hosts: Vec<&str> = host.as_deref().into_iter().collect();
    rung_forge::parse_remote_with_hosts(remote_url, &hosts)
}

/// A forge client, statically dispatched by backend kind.
pub enum Forge {
    /// GitHub backend.
    GitHub(GitHubClient),
    /// GitLab backend.
    GitLab(GitLabClient),
}

impl Forge {
    /// Build a forge client for a git remote, dispatching on the detected forge.
    ///
    /// Credentials are resolved per backend from its own environment/CLI
    /// (`GITHUB_TOKEN`/`gh` for GitHub, `GITLAB_TOKEN`/`glab` for GitLab), so
    /// call sites stay backend-agnostic.
    ///
    /// `config` supplies the self-hosted GitLab host/API URL (`gitlab.api_url`)
    /// so remotes on custom hostnames are recognized and addressed correctly.
    ///
    /// # Errors
    /// Returns an error if the remote is not a recognized forge, or if
    /// authentication for the detected forge fails.
    pub fn for_remote(remote_url: &str, config: &Config) -> Result<Self> {
        // `test_token` is `None` in production (each backend resolves its own
        // credentials); tests inject a token so dispatch is exercised without
        // shelling out to `gh`/`glab`.
        Self::for_remote_impl(remote_url, config, None)
    }

    fn for_remote_impl(
        remote_url: &str,
        config: &Config,
        test_token: Option<&str>,
    ) -> Result<Self> {
        let gl_host = gitlab_host(config);
        let hosts: Vec<&str> = gl_host.as_deref().into_iter().collect();
        match ForgeKind::detect_with_hosts(remote_url, &hosts) {
            Some(kind @ ForgeKind::GitHub) => {
                let auth = test_token.map_or_else(GitHubAuth::auto, |t| {
                    GitHubAuth::Token(rung_github::SecretString::from(t))
                });
                let client = GitHubClient::new(&auth).with_context(|| auth_context(kind))?;
                Ok(Self::GitHub(client))
            }
            Some(kind @ ForgeKind::GitLab) => {
                // Apply the configured self-hosted host/API URL only when the
                // remote actually matched it. A gitlab.com remote is recognized
                // by literal detection (`detect` with no extra hosts), so it
                // keeps the standard URL and credentials even when a self-hosted
                // instance is also configured.
                let self_hosted = gl_host.filter(|_| ForgeKind::detect(remote_url).is_none());
                let auth = test_token.map_or_else(
                    || {
                        self_hosted
                            .as_deref()
                            .map_or_else(GitLabAuth::auto, GitLabAuth::auto_for_host)
                    },
                    |t| GitLabAuth::Token(rung_gitlab::SecretString::from(t)),
                );
                let client = self_hosted
                    .and_then(|_| gitlab_api_url(config))
                    .map_or_else(
                        || GitLabClient::new(&auth),
                        |base_url| GitLabClient::with_base_url(&auth, base_url),
                    )
                    .with_context(|| auth_context(kind))?;
                Ok(Self::GitLab(client))
            }
            None => Err(anyhow!(
                "unsupported forge: remote is not a recognized forge repository (supported: {})",
                ForgeKind::supported_label()
            )),
        }
    }
}

/// Authentication-failure context for a detected forge.
fn auth_context(kind: ForgeKind) -> String {
    format!(
        "Failed to authenticate with {} - {}",
        kind.display_name(),
        kind.auth_hint()
    )
}

// `GitHubClient` has inherent `(owner, repo, …)` methods that shadow the
// trait's `(&RepoId, …)` methods under normal method-call resolution, so each
// arm dispatches through `ForgeApi` explicitly to reach the trait impl.
impl ForgeApi for Forge {
    async fn get_pr(&self, repo: &RepoId, number: u64) -> ForgeResult<PullRequest> {
        match self {
            Self::GitHub(c) => ForgeApi::get_pr(c, repo, number).await,
            Self::GitLab(c) => ForgeApi::get_pr(c, repo, number).await,
        }
    }

    async fn get_prs_batch(
        &self,
        repo: &RepoId,
        numbers: &[u64],
    ) -> ForgeResult<HashMap<u64, PullRequest>> {
        match self {
            Self::GitHub(c) => ForgeApi::get_prs_batch(c, repo, numbers).await,
            Self::GitLab(c) => ForgeApi::get_prs_batch(c, repo, numbers).await,
        }
    }

    async fn find_pr_for_branch(
        &self,
        repo: &RepoId,
        branch: &str,
    ) -> ForgeResult<Option<PullRequest>> {
        match self {
            Self::GitHub(c) => ForgeApi::find_pr_for_branch(c, repo, branch).await,
            Self::GitLab(c) => ForgeApi::find_pr_for_branch(c, repo, branch).await,
        }
    }

    async fn create_pr(&self, repo: &RepoId, pr: CreatePullRequest) -> ForgeResult<PullRequest> {
        match self {
            Self::GitHub(c) => ForgeApi::create_pr(c, repo, pr).await,
            Self::GitLab(c) => ForgeApi::create_pr(c, repo, pr).await,
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
            Self::GitLab(c) => ForgeApi::update_pr(c, repo, number, update).await,
        }
    }

    async fn get_check_runs(&self, repo: &RepoId, commit_sha: &str) -> ForgeResult<Vec<CheckRun>> {
        match self {
            Self::GitHub(c) => ForgeApi::get_check_runs(c, repo, commit_sha).await,
            Self::GitLab(c) => ForgeApi::get_check_runs(c, repo, commit_sha).await,
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
            Self::GitLab(c) => ForgeApi::merge_pr(c, repo, number, merge).await,
        }
    }

    async fn delete_ref(&self, repo: &RepoId, ref_name: &str) -> ForgeResult<()> {
        match self {
            Self::GitHub(c) => ForgeApi::delete_ref(c, repo, ref_name).await,
            Self::GitLab(c) => ForgeApi::delete_ref(c, repo, ref_name).await,
        }
    }

    async fn get_default_branch(&self, repo: &RepoId) -> ForgeResult<String> {
        match self {
            Self::GitHub(c) => ForgeApi::get_default_branch(c, repo).await,
            Self::GitLab(c) => ForgeApi::get_default_branch(c, repo).await,
        }
    }

    async fn list_pr_comments(
        &self,
        repo: &RepoId,
        pr_number: u64,
    ) -> ForgeResult<Vec<IssueComment>> {
        match self {
            Self::GitHub(c) => ForgeApi::list_pr_comments(c, repo, pr_number).await,
            Self::GitLab(c) => ForgeApi::list_pr_comments(c, repo, pr_number).await,
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
            Self::GitLab(c) => ForgeApi::create_pr_comment(c, repo, pr_number, comment).await,
        }
    }

    async fn update_pr_comment(
        &self,
        repo: &RepoId,
        pr_number: u64,
        comment_id: u64,
        comment: UpdateComment,
    ) -> ForgeResult<IssueComment> {
        match self {
            Self::GitHub(c) => {
                ForgeApi::update_pr_comment(c, repo, pr_number, comment_id, comment).await
            }
            Self::GitLab(c) => {
                ForgeApi::update_pr_comment(c, repo, pr_number, comment_id, comment).await
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    /// Resolve a forge with an injected token so dispatch is exercised without
    /// shelling out to `gh`/`glab`.
    fn for_remote(remote_url: &str) -> Result<Forge> {
        Forge::for_remote_impl(remote_url, &Config::default(), Some("test_token"))
    }

    /// Resolve a forge with a self-hosted GitLab API URL configured.
    fn for_remote_with_gitlab_api(remote_url: &str, api_url: &str) -> Result<Forge> {
        let mut config = Config::default();
        config.gitlab.api_url = Some(api_url.to_string());
        Forge::for_remote_impl(remote_url, &config, Some("test_token"))
    }

    #[test]
    fn test_for_remote_github_https() {
        let forge = for_remote("https://github.com/octocat/hello-world.git")
            .expect("github remote should resolve");
        assert!(matches!(forge, Forge::GitHub(_)));
    }

    #[test]
    fn test_for_remote_github_ssh() {
        let forge = for_remote("git@github.com:octocat/hello-world.git")
            .expect("github ssh remote should resolve");
        assert!(matches!(forge, Forge::GitHub(_)));
    }

    #[test]
    fn test_for_remote_gitlab_https() {
        let forge =
            for_remote("https://gitlab.com/owner/repo.git").expect("gitlab remote should resolve");
        assert!(matches!(forge, Forge::GitLab(_)));
    }

    #[test]
    fn test_for_remote_gitlab_ssh() {
        let remote = "git@gitlab.com:group/subgroup/project.git";
        let forge = for_remote(remote).expect("gitlab ssh remote should resolve");
        assert!(matches!(forge, Forge::GitLab(_)));

        // The full nested namespace must survive as the repo identity; the
        // GitLab client URL-encodes this whole path when addressing the project.
        let info = rung_forge::parse_remote(remote).expect("gitlab ssh remote should parse");
        assert_eq!(info.repo.path(), "group/subgroup/project");
    }

    #[test]
    fn test_for_remote_unrecognized_forge_errors() {
        // An unrecognized remote must not silently fall back to a known forge.
        assert!(for_remote("https://example.com/owner/repo.git").is_err());
        assert!(for_remote("not a url").is_err());
    }

    #[test]
    fn test_for_remote_self_hosted_gitlab_resolves() {
        // A remote on the configured self-hosted host resolves to GitLab.
        let forge = for_remote_with_gitlab_api(
            "https://gitlab.example.com/group/project.git",
            "https://gitlab.example.com/api/v4",
        )
        .expect("self-hosted gitlab remote should resolve");
        assert!(matches!(forge, Forge::GitLab(_)));
    }

    #[test]
    fn test_for_remote_self_hosted_gitlab_ssh_resolves() {
        let forge = for_remote_with_gitlab_api(
            "git@gitlab.example.com:group/sub/project.git",
            "https://gitlab.example.com/api/v4",
        )
        .expect("self-hosted gitlab ssh remote should resolve");
        assert!(matches!(forge, Forge::GitLab(_)));
    }

    #[test]
    fn test_for_remote_self_hosted_host_unconfigured_errors() {
        // Without the matching config, the self-hosted host is unrecognized.
        assert!(for_remote("https://gitlab.example.com/group/project.git").is_err());
    }

    #[test]
    fn test_parse_remote_honors_self_hosted_config() {
        let mut config = Config::default();
        config.gitlab.api_url = Some("https://gitlab.example.com/api/v4".into());

        let info = parse_remote("git@gitlab.example.com:group/sub/project.git", &config)
            .expect("self-hosted remote should parse");
        assert_eq!(info.kind, ForgeKind::GitLab);
        assert_eq!(info.repo.path(), "group/sub/project");

        // The same remote is unrecognized without the config.
        assert!(
            parse_remote(
                "git@gitlab.example.com:group/sub/project.git",
                &Config::default()
            )
            .is_err()
        );
    }

    #[test]
    fn test_for_remote_self_hosted_scheme_less_api_url() {
        // A scheme-less api_url must still yield a usable host so self-hosted
        // detection works rather than falling back to gitlab.com.
        let forge = for_remote_with_gitlab_api(
            "https://gitlab.example.com/group/project.git",
            "gitlab.example.com/api/v4",
        )
        .expect("self-hosted remote should resolve with a scheme-less api_url");
        assert!(matches!(forge, Forge::GitLab(_)));
    }

    #[test]
    fn test_parse_remote_scheme_less_api_url() {
        let mut config = Config::default();
        config.gitlab.api_url = Some("gitlab.example.com/api/v4".into());
        let info = parse_remote("git@gitlab.example.com:group/project.git", &config)
            .expect("scheme-less api_url should still derive the host");
        assert_eq!(info.kind, ForgeKind::GitLab);
        assert_eq!(info.repo.path(), "group/project");
    }

    #[test]
    fn test_for_remote_gitlab_com_not_misrouted_by_self_hosted_config() {
        // With a self-hosted instance configured, a gitlab.com remote must still
        // resolve (via literal detection) and not be treated as unrecognized or
        // routed to the self-hosted host.
        let forge = for_remote_with_gitlab_api(
            "https://gitlab.com/owner/repo.git",
            "https://gitlab.example.com/api/v4",
        )
        .expect("gitlab.com remote should resolve even with self-hosted config");
        assert!(matches!(forge, Forge::GitLab(_)));
    }
}
