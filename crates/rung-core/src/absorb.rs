//! Absorb command logic for automatic fixup commit creation.
//!
//! Analyzes staged changes and automatically creates fixup commits
//! targeting the appropriate commits in the local history.

use rung_git::{Hunk, Oid, Repository};

use crate::State;
use crate::error::Result;

/// A planned fixup operation mapping a hunk to its target commit.
#[derive(Debug, Clone)]
pub struct AbsorbAction {
    /// The hunk to be absorbed.
    pub hunk: Hunk,
    /// The target commit to fixup.
    pub target_commit: Oid,
    /// Short commit message of the target.
    pub target_message: String,
}

/// Result of analyzing staged changes for absorption.
#[derive(Debug, Clone)]
pub struct AbsorbPlan {
    /// Actions that can be executed (hunk mapped to valid target).
    pub actions: Vec<AbsorbAction>,
    /// Hunks that couldn't be mapped to a target commit.
    pub unmapped: Vec<UnmappedHunk>,
}

/// A hunk that couldn't be mapped to a target commit.
#[derive(Debug, Clone)]
pub struct UnmappedHunk {
    /// The hunk that couldn't be absorbed.
    pub hunk: Hunk,
    /// Reason why mapping failed.
    pub reason: UnmapReason,
}

/// Reason why a hunk couldn't be mapped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnmapReason {
    /// New file - no blame history.
    NewFile,
    /// Insert-only hunk (no deleted lines to blame).
    InsertOnly,
    /// Lines touched by multiple commits.
    MultipleCommits,
    /// Target commit is not in the rebaseable range.
    CommitNotInStack,
    /// Target commit is already on the base branch.
    CommitOnBaseBranch,
    /// Blame query failed.
    BlameError(String),
}

impl std::fmt::Display for UnmapReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NewFile => write!(f, "new file (no blame history)"),
            Self::InsertOnly => write!(f, "insert-only hunk (no lines to blame)"),
            Self::MultipleCommits => write!(f, "lines touched by multiple commits"),
            Self::CommitNotInStack => write!(f, "target commit not in stack"),
            Self::CommitOnBaseBranch => write!(f, "target commit already on base branch"),
            Self::BlameError(e) => write!(f, "blame error: {e}"),
        }
    }
}

/// Result of executing an absorb plan.
#[derive(Debug)]
pub struct AbsorbResult {
    /// Number of fixup commits created.
    pub fixups_created: usize,
    /// Commits that were targeted.
    pub targeted_commits: Vec<Oid>,
}

/// Create an absorb plan by analyzing staged changes.
///
/// For each staged hunk, this function:
/// 1. Queries git blame to find which commit last touched those lines
/// 2. Validates the target commit is within the rebaseable range
/// 3. Creates an action mapping the hunk to its target
///
/// # Arguments
/// * `repo` - The git repository
/// * `state` - Rung state for stack information
/// * `base_branch` - The base branch name (e.g., "main")
///
/// # Errors
/// Returns error if git operations fail.
pub fn create_absorb_plan(
    repo: &Repository,
    state: &State,
    base_branch: &str,
) -> Result<AbsorbPlan> {
    let mut actions = Vec::new();
    let mut unmapped = Vec::new();

    // Get staged hunks
    let hunks = repo.staged_diff_hunks()?;

    if hunks.is_empty() {
        return Ok(AbsorbPlan { actions, unmapped });
    }

    // Get the base branch commit for validation
    let base_commit = repo
        .branch_commit(base_branch)
        .or_else(|_| repo.remote_branch_commit(base_branch))?;

    // Get current HEAD
    let current_branch = repo.current_branch()?;
    let head_commit = repo.branch_commit(&current_branch)?;

    // Get commits in the rebaseable range (between base and HEAD)
    let rebaseable_commits: std::collections::HashSet<Oid> = repo
        .commits_between(base_commit, head_commit)?
        .into_iter()
        .collect();

    // Load stack (reserved for future validation enhancements)
    let _stack = state.load_stack()?;

    for hunk in hunks {
        // New files have no blame history
        if hunk.is_new_file {
            unmapped.push(UnmappedHunk {
                hunk,
                reason: UnmapReason::NewFile,
            });
            continue;
        }

        // Determine blame range
        // For insert-only hunks (old_lines == 0), blame an adjacent line instead
        let (blame_start, blame_end) = if hunk.old_lines == 0 {
            // Insert-only hunk: blame the line at old_start (or line 1 if at file start)
            // old_start is the line number where insertion happens (1-indexed)
            // If old_start is 0, the insertion is at the very start; blame line 1
            let line = hunk.old_start.max(1);
            (line, line)
        } else {
            (
                hunk.old_start,
                hunk.old_start
                    .saturating_add(hunk.old_lines)
                    .saturating_sub(1),
            )
        };

        // Query blame for the original lines (or adjacent line for insert-only hunks)
        let blame_result = match repo.blame_lines(&hunk.file_path, blame_start, blame_end) {
            Ok(results) => results,
            Err(e) => {
                unmapped.push(UnmappedHunk {
                    hunk,
                    reason: UnmapReason::BlameError(e.to_string()),
                });
                continue;
            }
        };

        // Check if all blamed lines point to the same commit
        if blame_result.is_empty() {
            unmapped.push(UnmappedHunk {
                hunk,
                reason: UnmapReason::BlameError("no blame results".to_string()),
            });
            continue;
        }

        if blame_result.len() > 1 {
            unmapped.push(UnmappedHunk {
                hunk,
                reason: UnmapReason::MultipleCommits,
            });
            continue;
        }

        let target = &blame_result[0];

        // Validate target is in rebaseable range
        if !rebaseable_commits.contains(&target.commit) {
            // Check if it's on base branch
            if repo
                .is_ancestor(target.commit, base_commit)
                .unwrap_or(false)
                || target.commit == base_commit
            {
                unmapped.push(UnmappedHunk {
                    hunk,
                    reason: UnmapReason::CommitOnBaseBranch,
                });
            } else {
                unmapped.push(UnmappedHunk {
                    hunk,
                    reason: UnmapReason::CommitNotInStack,
                });
            }
            continue;
        }

        actions.push(AbsorbAction {
            hunk,
            target_commit: target.commit,
            target_message: target.message.clone(),
        });
    }

    Ok(AbsorbPlan { actions, unmapped })
}

