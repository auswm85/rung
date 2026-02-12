//! Service layer for business logic with dependency injection.
//!
//! This module contains services that encapsulate business logic and accept
//! trait-based dependencies, enabling testing with mock implementations.

pub mod absorb;
pub mod adopt;
pub mod create;
pub mod doctor;
pub mod fold;
pub mod log;
pub mod merge;
pub mod restack;
pub mod split;
pub mod status;
pub mod submit;
pub mod sync;

#[cfg(test)]
pub mod test_mocks;

pub use absorb::AbsorbService;
pub use adopt::AdoptService;
pub use create::CreateService;
pub use doctor::{CheckResult, DiagnosticReport, DoctorService, Issue, Severity};
pub use log::{CommitInfo, LogResult, LogService};
pub use merge::MergeService;
pub use restack::{DivergenceInfo, RestackConfig, RestackError, RestackService};
pub use split::SplitService;
pub use status::{BranchStatusInfo, RemoteDivergenceInfo, StatusService};
pub use submit::{
    BranchSubmitResult, PlannedBranchAction, SubmitAction, SubmitConfig, SubmitPlan, SubmitService,
};
pub use sync::SyncService;
