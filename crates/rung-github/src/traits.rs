//! Trait abstractions for GitHub API operations.
//!
//! This module defines the `GitHubApi` trait which abstracts GitHub API operations,
//! enabling dependency injection and testability.

use std::collections::HashMap;

use crate::{
    CheckRun, CreateComment, CreatePullRequest, IssueComment, MergePullRequest, MergeResult,
    PullRequest, Result, UpdateComment, UpdatePullRequest,
};

/// Trait for GitHub API operations.
///
/// This trait abstracts GitHub API calls, allowing for:
/// - Dependency injection in commands/services
/// - Mock implementations for testing
/// - Alternative implementations (e.g., offline mode, caching)
///
/// All methods take `owner` and `repo` as parameters to support
/// operations across different repositories.
pub trait GitHubApi: Send + Sync {
    // === PR Operations ===

    /// Get a pull request by number.
    fn get_pr(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> impl std::future::Future<Output = Result<PullRequest>> + Send;

    /// Get multiple pull requests by number (batch operation).
    ///
    /// Returns a map of PR number to PR data. Missing PRs are omitted.
    fn get_prs_batch(
        &self,
        owner: &str,
        repo: &str,
        numbers: &[u64],
    ) -> impl std::future::Future<Output = Result<HashMap<u64, PullRequest>>> + Send;

    /// Find a PR for a branch.
    ///
    /// Returns `None` if no open PR exists for the branch.
    fn find_pr_for_branch(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> impl std::future::Future<Output = Result<Option<PullRequest>>> + Send;

    /// Create a pull request.
    fn create_pr(
        &self,
        owner: &str,
        repo: &str,
        pr: CreatePullRequest,
    ) -> impl std::future::Future<Output = Result<PullRequest>> + Send;

    /// Update a pull request.
    fn update_pr(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        update: UpdatePullRequest,
    ) -> impl std::future::Future<Output = Result<PullRequest>> + Send;

    // === Check Runs ===

    /// Get check runs for a commit.
    fn get_check_runs(
        &self,
        owner: &str,
        repo: &str,
        commit_sha: &str,
    ) -> impl std::future::Future<Output = Result<Vec<CheckRun>>> + Send;

    // === Merge Operations ===

    /// Merge a pull request.
    fn merge_pr(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        merge: MergePullRequest,
    ) -> impl std::future::Future<Output = Result<MergeResult>> + Send;

    // === Ref Operations ===

    /// Delete a git reference (branch).
    fn delete_ref(
        &self,
        owner: &str,
        repo: &str,
        ref_name: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    // === Repository Operations ===

    /// Get the repository's default branch name.
    fn get_default_branch(
        &self,
        owner: &str,
        repo: &str,
    ) -> impl std::future::Future<Output = Result<String>> + Send;

    // === Comment Operations ===

    /// List comments on a pull request.
    fn list_pr_comments(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> impl std::future::Future<Output = Result<Vec<IssueComment>>> + Send;

    /// Create a comment on a pull request.
    fn create_pr_comment(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        comment: CreateComment,
    ) -> impl std::future::Future<Output = Result<IssueComment>> + Send;

    /// Update a comment on a pull request.
    fn update_pr_comment(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        comment: UpdateComment,
    ) -> impl std::future::Future<Output = Result<IssueComment>> + Send;
}
