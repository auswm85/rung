//! Service layer for business logic with dependency injection.
//!
//! This module contains services that encapsulate business logic and accept
//! trait-based dependencies, enabling testing with mock implementations.

pub mod merge;
pub mod restack;
pub mod submit;
pub mod sync;

pub use submit::{
    BranchSubmitResult, PlannedBranchAction, SubmitAction, SubmitConfig, SubmitPlan, SubmitService,
};

// Re-exports for services not yet wired up to commands
#[allow(unused_imports)]
pub use merge::{DescendantResult, MergeConfig, MergeResult, MergeService};
#[allow(unused_imports)]
pub use restack::{DivergenceInfo, RestackConfig, RestackPlan, RestackResult, RestackService};
#[allow(unused_imports)]
pub use sync::{PushResult, SyncConfig, SyncPlanResult, SyncService};
