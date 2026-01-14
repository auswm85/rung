//! Error types for rung-github.

/// Result type alias using [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur during GitHub API operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Authentication failed or token missing.
    #[error("GitHub authentication failed - run `gh auth login` or set GITHUB_TOKEN")]
    AuthenticationFailed,

    /// Token not found.
    #[error("no GitHub token found - run `gh auth login` or set GITHUB_TOKEN")]
    NoToken,

    /// API rate limit exceeded.
    #[error("GitHub API rate limit exceeded - wait and try again")]
    RateLimited,

    /// Repository not found or no access.
    #[error("repository not found or no access: {0}")]
    RepoNotFound(String),

    /// PR not found.
    #[error("pull request not found: #{0}")]
    PrNotFound(u64),

    /// API error with status code.
    #[error("GitHub API error ({status}): {message}")]
    ApiError { status: u16, message: String },

    /// Network error.
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    /// JSON parsing error.
    #[error("failed to parse GitHub response: {0}")]
    Parse(#[from] serde_json::Error),

    /// IO error (e.g., reading gh CLI token).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
