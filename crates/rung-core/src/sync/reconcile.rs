use super::types::{
    ExternalMergeInfo, MergedBranch, ReconcileResult, ReparentedBranch, StaleBranches,
};
use crate::error::Result;
use crate::traits::StateStore;

/// Reconcile the stack after PRs were merged externally.
///
/// For each merged branch:
/// 1. Re-parent its children to the merge target
/// 2. Remove the merged branch from the stack
///
/// This function does NOT call GitHub - the caller provides the list of
/// merged PRs (obtained from GitHub API).
///
/// # Errors
/// Returns error if stack operations fail.
pub fn reconcile_merged(
    state: &impl StateStore,
    merged_prs: &[ExternalMergeInfo],
) -> Result<ReconcileResult> {
    if merged_prs.is_empty() {
        return Ok(ReconcileResult::default());
    }

    let mut stack = state.load_stack()?;
    let mut result = ReconcileResult::default();

    for merge_info in merged_prs {
        // Find children of the merged branch (collect names first to avoid borrow issues)
        let children: Vec<String> = stack
            .children_of(&merge_info.branch_name)
            .iter()
            .map(|b| b.name.to_string())
            .collect();

        // Re-parent children to the merge target
        for child_name in children {
            if let Some(child) = stack.find_branch_mut(&child_name) {
                let old_parent = child
                    .parent
                    .as_ref()
                    .map_or_else(String::new, ToString::to_string);
                let pr_number = child.pr;
                // Create validated BranchName for the new parent
                let new_parent =
                    crate::BranchName::new(merge_info.merged_into.clone()).map_err(|_| {
                        crate::error::Error::BranchNotFound(merge_info.merged_into.clone())
                    })?;
                child.parent = Some(new_parent);

                result.reparented.push(ReparentedBranch {
                    name: child_name,
                    old_parent,
                    new_parent: merge_info.merged_into.clone(),
                    pr_number,
                });
            }
        }

        // Remove merged branch from stack
        stack.remove_branch(&merge_info.branch_name);

        result.merged.push(MergedBranch {
            name: merge_info.branch_name.clone(),
            pr_number: merge_info.pr_number,
            merged_into: merge_info.merged_into.clone(),
        });
    }

    // Save updated stack
    state.save_stack(&stack)?;

    Ok(result)
}

/// Find and remove stale branches from the stack.
///
/// A stale branch is one that exists in `stack.json` but not in the local git repository.
/// This can happen if a branch was deleted externally or if the stack got out of sync.
///
/// Returns information about the branches that were removed.
///
/// # Errors
/// Returns error if stack operations fail.
pub fn remove_stale_branches(
    repo: &impl rung_git::GitOps,
    state: &impl StateStore,
) -> Result<StaleBranches> {
    let mut stack = state.load_stack()?;
    let mut removed = Vec::new();

    // Find branches that don't exist locally
    let missing: Vec<String> = stack
        .branches
        .iter()
        .filter(|b| !repo.branch_exists(&b.name))
        .map(|b| b.name.to_string())
        .collect();

    if missing.is_empty() {
        return Ok(StaleBranches::default());
    }

    // For each stale branch, re-parent its children to its parent
    for missing_name in &missing {
        let missing_parent = stack
            .find_branch(missing_name)
            .and_then(|b| b.parent.clone());

        // Re-parent children of this stale branch
        for branch in &mut stack.branches {
            if branch.parent.as_ref().is_some_and(|p| p == missing_name) {
                branch.parent.clone_from(&missing_parent);
            }
        }

        removed.push(missing_name.clone());
    }

    // Remove stale branches from stack
    stack
        .branches
        .retain(|b| !missing.contains(&b.name.to_string()));
    state.save_stack(&stack)?;

    Ok(StaleBranches { removed })
}
