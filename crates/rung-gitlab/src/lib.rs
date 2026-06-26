//! # rung-gitlab
//!
//! GitLab API integration for Rung, providing merge request management and CI
//! status fetching capabilities.
//!
//! # Architecture
//!
//! This crate provides the concrete [`GitLabClient`], which implements the
//! [`ForgeApi`] trait defined in the `rung-forge` contract crate. The forge
//! types and trait are re-exported here for convenience. Authentication and the
//! credential-bearing HTTP client are functional today; the merge-request,
//! pipeline, and comment methods are currently skeletons (see issue #170).
//!
//! # Security
//!
//! Authentication tokens are stored using `SecretString`, which automatically
//! zeroizes memory when dropped, reducing credential exposure in memory dumps.

mod auth;
mod client;

pub use auth::Auth;
pub use client::{GitLabClient, GitLabUser};
// Re-export SecretString for constructing `Auth::Token`.
pub use secrecy::SecretString;
// Re-export the forge contract so `rung_gitlab::{...}` mirrors `rung_github`.
// `ForgeError` is re-exported as `Error` for parity with the GitHub crate.
pub use rung_forge::{
    CheckRun, CheckStatus, CreateComment, CreatePullRequest, ForgeApi, ForgeError as Error,
    IssueComment, MergeMethod, MergePullRequest, MergeResult, PullRequest, PullRequestState,
    RepoId, Result, UpdateComment, UpdatePullRequest,
};
