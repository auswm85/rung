//! # rung-core
//!
//! Core library for Rung providing stack management, state persistence,
//! and the sync engine for dependent PR stacks.

pub mod branch_name;
pub mod config;
pub mod error;
pub mod stack;
pub mod state;
pub mod sync;

pub use branch_name::{BranchName, slugify};
pub use config::Config;
pub use error::{Error, Result};
pub use stack::{BranchState, Stack, StackBranch};
pub use state::State;