/// Execute an absorb plan by creating fixup commits.
///
/// Creates a single fixup commit targeting the identified commit.
/// This modifies the repository by creating new commits.
///
/// # Errors
/// Returns error if commit creation fails or if hunks target multiple commits.
/// Multiple targets are not supported because git commit consumes the entire
/// staging area, making it impossible to create separate fixup commits for
/// different targets without per-hunk staging (a future enhancement).
pub fn execute_absorb(repo: &Repository, plan: &AbsorbPlan) -> Result<AbsorbResult> {
    if plan.actions.is_empty() {
        return Ok(AbsorbResult {
            fixups_created: 0,
            targeted_commits: vec![],
        });
    }

    // Group actions by target commit
    let mut by_target: std::collections::HashMap<Oid, Vec<&AbsorbAction>> =
        std::collections::HashMap::new();
    for action in &plan.actions {
        by_target
            .entry(action.target_commit)
            .or_default()
            .push(action);
    }

    // Reject multi-target plans - git commit consumes the entire index,
    // so we can't create separate fixup commits without per-hunk staging.
    if by_target.len() > 1 {
        let target_descriptions: Vec<String> = by_target
            .iter()
            .map(|(oid, actions)| {
                let short_sha = &oid.to_string()[..8];
                let msg = &actions[0].target_message;
                format!("{short_sha} ({msg})")
            })
            .collect();
        return Err(crate::error::Error::Absorb(format!(
            "staged changes target {} different commits; selective hunk staging not supported. \
             Targets: {}. Stage fewer changes so all hunks target the same commit.",
            by_target.len(),
            target_descriptions.join(", ")
        )));
    }

    let mut targeted_commits = Vec::new();

    // Create fixup commit for the single target
    for target in by_target.keys() {
        repo.create_fixup_commit(*target)?;
        targeted_commits.push(*target);
    }

    Ok(AbsorbResult {
        fixups_created: by_target.len(),
        targeted_commits,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_unmap_reason_display() {
        assert_eq!(
            UnmapReason::NewFile.to_string(),
            "new file (no blame history)"
        );
        assert_eq!(
            UnmapReason::InsertOnly.to_string(),
            "insert-only hunk (no lines to blame)"
        );
        assert_eq!(
            UnmapReason::MultipleCommits.to_string(),
            "lines touched by multiple commits"
        );
        assert_eq!(
            UnmapReason::CommitNotInStack.to_string(),
            "target commit not in stack"
        );
        assert_eq!(
            UnmapReason::CommitOnBaseBranch.to_string(),
            "target commit already on base branch"
        );
        assert_eq!(
            UnmapReason::BlameError("test".to_string()).to_string(),
            "blame error: test"
        );
    }

    #[test]
    fn test_absorb_plan_empty() {
        let plan = AbsorbPlan {
            actions: vec![],
            unmapped: vec![],
        };
        assert!(plan.actions.is_empty());
        assert!(plan.unmapped.is_empty());
    }
}
