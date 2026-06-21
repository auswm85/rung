//! # rung-forge
//!
//! Forge-agnostic contract for Rung. This crate defines the types and the
//! [`ForgeApi`] trait that every code-hosting backend implements, so that
//! `rung-core` and `rung-cli` depend only on this neutral contract rather than
//! on any specific forge.
//!
//! Concrete backends live in their own crates (e.g. `rung-github`) and depend
//! on `rung-forge` — never on each other.

mod error;
mod traits;
mod types;

pub use error::{ForgeError, Result};
pub use traits::ForgeApi;
pub use types::{
    CheckRun, CheckStatus, CreateComment, CreatePullRequest, IssueComment, MergeMethod,
    MergePullRequest, MergeResult, PullRequest, PullRequestState, UpdateComment, UpdatePullRequest,
};
