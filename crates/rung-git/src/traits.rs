//! Trait abstractions for git operations.
//!
//! This module defines the `GitOps` trait which abstracts git operations,
//! enabling dependency injection and testability.

use std::path::Path;

use git2::Oid;

use crate::{BlameResult, Hunk, RemoteDivergence, Result};

/// Trait for git repository operations.
///
/// This trait abstracts git operations, allowing for:
/// - Dependency injection in commands/services
/// - Mock implementations for testing
/// - Alternative implementations (e.g., dry-run mode)
///
/// Note: Unlike `GitHubApi`, git operations are synchronous since
/// git2 is a synchronous library.
#[allow(clippy::missing_errors_doc)]
pub trait GitOps {
    // === Repository Info ===

    /// Get the working directory path.
    fn workdir(&self) -> Option<&Path>;

    /// Get the current branch name.
    ///
    /// Returns an error if HEAD is detached or not on a branch.
    fn current_branch(&self) -> Result<String>;

    /// Check if HEAD is detached.
    fn head_detached(&self) -> Result<bool>;

    /// Check if a rebase is in progress.
    fn is_rebasing(&self) -> bool;

    // === Branch Operations ===

    /// Check if a branch exists.
    fn branch_exists(&self, name: &str) -> bool;

    /// Create a new branch at the current HEAD.
    ///
    /// Returns the OID of the new branch's tip commit.
    fn create_branch(&self, name: &str) -> Result<Oid>;

    /// Checkout a branch.
    fn checkout(&self, branch: &str) -> Result<()>;

    /// Delete a local branch.
    fn delete_branch(&self, name: &str) -> Result<()>;

    /// List all local branches.
    fn list_branches(&self) -> Result<Vec<String>>;

    // === Commit Operations ===

    /// Get the commit ID for a branch.
    fn branch_commit(&self, branch: &str) -> Result<Oid>;

    /// Get the commit ID for a remote branch.
    fn remote_branch_commit(&self, branch: &str) -> Result<Oid>;

    /// Get the commit message for a branch's tip.
    fn branch_commit_message(&self, branch: &str) -> Result<String>;

    /// Find the merge base of two commits.
    fn merge_base(&self, one: Oid, two: Oid) -> Result<Oid>;

    /// Get commits between two OIDs.
    fn commits_between(&self, from: Oid, to: Oid) -> Result<Vec<Oid>>;

    /// Count commits between two OIDs.
    fn count_commits_between(&self, from: Oid, to: Oid) -> Result<usize>;

    // === Working Directory ===

    /// Check if the working directory is clean.
    fn is_clean(&self) -> Result<bool>;

    /// Require that the working directory is clean.
    fn require_clean(&self) -> Result<()>;

    /// Stage all changes.
    fn stage_all(&self) -> Result<()>;

    /// Check if there are staged changes.
    fn has_staged_changes(&self) -> Result<bool>;

    /// Create a commit with the staged changes.
    fn create_commit(&self, message: &str) -> Result<Oid>;

    // === Rebase Operations ===

    /// Rebase the current branch onto a target commit.
    fn rebase_onto(&self, target: Oid) -> Result<()>;

    /// Rebase using --onto semantics (rebase commits from `from` onto `onto`).
    fn rebase_onto_from(&self, onto: Oid, from: Oid) -> Result<()>;

    /// Get files with conflicts during a rebase.
    fn conflicting_files(&self) -> Result<Vec<String>>;

    /// Abort a rebase in progress.
    fn rebase_abort(&self) -> Result<()>;

    /// Continue a rebase after resolving conflicts.
    fn rebase_continue(&self) -> Result<()>;

    // === Remote Operations ===

    /// Get the origin URL.
    fn origin_url(&self) -> Result<String>;

    /// Check divergence between local and remote branch.
    fn remote_divergence(&self, branch: &str) -> Result<RemoteDivergence>;

    /// Detect the default branch (main/master).
    ///
    /// Returns `None` if neither main nor master exists.
    fn detect_default_branch(&self) -> Option<String>;

    /// Push a branch to the remote.
    fn push(&self, branch: &str, force: bool) -> Result<()>;

    /// Fetch all remotes.
    fn fetch_all(&self) -> Result<()>;

    /// Fetch a specific branch.
    fn fetch(&self, branch: &str) -> Result<()>;

    /// Pull with fast-forward only.
    fn pull_ff(&self) -> Result<()>;

    /// Reset a branch to a specific commit.
    fn reset_branch(&self, branch: &str, commit: Oid) -> Result<()>;
}

/// Trait for absorb-specific git operations.
///
/// This trait abstracts the git operations needed for the absorb command,
/// enabling dependency injection and testability.
#[allow(clippy::missing_errors_doc)]
pub trait AbsorbOps: GitOps {
    /// Get the staged diff as a list of hunks.
    fn staged_diff_hunks(&self) -> Result<Vec<Hunk>>;

    /// Query git blame for a specific line range in a file.
    fn blame_lines(&self, file_path: &str, start: u32, end: u32) -> Result<Vec<BlameResult>>;

    /// Check if a commit is an ancestor of another commit.
    fn is_ancestor(&self, ancestor: Oid, descendant: Oid) -> Result<bool>;

    /// Create a fixup commit targeting the specified commit.
    fn create_fixup_commit(&self, target: Oid) -> Result<Oid>;
}
