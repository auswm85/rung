//! Error types for rung-core.

use std::path::PathBuf;

/// Result type alias using [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur in rung-core operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Not inside a Git repository.
    #[error("not a git repository (or any parent up to mount point)")]
    NotARepository,

    /// The .git/rung directory doesn't exist (not initialized).
    #[error("rung not initialized in this repository - run `rung init` first")]
    NotInitialized,

    /// Branch not found.
    #[error("branch not found: {0}")]
    BranchNotFound(String),

    /// Invalid branch name.
    #[error("invalid branch name '{name}': {reason}")]
    InvalidBranchName {
        /// The invalid name.
        name: String,
        /// Why the name is invalid.
        reason: String,
    },

    /// Branch is not part of any stack.
    #[error("branch '{0}' is not part of a rung stack")]
    NotInStack(String),

    /// Cyclic dependency detected in stack.
    #[error("cyclic dependency detected: {0}")]
    CyclicDependency(String),

    /// Parent branch was deleted.
    #[error("parent branch '{parent}' for '{branch}' no longer exists")]
    OrphanedBranch { branch: String, parent: String },

    /// Conflict detected during sync.
    #[error("conflict in {file} while syncing {branch}")]
    ConflictDetected { branch: String, file: String },

    /// Rebase operation failed.
    #[error("rebase failed for branch '{0}': {1}")]
    RebaseFailed(String, String),

    /// No backup found for undo.
    #[error("no backup found - nothing to undo")]
    NoBackupFound,

    /// Sync already in progress.
    #[error("sync already in progress - run `rung sync --continue` or `rung sync --abort`")]
    SyncInProgress,

    /// State file parsing error.
    #[error("failed to parse {file}: {message}")]
    StateParseError { file: PathBuf, message: String },

    /// IO error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// TOML parsing error.
    #[error("toml error: {0}")]
    Toml(#[from] toml::de::Error),

    /// Git operation error.
    #[error("git error: {0}")]
    Git(#[from] rung_git::Error),

    /// Absorb operation error.
    #[error("absorb error: {0}")]
    Absorb(String),
}
