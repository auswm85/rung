//! # rung-github
//!
//! GitHub API integration for Rung, providing PR management
//! and CI status fetching capabilities.
//!
//! # Architecture
//!
//! The crate provides both a concrete [`GitHubClient`] implementation and
//! a [`GitHubApi`] trait for dependency injection and testing.
//!
//! # Security
//!
//! Authentication tokens are stored using `SecretString` which automatically
//! zeroizes memory when dropped, reducing credential exposure in memory dumps.

mod auth;
mod client;
mod error;
mod traits;
mod types;

pub use auth::Auth;
pub use client::GitHubClient;
pub use error::{Error, Result};
pub use traits::GitHubApi;
// Re-export SecretString for constructing Auth::Token
pub use secrecy::SecretString;
pub use types::{
    CheckRun, CheckStatus, CreateComment, CreatePullRequest, IssueComment, MergeMethod,
    MergePullRequest, MergeResult, PullRequest, PullRequestState, UpdateComment, UpdatePullRequest,
};
