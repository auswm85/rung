//! Service layer for business logic with dependency injection.
//!
//! This module contains services that encapsulate business logic and accept
//! trait-based dependencies, enabling testing with mock implementations.

pub mod submit;

pub use submit::{
    BranchSubmitResult, PlannedBranchAction, SubmitAction, SubmitConfig, SubmitPlan, SubmitService,
};
