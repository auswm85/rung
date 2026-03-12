use super::types::{SyncAction, SyncPlan};
use crate::error::Result;
use crate::stack::Stack;

/// Create a sync plan for the given stack.
///
/// Analyzes which branches need rebasing based on their parent's current position.
/// Uses proactive cascade: when a branch needs rebasing, all its descendants are
/// automatically included in the plan, ensuring one sync handles the entire stack.
///
/// Branches are processed in stack order (parents before children) to ensure
/// each branch is rebased onto the correct target.
///
/// Stale branches (in stack but not in git) are detected and can be cleaned up
/// by calling `remove_stale_branches`.
///
/// # Errors
/// Returns error if git operations fail.
pub fn create_sync_plan(
    repo: &impl rung_git::GitOps,
    stack: &Stack,
    base_branch: &str,
) -> Result<SyncPlan> {
    let mut actions = Vec::new();

    // Track branches that need rebasing (including cascaded descendants)
    let mut needs_rebase: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Process branches in stack order (parents before children)
    for branch in &stack.branches {
        // Skip branches that don't exist locally (stale branches)
        // These will be handled separately by remove_stale_branches
        if !repo.branch_exists(&branch.name) {
            continue;
        }

        // Determine the parent branch name
        let parent_name = branch.parent.as_deref().unwrap_or(base_branch);

        // Skip if parent doesn't exist (external branch like main might not exist locally)
        if !repo.branch_exists(parent_name) && branch.parent.is_none() {
            // Base branch doesn't exist - this is an error
            return Err(crate::error::Error::BranchNotFound(parent_name.to_string()));
        }

        // If parent is a stack branch that doesn't exist, skip this branch too
        // (it will be handled when we clean up stale branches)
        if branch.parent.is_some() && !repo.branch_exists(parent_name) {
            continue;
        }

        // Get commits
        let branch_commit = repo.branch_commit(&branch.name)?;
        let parent_commit = repo.branch_commit(parent_name)?;

        // Find where this branch diverged from parent
        let merge_base = repo.merge_base(branch_commit, parent_commit)?;

        // Determine if this branch needs rebasing:
        // 1. Its merge_base differs from parent tip (direct divergence), OR
        // 2. It was marked for cascade rebase (parent was rebased)
        let needs_direct_rebase = merge_base != parent_commit;
        let needs_cascade_rebase = needs_rebase.contains(branch.name.as_str());

        if needs_direct_rebase || needs_cascade_rebase {
            actions.push(SyncAction {
                branch: branch.name.to_string(),
                old_base: merge_base.to_string(),
                new_base: parent_commit.to_string(),
            });

            // Proactive cascade: mark all descendants as needing rebase
            // This ensures the entire sub-tree is synced in one pass
            for descendant in stack.descendants(&branch.name) {
                needs_rebase.insert(descendant.name.to_string());
            }
        }
    }

    Ok(SyncPlan { branches: actions })
}
