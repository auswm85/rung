//! Forge detection and remote-URL parsing.
//!
//! A git remote URL identifies both *which* forge hosts the repository
//! ([`ForgeKind`]) and *which* repository it is ([`RemoteInfo`]). Keeping this
//! logic in the contract crate means `rung-git` stays forge-agnostic and new
//! backends extend detection in one place.

use crate::{ForgeError, RepoId, Result};

/// A supported code-hosting forge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgeKind {
    /// github.com (and GitHub-style remotes).
    GitHub,
    /// gitlab.com (and GitLab-style remotes).
    GitLab,
}

impl ForgeKind {
    /// Every forge backend rung supports.
    ///
    /// Used to build user-facing "supported forges" hints in errors where no
    /// specific forge was detected (see [`ForgeKind::supported_label`]).
    pub const ALL: &'static [Self] = &[Self::GitHub, Self::GitLab];

    /// Human-readable name of the forge, for user-facing messages.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::GitLab => "GitLab",
        }
    }

    /// Markdown/reference prefix for a change request number: `#` on GitHub,
    /// `!` on GitLab (where `#` refers to an issue). Mirrors
    /// [`ForgeApi::pr_reference_prefix`](crate::ForgeApi::pr_reference_prefix)
    /// for contexts that have a [`ForgeKind`] but no authenticated client (e.g.
    /// rendering the status tree without contacting the forge).
    #[must_use]
    pub const fn reference_prefix(self) -> &'static str {
        match self {
            Self::GitHub => "#",
            Self::GitLab => "!",
        }
    }

    /// Hint describing how to authenticate with this forge, for error messages.
    #[must_use]
    pub const fn auth_hint(self) -> &'static str {
        match self {
            Self::GitHub => "run `gh auth login` or set GITHUB_TOKEN",
            Self::GitLab => "run `glab auth login` or set GITLAB_TOKEN",
        }
    }

    /// Comma-separated display names of every supported forge (e.g. `"GitHub"`).
    ///
    /// For "unrecognized remote" errors, where there is no detected forge to
    /// name but listing the supported ones guides the user.
    #[must_use]
    pub fn supported_label() -> String {
        Self::ALL
            .iter()
            .map(|kind| kind.display_name())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Detect the forge that hosts a git remote URL.
    ///
    /// Recognizes both HTTPS and SSH forms against the hosted forges
    /// (`github.com`, `gitlab.com`). Returns `None` if the host is not a known
    /// forge. Use [`ForgeKind::detect_with_hosts`] to also recognize self-hosted
    /// GitLab instances.
    #[must_use]
    pub fn detect(url: &str) -> Option<Self> {
        Self::detect_with_hosts(url, &[])
    }

    /// Detect the forge, additionally treating each host in `gitlab_hosts` as a
    /// self-hosted GitLab instance.
    ///
    /// Self-hosted GitLab lives on arbitrary hostnames that cannot be inferred
    /// from the URL alone, so the configured hosts are supplied by the caller
    /// (from `.git/rung/config.toml`). Recognizes both HTTPS and SSH forms.
    #[must_use]
    pub fn detect_with_hosts(url: &str, gitlab_hosts: &[&str]) -> Option<Self> {
        if url_has_host(url, "github.com") {
            return Some(Self::GitHub);
        }
        if url_has_host(url, "gitlab.com")
            || gitlab_hosts.iter().any(|host| url_has_host(url, host))
        {
            return Some(Self::GitLab);
        }
        None
    }
}

/// Whether a remote URL points at `host`, in either SSH or HTTPS/HTTP form.
fn url_has_host(url: &str, host: &str) -> bool {
    url.starts_with(&format!("git@{host}:"))
        || url.starts_with(&format!("https://{host}/"))
        || url.starts_with(&format!("http://{host}/"))
}

