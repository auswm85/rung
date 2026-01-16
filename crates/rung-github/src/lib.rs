//! # rung-github
//!
//! GitHub API integration for Rung, providing PR management
//! and CI status fetching capabilities.
//!
//! # Security
//!
//! Authentication tokens are stored using `SecretString` which automatically
//! zeroizes memory when dropped, reducing credential exposure in memory dumps.

mod auth;
mod client;
mod error;
mod types;

pub use auth::Auth;
pub use client::GitHubClient;
pub use error::{Error, Result};
// Re-export SecretString for constructing Auth::Token
pub use secrecy::SecretString;
pub use types::{
    CheckRun, CheckStatus, CreateComment, CreatePullRequest, IssueComment, MergeMethod,
    MergePullRequest, MergeResult, PullRequest, PullRequestState, UpdateComment, UpdatePullRequest,
};
