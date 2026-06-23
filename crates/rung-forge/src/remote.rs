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
}

impl ForgeKind {
    /// Detect the forge that hosts a git remote URL.
    ///
    /// Recognizes both HTTPS and SSH forms. Returns `None` if the host is not
    /// a known forge.
    #[must_use]
    pub fn detect(url: &str) -> Option<Self> {
        if url.starts_with("git@github.com:")
            || url.starts_with("https://github.com/")
            || url.starts_with("http://github.com/")
        {
            return Some(Self::GitHub);
        }
        None
    }
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
///
/// # Errors
/// Returns [`ForgeError::InvalidRemoteUrl`] if the URL is not a recognized
/// forge remote or the owner/repo path cannot be extracted.
pub fn parse_remote(url: &str) -> Result<RemoteInfo> {
    match ForgeKind::detect(url) {
        Some(ForgeKind::GitHub) => parse_github(url),
        None => Err(ForgeError::InvalidRemoteUrl(url.to_string())),
    }
}

/// Extract `(owner, repo)` from a github.com remote in either SSH or HTTPS form.
fn parse_github(url: &str) -> Result<RemoteInfo> {
    // SSH format: git@github.com:owner/repo.git
    let path = url.strip_prefix("git@github.com:").or_else(|| {
        // HTTPS format: https://github.com/owner/repo.git
        url.strip_prefix("https://github.com/")
            .or_else(|| url.strip_prefix("http://github.com/"))
    });

    if let Some(path) = path {
        let path = path.trim_end_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        // Require exactly `owner/repo` — reject extra path segments so a
        // malformed remote fails here rather than later as a bad API repo name.
        // The guard proves `path` is exactly two non-empty segments, so it is
        // already the canonical `owner/repo` slug; use it as-is.
        let mut parts = path.split('/');
        if let (Some(owner), Some(repo), None) = (parts.next(), parts.next(), parts.next())
            && !owner.is_empty()
            && !repo.is_empty()
        {
            return Ok(RemoteInfo {
                kind: ForgeKind::GitHub,
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
    fn test_detect_unknown_host() {
        assert_eq!(ForgeKind::detect("https://gitlab.com/owner/repo.git"), None);
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
    fn test_parse_unknown_forge_errors() {
        let err = parse_remote("https://gitlab.com/owner/repo.git").unwrap_err();
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