/// Extract the host from an HTTP(S) URL, e.g.
/// `https://gitlab.example.com/api/v4` → `Some("gitlab.example.com")`.
///
/// Strips any userinfo (`user:pass@`) and port. Bracketed IPv6 literals are
/// returned with their brackets (`https://[::1]:8080/…` → `Some("[::1]")`).
/// Returns `None` for URLs that are not HTTP(S) or that have an empty host.
/// Used to derive a self-hosted GitLab host from a configured API base URL.
#[must_use]
pub fn host_from_url(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split('/').next()?;
    // Drop any `user:pass@` userinfo prefix.
    let host_port = authority.rsplit('@').next()?;
    let host = if host_port.starts_with('[') {
        // IPv6 literal: the host is the bracketed portion; a `:port` may follow
        // the closing bracket. Keep the brackets so the host round-trips in URLs.
        let end = host_port.find(']')?;
        &host_port[..=end]
    } else {
        // Otherwise strip any `:port` suffix.
        host_port.split(':').next()?
    };
    if host.is_empty() { None } else { Some(host) }
}

/// A repository identified from a git remote URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteInfo {
    /// The forge hosting the repository.
    pub kind: ForgeKind,
    /// Forge-neutral identifier for the repository/project.
    pub repo: RepoId,
}

/// Parse a git remote URL into its forge, owner, and repository.
///
/// Supports both HTTPS and SSH URLs:
/// - `https://github.com/owner/repo.git`
/// - `git@github.com:owner/repo.git`
/// - `https://gitlab.com/owner/repo.git`
/// - `git@gitlab.com:owner/repo.git`
///
/// # Errors
/// Returns [`ForgeError::InvalidRemoteUrl`] if the URL is not a recognized
/// forge remote or the owner/repo path cannot be extracted.
pub fn parse_remote(url: &str) -> Result<RemoteInfo> {
    parse_remote_with_hosts(url, &[])
}

/// Parse a git remote URL, additionally treating each host in `gitlab_hosts` as
/// a self-hosted GitLab instance.
///
/// Mirrors [`ForgeKind::detect_with_hosts`]: the configured hosts extend GitLab
/// recognition beyond `gitlab.com` so self-hosted remotes resolve to their
/// project path.
///
/// # Errors
/// Returns [`ForgeError::InvalidRemoteUrl`] if the URL is not a recognized
/// forge remote or the owner/repo path cannot be extracted.
pub fn parse_remote_with_hosts(url: &str, gitlab_hosts: &[&str]) -> Result<RemoteInfo> {
    if url_has_host(url, "github.com") {
        return parse_host(url, ForgeKind::GitHub, "github.com");
    }
    if url_has_host(url, "gitlab.com") {
        return parse_host(url, ForgeKind::GitLab, "gitlab.com");
    }
    for host in gitlab_hosts {
        if url_has_host(url, host) {
            return parse_host(url, ForgeKind::GitLab, host);
        }
    }
    Err(ForgeError::InvalidRemoteUrl(url.to_string()))
}

