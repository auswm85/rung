//! # rung-git
//!
//! Git operations abstraction layer for Rung, built on git2-rs.
//! Provides high-level operations for branch management, rebasing,
//! and repository state inspection.

mod error;
mod repository;

pub use error::{Error, Result};
pub use git2::Oid;
pub use repository::Repository;
