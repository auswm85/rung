//! Service layer for business logic with dependency injection.
//!
//! This module contains services that encapsulate business logic and accept
//! trait-based dependencies, enabling testing with mock implementations.

pub mod doctor;
pub mod merge;
pub mod restack;
pub mod submit;
pub mod sync;

pub use doctor::{CheckResult, DoctorService, Issue, Severity};
pub use merge::MergeService;
pub use restack::{DivergenceInfo, RestackConfig, RestackService};
pub use submit::{
    BranchSubmitResult, PlannedBranchAction, SubmitAction, SubmitConfig, SubmitPlan, SubmitService,
};
pub use sync::SyncService;
