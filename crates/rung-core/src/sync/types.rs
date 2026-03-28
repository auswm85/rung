/// Result of a sync operation.
#[derive(Debug)]
pub enum SyncResult {
    /// Stack was already up-to-date.
    AlreadySynced,

    /// Sync completed successfully.
    Complete {
        /// Number of branches rebased.
        branches_rebased: usize,
        /// Backup ID that can be used for undo.
        backup_id: String,
    },

    /// Sync paused due to conflict.
    Paused {
        /// Branch where conflict occurred.
        at_branch: String,
        /// Files with conflicts.
        conflict_files: Vec<String>,
        /// Backup ID for potential undo.
        backup_id: String,
    },
}

/// Plan for syncing a stack.
#[derive(Debug)]
pub struct SyncPlan {
    /// Branches to rebase, in order.
    pub branches: Vec<SyncAction>,
}

/// A single rebase action in the sync plan.
#[derive(Debug)]
pub struct SyncAction {
    /// Branch to rebase.
    pub branch: String,
    /// Current base commit (will be replaced).
    pub old_base: String,
    /// New base commit (parent's new tip).
    pub new_base: String,
    /// Parent branch name (for display purposes).
    pub parent_branch: String,
}

impl SyncPlan {
    /// Check if the plan is empty (nothing to sync).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.branches.is_empty()
    }
}

/// Branches that were found to be stale (in stack but not in git).
#[derive(Debug, Default)]
pub struct StaleBranches {
    /// Names of branches that were removed from the stack.
    pub removed: Vec<String>,
}

/// Result of reconciling merged PRs and validating PR bases.
#[derive(Debug, Default)]
pub struct ReconcileResult {
    /// Branches removed because their PRs merged.
    pub merged: Vec<MergedBranch>,
    /// Branches re-parented to new parents.
    pub reparented: Vec<ReparentedBranch>,
    /// PRs repaired due to ghost parent detection (base mismatch).
    pub repaired: Vec<ReparentedBranch>,
}

/// A branch whose PR was merged.
#[derive(Debug)]
pub struct MergedBranch {
    /// Branch name.
    pub name: String,
    /// PR number that was merged.
    pub pr_number: u64,
    /// Branch it was merged into.
    pub merged_into: String,
}

/// A branch that was re-parented due to its parent being merged.
#[derive(Debug)]
pub struct ReparentedBranch {
    /// Branch name.
    pub name: String,
    /// Previous parent branch.
    pub old_parent: String,
    /// New parent branch.
    pub new_parent: String,
    /// PR number (if any) that needs base branch update.
    pub pr_number: Option<u64>,
}

/// Information about a PR that was merged externally (e.g., via GitHub UI).
#[derive(Debug)]
pub struct ExternalMergeInfo {
    /// Branch name that was merged.
    pub branch_name: String,
    /// PR number that was merged.
    pub pr_number: u64,
    /// Branch it was merged into.
    pub merged_into: String,
}

/// Result of an undo operation.
#[derive(Debug)]
pub struct UndoResult {
    /// Number of branches restored.
    pub branches_restored: usize,
    /// The backup ID that was used.
    pub backup_id: String,
}

/// Predicted conflicts for a sync operation.
#[derive(Debug, Default)]
pub struct SyncConflictPrediction {
    /// Branches with predicted conflicts.
    pub branches: Vec<BranchConflictPrediction>,
}

impl SyncConflictPrediction {
    /// Check if there are any predicted conflicts.
    #[must_use]
    pub const fn has_conflicts(&self) -> bool {
        !self.branches.is_empty()
    }

    /// Get total number of branches with conflicts.
    #[must_use]
    pub const fn conflict_count(&self) -> usize {
        self.branches.len()
    }
}

/// Predicted conflicts for a single branch.
#[derive(Debug)]
pub struct BranchConflictPrediction {
    /// Branch that would conflict.
    pub branch: String,
    /// Target branch it's being rebased onto.
    pub onto: String,
    /// Individual commit conflicts.
    pub conflicts: Vec<CommitConflictPrediction>,
}

impl BranchConflictPrediction {
    /// Get all unique conflicting files across all commits.
    #[must_use]
    pub fn conflicting_files(&self) -> Vec<&str> {
        let mut files: Vec<&str> = self
            .conflicts
            .iter()
            .flat_map(|c| c.files.iter().map(String::as_str))
            .collect();
        files.sort_unstable();
        files.dedup();
        files
    }
}

/// Predicted conflict for a single commit.
#[derive(Debug)]
pub struct CommitConflictPrediction {
    /// Commit hash (short form).
    pub commit_hash: String,
    /// Commit message summary.
    pub commit_summary: String,
    /// Files that would conflict.
    pub files: Vec<String>,
}
