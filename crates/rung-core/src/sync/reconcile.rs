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
/// Merged PRs are processed in topological order (parents before children)
/// to ensure correct re-parenting when both a parent and its descendant
/// are merged in the same batch.
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

    // Sort merged PRs topologically (parents before children) to handle cases
    // where both a parent and its descendant are merged in the same batch
    let sorted_merged = topological_sort_merged(merged_prs, &stack);

    for merge_info in sorted_merged {
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
                let new_parent = crate::BranchName::new(merge_info.merged_into.clone())?;
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

    // Build a map of branch name -> parent for efficient ancestor lookup
    let parent_map: std::collections::HashMap<String, Option<crate::BranchName>> = stack
        .branches
        .iter()
        .map(|b| (b.name.to_string(), b.parent.clone()))
        .collect();

    // For each stale branch, re-parent its children to the first valid ancestor
    for missing_name in &missing {
        // Find the first ancestor that is not being removed
        let mut resolved_parent = parent_map.get(missing_name).and_then(Clone::clone);
        while let Some(ref parent) = resolved_parent {
            let parent_str = parent.to_string();
            if !missing.contains(&parent_str) {
                // Found a valid ancestor that's not being removed
                break;
            }
            // This parent is also being removed, walk up the chain
            resolved_parent = parent_map.get(&parent_str).and_then(Clone::clone);
        }

        // Re-parent children of this stale branch to the resolved ancestor
        for branch in &mut stack.branches {
            if branch.parent.as_ref().is_some_and(|p| p == missing_name) {
                branch.parent.clone_from(&resolved_parent);
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

/// Sort merged PRs topologically so parents are processed before children.
///
/// This ensures correct re-parenting when both a parent and its descendant
/// are merged in the same batch. Uses Kahn's algorithm with the stack's
/// parent relationships.
fn topological_sort_merged<'a>(
    merged_prs: &'a [ExternalMergeInfo],
    stack: &crate::stack::Stack,
) -> Vec<&'a ExternalMergeInfo> {
    // Build a set of branch names being merged for quick lookup
    let merged_names: std::collections::HashSet<&str> =
        merged_prs.iter().map(|m| m.branch_name.as_str()).collect();

    // Build parent lookup from stack
    let parent_map: std::collections::HashMap<&str, Option<&str>> = stack
        .branches
        .iter()
        .map(|b| (b.name.as_str(), b.parent.as_deref()))
        .collect();

    let mut result = Vec::with_capacity(merged_prs.len());
    let mut remaining: Vec<&ExternalMergeInfo> = merged_prs.iter().collect();
    let mut processed: std::collections::HashSet<&str> = std::collections::HashSet::new();

    // Repeatedly find merged PRs whose parent (if also being merged) is already processed
    while !remaining.is_empty() {
        let prev_len = remaining.len();

        remaining.retain(|merge_info| {
            let branch_name = merge_info.branch_name.as_str();

            // Get parent from stack, defaulting to None (base branch)
            let parent = parent_map.get(branch_name).copied().flatten();

            // A branch can be processed if:
            // 1. Its parent is not in the merged set (external dependency), OR
            // 2. Its parent has already been processed
            let parent_ready =
                parent.is_none_or(|p| !merged_names.contains(p) || processed.contains(p));

            if parent_ready {
                processed.insert(branch_name);
                result.push(*merge_info);
                false // Remove from remaining
            } else {
                true // Keep in remaining
            }
        });

        // If no progress was made, there's a cycle - just add remaining in order
        if remaining.len() == prev_len {
            result.append(&mut remaining);
            break;
        }
    }

    result
}
