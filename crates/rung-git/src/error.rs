//! Error types for rung-git.

/// Result type alias using [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur during git operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Not inside a git repository.
    #[error("not a git repository")]
    NotARepository,

    /// Branch not found.
    #[error("branch not found: {0}")]
    BranchNotFound(String),

    /// Reference not found.
    #[error("reference not found: {0}")]
    RefNotFound(String),

    /// HEAD is detached (not on a branch).
    #[error("HEAD is detached - checkout a branch first")]
    DetachedHead,

    /// Rebase conflict.
    #[error("rebase conflict in: {0:?}")]
    RebaseConflict(Vec<String>),

    /// Rebase failed.
    #[error("rebase failed: {0}")]
    RebaseFailed(String),

    /// Working directory is dirty.
    #[error("working directory has uncommitted changes")]
    DirtyWorkingDirectory,

    /// Remote not found.
    #[error("remote not found: {0}")]
    RemoteNotFound(String),

    /// Invalid remote URL.
    #[error("invalid remote URL: {0}")]
    InvalidRemoteUrl(String),

    /// Push failed.
    #[error("push failed: {0}")]
    PushFailed(String),

    /// Fetch failed.
    #[error("fetch failed: {0}")]
    FetchFailed(String),

    /// Underlying git2 error.
    #[error("git error: {0}")]
    Git2(#[from] git2::Error),
}
