//! GitHub API types.

use serde::{Deserialize, Serialize};

/// A GitHub Pull Request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequest {
    /// PR number.
    pub number: u64,

    /// PR title.
    pub title: String,

    /// PR body/description.
    pub body: Option<String>,

    /// PR state.
    pub state: PullRequestState,

    /// Whether this is a draft PR.
    pub draft: bool,

    /// Head branch name.
    pub head_branch: String,

    /// Base branch name.
    pub base_branch: String,

    /// PR URL.
    pub html_url: String,

    /// Whether the PR is mergeable (None if GitHub is still computing).
    pub mergeable: Option<bool>,

    /// The mergeable state (e.g., "clean", "dirty", "blocked", "behind").
    pub mergeable_state: Option<String>,
}

/// State of a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PullRequestState {
    /// PR is open.
    Open,
    /// PR was closed without merging.
    Closed,
    /// PR was merged.
    Merged,
}

/// A CI check run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckRun {
    /// Check name.
    pub name: String,

    /// Check status.
    pub status: CheckStatus,

    /// URL to view check details.
    pub details_url: Option<String>,
}

/// Status of a CI check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    /// Check is queued.
    Queued,
    /// Check is in progress.
    InProgress,
    /// Check completed successfully.
    Success,
    /// Check failed.
    Failure,
    /// Check was skipped.
    Skipped,
    /// Check was cancelled.
    Cancelled,
}

impl CheckStatus {
    /// Check if this status indicates success.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success | Self::Skipped)
    }

    /// Check if this status indicates failure.
    #[must_use]
    pub const fn is_failure(&self) -> bool {
        matches!(self, Self::Failure)
    }

    /// Check if this status indicates the check is still running.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        matches!(self, Self::Queued | Self::InProgress)
    }
}

/// Request to create a pull request.
#[derive(Debug, Serialize)]
pub struct CreatePullRequest {
    /// PR title.
    pub title: String,

    /// PR body.
    pub body: String,

    /// Head branch.
    pub head: String,

    /// Base branch.
    pub base: String,

    /// Whether to create as draft.
    pub draft: bool,
}

/// Request to update a pull request.
#[derive(Debug, Serialize)]
pub struct UpdatePullRequest {
    /// New title (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// New body (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,

    /// New base branch (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
}

/// Method used to merge a pull request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MergeMethod {
    /// Create a merge commit.
    Merge,
    /// Squash all commits into one.
    #[default]
    Squash,
    /// Rebase commits onto base.
    Rebase,
}

/// Request to merge a pull request.
#[derive(Debug, Serialize)]
pub struct MergePullRequest {
    /// Commit title (for squash/merge).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_title: Option<String>,

    /// Commit message (for squash/merge).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_message: Option<String>,

    /// Merge method.
    pub merge_method: MergeMethod,
}

/// Result of merging a pull request.
#[derive(Debug, Clone, Deserialize)]
pub struct MergeResult {
    /// SHA of the merge commit.
    pub sha: String,

    /// Whether the merge was successful.
    pub merged: bool,

    /// Message from the API.
    pub message: String,
}

/// A comment on an issue or pull request.
#[derive(Debug, Clone, Deserialize)]
pub struct IssueComment {
    /// Comment ID.
    pub id: u64,

    /// Comment body.
    pub body: Option<String>,
}

/// Request to create an issue/PR comment.
#[derive(Debug, Serialize)]
pub struct CreateComment {
    /// Comment body.
    pub body: String,
}

/// Request to update an issue/PR comment.
#[derive(Debug, Serialize)]
pub struct UpdateComment {
    /// New comment body.
    pub body: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_status_is_success() {
        assert!(CheckStatus::Success.is_success());
        assert!(CheckStatus::Skipped.is_success());
        assert!(!CheckStatus::Failure.is_success());
        assert!(!CheckStatus::Queued.is_success());
        assert!(!CheckStatus::InProgress.is_success());
        assert!(!CheckStatus::Cancelled.is_success());
    }

    #[test]
    fn test_check_status_is_failure() {
        assert!(CheckStatus::Failure.is_failure());
        assert!(!CheckStatus::Success.is_failure());
        assert!(!CheckStatus::Skipped.is_failure());
        assert!(!CheckStatus::Queued.is_failure());
        assert!(!CheckStatus::InProgress.is_failure());
        assert!(!CheckStatus::Cancelled.is_failure());
    }

    #[test]
    fn test_check_status_is_pending() {
        assert!(CheckStatus::Queued.is_pending());
        assert!(CheckStatus::InProgress.is_pending());
        assert!(!CheckStatus::Success.is_pending());
        assert!(!CheckStatus::Failure.is_pending());
        assert!(!CheckStatus::Skipped.is_pending());
        assert!(!CheckStatus::Cancelled.is_pending());
    }

    #[test]
    fn test_pull_request_state_equality() {
        assert_eq!(PullRequestState::Open, PullRequestState::Open);
        assert_eq!(PullRequestState::Closed, PullRequestState::Closed);
        assert_eq!(PullRequestState::Merged, PullRequestState::Merged);
        assert_ne!(PullRequestState::Open, PullRequestState::Closed);
    }

    #[test]
    fn test_merge_method_default() {
        assert_eq!(MergeMethod::default(), MergeMethod::Squash);
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_pull_request_state_serialization() {
        assert_eq!(
            serde_json::to_string(&PullRequestState::Open).unwrap(),
            "\"open\""
        );
        assert_eq!(
            serde_json::to_string(&PullRequestState::Closed).unwrap(),
            "\"closed\""
        );
        assert_eq!(
            serde_json::to_string(&PullRequestState::Merged).unwrap(),
            "\"merged\""
        );
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_check_status_serialization() {
        assert_eq!(
            serde_json::to_string(&CheckStatus::Queued).unwrap(),
            "\"queued\""
        );
        assert_eq!(
            serde_json::to_string(&CheckStatus::InProgress).unwrap(),
            "\"inprogress\""
        );
        assert_eq!(
            serde_json::to_string(&CheckStatus::Success).unwrap(),
            "\"success\""
        );
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_merge_method_serialization() {
        assert_eq!(
            serde_json::to_string(&MergeMethod::Merge).unwrap(),
            "\"merge\""
        );
        assert_eq!(
            serde_json::to_string(&MergeMethod::Squash).unwrap(),
            "\"squash\""
        );
        assert_eq!(
            serde_json::to_string(&MergeMethod::Rebase).unwrap(),
            "\"rebase\""
        );
    }
}
