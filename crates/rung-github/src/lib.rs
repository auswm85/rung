//! # rung-github
//!
//! GitHub API integration for Rung, providing PR management
//! and CI status fetching capabilities.

mod auth;
mod client;
mod error;
mod types;

pub use auth::Auth;
pub use client::GitHubClient;
pub use error::{Error, Result};
pub use types::{
    CheckRun, CheckStatus, CreateComment, CreatePullRequest, IssueComment, MergeMethod,
    MergePullRequest, MergeResult, PullRequest, PullRequestState, UpdateComment, UpdatePullRequest,
};
