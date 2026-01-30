//! Service layer for business logic with dependency injection.
//!
//! This module contains services that encapsulate business logic and accept
//! trait-based dependencies, enabling testing with mock implementations.

pub mod absorb;
pub mod create;
pub mod doctor;
pub mod merge;
pub mod restack;
pub mod status;
pub mod submit;
pub mod sync;

pub use absorb::AbsorbService;
pub use create::CreateService;
pub use doctor::{CheckResult, DoctorService, Issue, Severity};
pub use merge::MergeService;
pub use restack::{DivergenceInfo, RestackConfig, RestackService};
pub use status::{BranchStatusInfo, RemoteDivergenceInfo, StackStatus, StatusService};
pub use submit::{
    BranchSubmitResult, PlannedBranchAction, SubmitAction, SubmitConfig, SubmitPlan, SubmitService,
};
pub use sync::SyncService;
