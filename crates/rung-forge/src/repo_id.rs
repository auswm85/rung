//! Forge-neutral repository identifier.

use std::fmt;

/// A forge-neutral identifier for a repository (GitHub) or project (GitLab).
///
/// Different forges name repositories differently:
/// - GitHub uses a flat `owner/repo` pair.
/// - GitLab uses a nested namespace path (`group/subgroup/project`) or a
///   numeric project ID — neither of which an `owner`/`repo` pair can represent.
///
/// `RepoId` stores the canonical, forge-native project *path* as a single
/// string so both models map onto one type. Each backend interprets it:
/// the GitHub client splits it into `owner` and `repo` on its single `/`,
/// while a GitLab client URL-encodes the whole path for its API.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoId {
    path: String,
}

impl RepoId {
    /// Create a `RepoId` from a forge-native project path.
    ///
    /// The path is the full slug as the forge names it, e.g. `owner/repo`
    /// for GitHub or `group/subgroup/project` for GitLab.
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }

    /// The forge-native project path (e.g. `owner/repo` or `group/sub/project`).
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
}

impl fmt::Display for RepoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_roundtrips() {
        let id = RepoId::new("octocat/hello-world");
        assert_eq!(id.path(), "octocat/hello-world");
    }

    #[test]
    fn test_accepts_nested_namespace() {
        let id = RepoId::new("group/subgroup/project");
        assert_eq!(id.path(), "group/subgroup/project");
    }

    #[test]
    fn test_display_is_path() {
        assert_eq!(RepoId::new("a/b").to_string(), "a/b");
    }

    #[test]
    fn test_equality_and_clone() {
        let a = RepoId::new("o/r");
        let b = a.clone();
        assert_eq!(a, b);
        assert_ne!(a, RepoId::new("o/other"));
    }
}