/// Extract `(owner, repo)` from a `host` remote in either SSH or HTTPS form.
fn parse_host(url: &str, kind: ForgeKind, host: &str) -> Result<RemoteInfo> {
    // SSH format: git@<host>:owner/repo.git
    let path = url
        .strip_prefix(&format!("git@{host}:"))
        .or_else(|| {
            // HTTPS format: https://<host>/owner/repo.git
            url.strip_prefix(&format!("https://{host}/"))
        })
        .or_else(|| url.strip_prefix(&format!("http://{host}/")));

    if let Some(path) = path {
        let path = path.trim_end_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        // GitHub repositories are always `owner/repo`, so reject extra segments
        // as malformed. GitLab projects live under nested namespaces
        // (`group/subgroup/project`), so accept two or more segments there. In
        // both cases every segment must be non-empty; the validated path is the
        // canonical slug, used as-is.
        let segments: Vec<&str> = path.split('/').collect();
        let segments_ok = segments.iter().all(|s| !s.is_empty())
            && match kind {
                ForgeKind::GitHub => segments.len() == 2,
                ForgeKind::GitLab => segments.len() >= 2,
            };
        if segments_ok {
            return Ok(RemoteInfo {
                kind,
                repo: RepoId::new(path),
            });
        }
    }

    Err(ForgeError::InvalidRemoteUrl(url.to_string()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_display_name() {
        assert_eq!(ForgeKind::GitHub.display_name(), "GitHub");
        assert_eq!(ForgeKind::GitLab.display_name(), "GitLab");
    }

    #[test]
    fn test_auth_hint_mentions_credentials() {
        let hint = ForgeKind::GitHub.auth_hint();
        assert!(hint.contains("gh auth login"));
        assert!(hint.contains("GITHUB_TOKEN"));

        let hint = ForgeKind::GitLab.auth_hint();
        assert!(hint.contains("glab auth login"));
        assert!(hint.contains("GITLAB_TOKEN"));
    }

    #[test]
    fn test_supported_label_lists_all_kinds() {
        let label = ForgeKind::supported_label();
        for kind in ForgeKind::ALL {
            assert!(
                label.contains(kind.display_name()),
                "supported_label {label:?} omits {}",
                kind.display_name()
            );
        }
    }

    #[test]
    fn test_detect_github_https() {
        assert_eq!(
            ForgeKind::detect("https://github.com/owner/repo.git"),
            Some(ForgeKind::GitHub)
        );
    }

    #[test]
    fn test_detect_github_ssh() {
        assert_eq!(
            ForgeKind::detect("git@github.com:owner/repo.git"),
            Some(ForgeKind::GitHub)
        );
    }

    #[test]
    fn test_detect_gitlab_https() {
        assert_eq!(
            ForgeKind::detect("https://gitlab.com/owner/repo.git"),
            Some(ForgeKind::GitLab)
        );
    }

    #[test]
    fn test_detect_gitlab_ssh() {
        assert_eq!(
            ForgeKind::detect("git@gitlab.com:owner/repo.git"),
            Some(ForgeKind::GitLab)
        );
    }

    #[test]
    fn test_detect_unknown_host() {
        assert_eq!(
            ForgeKind::detect("https://bitbucket.org/owner/repo.git"),
            None
        );
        assert_eq!(ForgeKind::detect("git@bitbucket.org:owner/repo.git"), None);
        assert_eq!(ForgeKind::detect("not a url"), None);
    }

    #[test]
    fn test_parse_https_with_git_suffix() {
        let info = parse_remote("https://github.com/octocat/hello-world.git").unwrap();
        assert_eq!(info.kind, ForgeKind::GitHub);
        assert_eq!(info.repo.path(), "octocat/hello-world");
    }

    #[test]
    fn test_parse_https_without_git_suffix() {
        let info = parse_remote("https://github.com/octocat/hello-world").unwrap();
        assert_eq!(info.repo.path(), "octocat/hello-world");
    }

    #[test]
    fn test_parse_ssh() {
        let info = parse_remote("git@github.com:octocat/hello-world.git").unwrap();
        assert_eq!(info.kind, ForgeKind::GitHub);
        assert_eq!(info.repo.path(), "octocat/hello-world");
    }

    #[test]
    fn test_parse_gitlab_https() {
        let info = parse_remote("https://gitlab.com/octocat/hello-world.git").unwrap();
        assert_eq!(info.kind, ForgeKind::GitLab);
        assert_eq!(info.repo.path(), "octocat/hello-world");
    }

    #[test]
    fn test_parse_gitlab_ssh() {
        let info = parse_remote("git@gitlab.com:octocat/hello-world.git").unwrap();
        assert_eq!(info.kind, ForgeKind::GitLab);
        assert_eq!(info.repo.path(), "octocat/hello-world");
    }

    #[test]
    fn test_parse_gitlab_nested_namespace_https() {
        // GitLab projects can live under nested namespaces; the whole path is
        // retained as the repo identity (the client URL-encodes it).
        let info = parse_remote("https://gitlab.com/group/subgroup/project.git").unwrap();
        assert_eq!(info.kind, ForgeKind::GitLab);
        assert_eq!(info.repo.path(), "group/subgroup/project");
    }

    #[test]
    fn test_parse_gitlab_nested_namespace_ssh() {
        let info = parse_remote("git@gitlab.com:group/subgroup/project.git").unwrap();
        assert_eq!(info.kind, ForgeKind::GitLab);
        assert_eq!(info.repo.path(), "group/subgroup/project");
    }

    #[test]
    fn test_parse_github_rejects_nested_namespace() {
        // GitHub has no nested namespaces; extra segments stay an error.
        assert!(matches!(
            parse_remote("git@github.com:group/subgroup/project.git").unwrap_err(),
            ForgeError::InvalidRemoteUrl(_)
        ));
    }

    #[test]
    fn test_detect_self_hosted_gitlab_https() {
        let hosts = ["gitlab.example.com"];
        assert_eq!(
            ForgeKind::detect_with_hosts("https://gitlab.example.com/group/project.git", &hosts),
            Some(ForgeKind::GitLab)
        );
        // Without the configured host it is unrecognized.
        assert_eq!(
            ForgeKind::detect("https://gitlab.example.com/group/project.git"),
            None
        );
    }

    #[test]
    fn test_detect_self_hosted_gitlab_ssh() {
        let hosts = ["gitlab.example.com"];
        assert_eq!(
            ForgeKind::detect_with_hosts("git@gitlab.example.com:group/project.git", &hosts),
            Some(ForgeKind::GitLab)
        );
    }

    #[test]
    fn test_parse_self_hosted_gitlab_nested_namespace() {
        let hosts = ["gitlab.example.com"];
        let info = parse_remote_with_hosts("git@gitlab.example.com:group/sub/project.git", &hosts)
            .unwrap();
        assert_eq!(info.kind, ForgeKind::GitLab);
        assert_eq!(info.repo.path(), "group/sub/project");
    }

    #[test]
    fn test_parse_self_hosted_host_still_honors_hosted_forges() {
        // A configured self-hosted host must not shadow github.com/gitlab.com.
        let hosts = ["gitlab.example.com"];
        let info = parse_remote_with_hosts("https://github.com/octocat/repo.git", &hosts).unwrap();
        assert_eq!(info.kind, ForgeKind::GitHub);
    }

    #[test]
    fn test_host_from_url() {
        assert_eq!(
            host_from_url("https://gitlab.example.com/api/v4"),
            Some("gitlab.example.com")
        );
        assert_eq!(
            host_from_url("http://gitlab.local:8080/api/v4"),
            Some("gitlab.local")
        );
        assert_eq!(
            host_from_url("https://user:pass@gitlab.example.com/api/v4"),
            Some("gitlab.example.com")
        );
        assert_eq!(
            host_from_url("git@gitlab.example.com:group/project.git"),
            None
        );
        assert_eq!(host_from_url("https:///api/v4"), None);
        // IPv6 literals keep their brackets and drop the port.
        assert_eq!(host_from_url("https://[::1]:8080/api/v4"), Some("[::1]"));
        assert_eq!(
            host_from_url("http://[2001:db8::1]/api/v4"),
            Some("[2001:db8::1]")
        );
    }

    #[test]
    fn test_parse_unknown_forge_errors() {
        let err = parse_remote("https://bitbucket.org/owner/repo.git").unwrap_err();
        assert!(matches!(err, ForgeError::InvalidRemoteUrl(_)));
    }

    #[test]
    fn test_parse_missing_repo_errors() {
        // Host matches but there is no owner/repo path.
        assert!(matches!(
            parse_remote("https://github.com/").unwrap_err(),
            ForgeError::InvalidRemoteUrl(_)
        ));
        assert!(matches!(
            parse_remote("https://github.com/owner").unwrap_err(),
            ForgeError::InvalidRemoteUrl(_)
        ));
    }

    #[test]
    fn test_parse_trailing_slash_is_trimmed() {
        let info = parse_remote("https://github.com/octocat/hello-world/").unwrap();
        assert_eq!(info.repo.path(), "octocat/hello-world");

        // Trailing slash after the `.git` suffix is also tolerated.
        let info = parse_remote("https://github.com/octocat/hello-world.git/").unwrap();
        assert_eq!(info.repo.path(), "octocat/hello-world");
    }

    #[test]
    fn test_parse_extra_path_segments_error() {
        // Extra segments must not be swallowed into `repo`.
        assert!(matches!(
            parse_remote("https://github.com/owner/repo/extra").unwrap_err(),
            ForgeError::InvalidRemoteUrl(_)
        ));
    }

    #[test]
    fn test_invalid_remote_url_message_omits_url() {
        // Credentials embedded in a remote URL must not leak via Display.
        let err = ForgeError::InvalidRemoteUrl("https://user:token@host/x".to_string());
        let msg = err.to_string();
        assert!(!msg.contains("token"), "URL leaked in error message: {msg}");
        assert!(!msg.contains("host"), "URL leaked in error message: {msg}");
    }
}
