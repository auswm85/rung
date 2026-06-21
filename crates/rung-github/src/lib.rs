//! # rung-github
//!
//! GitHub API integration for Rung, providing PR management
//! and CI status fetching capabilities.
//!
//! # Architecture
//!
//! This crate provides the concrete [`GitHubClient`], which implements the
//! [`ForgeApi`] trait defined in the `rung-forge` contract crate. The forge
//! types and trait are re-exported here for convenience.
//!
//! # Security
//!
//! Authentication tokens are stored using `SecretString` which automatically
//! zeroizes memory when dropped, reducing credential exposure in memory dumps.

mod auth;
mod client;

pub use auth::Auth;
pub use client::GitHubClient;
// Re-export SecretString for constructing Auth::Token
pub use secrecy::SecretString;
// Re-export the forge contract so existing `rung_github::{...}` paths keep working.
// `ForgeError` is re-exported as `Error` for backward compatibility.
pub use rung_forge::{
    CheckRun, CheckStatus, CreateComment, CreatePullRequest, ForgeApi, ForgeError as Error,
    IssueComment, MergeMethod, MergePullRequest, MergeResult, PullRequest, PullRequestState,
    Result, UpdateComment, UpdatePullRequest,
};
