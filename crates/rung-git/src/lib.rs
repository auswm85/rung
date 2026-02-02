//! # rung-git
//!
//! Git operations abstraction layer for Rung, built on git2-rs.
//! Provides high-level operations for branch management, rebasing,
//! and repository state inspection.
//!
//! # Architecture
//!
//! The crate provides both a concrete [`Repository`] implementation and
//! a [`GitOps`] trait for dependency injection and testing.

mod absorb;
mod error;
mod repository;
mod traits;

pub use absorb::{BlameResult, Hunk};
pub use error::{Error, Result};
pub use git2::Oid;
pub use repository::{RemoteDivergence, Repository};
pub use traits::{AbsorbOps, GitOps};
