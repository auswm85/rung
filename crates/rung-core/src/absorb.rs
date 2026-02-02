//! Absorb command logic for automatic fixup commit creation.
//!
//! Analyzes staged changes and automatically creates fixup commits
//! targeting the appropriate commits in the local history.

use rung_git::{AbsorbOps, BlameResult, Hunk, Oid};

use crate::StateStore;
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
/// * `repo` - The git repository (implementing `AbsorbOps`)
/// * `state` - Rung state for stack information (implementing `StateStore`)
/// * `base_branch` - The base branch name (e.g., "main")
///
/// # Errors
/// Returns error if git operations fail.
pub fn create_absorb_plan<G, S>(repo: &G, state: &S, base_branch: &str) -> Result<AbsorbPlan>
where
    G: AbsorbOps,
    S: StateStore,
{
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
        let blame_result: Vec<BlameResult> =
            match repo.blame_lines(&hunk.file_path, blame_start, blame_end) {
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
pub fn execute_absorb<G: AbsorbOps>(repo: &G, plan: &AbsorbPlan) -> Result<AbsorbResult> {
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
                let oid_str = oid.to_string();
                let short_sha = oid_str.get(..8).unwrap_or(&oid_str);
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
#[allow(clippy::unwrap_used, clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::stack::Stack;
    use crate::state::{RestackState, SyncState};
    use rung_git::{GitOps, RemoteDivergence};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::Path;

    // Mock implementation for AbsorbOps
    struct MockRepo {
        hunks: Vec<Hunk>,
        blame_results: HashMap<String, Vec<BlameResult>>,
        blame_errors: HashMap<String, String>,
        branch_commits: HashMap<String, Oid>,
        commits_between: Vec<Oid>,
        current_branch: String,
        is_ancestor_results: HashMap<(Oid, Oid), bool>,
        fixup_commits_created: RefCell<Vec<Oid>>,
    }

    impl Default for MockRepo {
        fn default() -> Self {
            Self {
                hunks: vec![],
                blame_results: HashMap::new(),
                blame_errors: HashMap::new(),
                branch_commits: HashMap::new(),
                commits_between: vec![],
                current_branch: "feature".to_string(),
                is_ancestor_results: HashMap::new(),
                fixup_commits_created: RefCell::new(vec![]),
            }
        }
    }

    impl GitOps for MockRepo {
        fn workdir(&self) -> Option<&Path> {
            None
        }
        fn current_branch(&self) -> rung_git::Result<String> {
            Ok(self.current_branch.clone())
        }
        fn head_detached(&self) -> rung_git::Result<bool> {
            Ok(false)
        }
        fn is_rebasing(&self) -> bool {
            false
        }
        fn branch_exists(&self, _name: &str) -> bool {
            true
        }
        fn create_branch(&self, _name: &str) -> rung_git::Result<Oid> {
            unimplemented!()
        }
        fn checkout(&self, _branch: &str) -> rung_git::Result<()> {
            Ok(())
        }
        fn delete_branch(&self, _name: &str) -> rung_git::Result<()> {
            unimplemented!()
        }
        fn list_branches(&self) -> rung_git::Result<Vec<String>> {
            unimplemented!()
        }
        fn branch_commit(&self, branch: &str) -> rung_git::Result<Oid> {
            self.branch_commits
                .get(branch)
                .copied()
                .ok_or_else(|| rung_git::Error::BranchNotFound(branch.to_string()))
        }
        fn remote_branch_commit(&self, branch: &str) -> rung_git::Result<Oid> {
            self.branch_commits
                .get(&format!("origin/{branch}"))
                .copied()
                .ok_or_else(|| rung_git::Error::BranchNotFound(branch.to_string()))
        }
        fn branch_commit_message(&self, _branch: &str) -> rung_git::Result<String> {
            unimplemented!()
        }
        fn merge_base(&self, _one: Oid, _two: Oid) -> rung_git::Result<Oid> {
            unimplemented!()
        }
        fn commits_between(&self, _from: Oid, _to: Oid) -> rung_git::Result<Vec<Oid>> {
            Ok(self.commits_between.clone())
        }
        fn count_commits_between(&self, _from: Oid, _to: Oid) -> rung_git::Result<usize> {
            unimplemented!()
        }
        fn is_clean(&self) -> rung_git::Result<bool> {
            Ok(true)
        }
        fn require_clean(&self) -> rung_git::Result<()> {
            Ok(())
        }
        fn stage_all(&self) -> rung_git::Result<()> {
            unimplemented!()
        }
        fn has_staged_changes(&self) -> rung_git::Result<bool> {
            Ok(!self.hunks.is_empty())
        }
        fn create_commit(&self, _message: &str) -> rung_git::Result<Oid> {
            unimplemented!()
        }
        fn rebase_onto(&self, _target: Oid) -> rung_git::Result<()> {
            unimplemented!()
        }
        fn rebase_onto_from(&self, _onto: Oid, _from: Oid) -> rung_git::Result<()> {
            unimplemented!()
        }
        fn conflicting_files(&self) -> rung_git::Result<Vec<String>> {
            unimplemented!()
        }
        fn rebase_abort(&self) -> rung_git::Result<()> {
            unimplemented!()
        }
        fn rebase_continue(&self) -> rung_git::Result<()> {
            unimplemented!()
        }
        fn origin_url(&self) -> rung_git::Result<String> {
            unimplemented!()
        }
        fn remote_divergence(&self, _branch: &str) -> rung_git::Result<RemoteDivergence> {
            unimplemented!()
        }
        fn detect_default_branch(&self) -> Option<String> {
            Some("main".to_string())
        }
        fn push(&self, _branch: &str, _force: bool) -> rung_git::Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> rung_git::Result<()> {
            unimplemented!()
        }
        fn fetch(&self, _branch: &str) -> rung_git::Result<()> {
            unimplemented!()
        }
        fn pull_ff(&self) -> rung_git::Result<()> {
            unimplemented!()
        }
        fn reset_branch(&self, _branch: &str, _commit: Oid) -> rung_git::Result<()> {
            unimplemented!()
        }
    }

    impl AbsorbOps for MockRepo {
        fn staged_diff_hunks(&self) -> rung_git::Result<Vec<Hunk>> {
            Ok(self.hunks.clone())
        }

        fn blame_lines(
            &self,
            file_path: &str,
            _start: u32,
            _end: u32,
        ) -> rung_git::Result<Vec<BlameResult>> {
            if let Some(err) = self.blame_errors.get(file_path) {
                return Err(rung_git::Error::BlameError(err.clone()));
            }
            Ok(self
                .blame_results
                .get(file_path)
                .cloned()
                .unwrap_or_default())
        }

        fn is_ancestor(&self, ancestor: Oid, descendant: Oid) -> rung_git::Result<bool> {
            Ok(self
                .is_ancestor_results
                .get(&(ancestor, descendant))
                .copied()
                .unwrap_or(false))
        }

        fn create_fixup_commit(&self, target: Oid) -> rung_git::Result<Oid> {
            self.fixup_commits_created.borrow_mut().push(target);
            // Return a new "fixup" commit OID
            Ok(Oid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap())
        }
    }

    // Mock implementation for StateStore
    #[derive(Default)]
    struct MockState {
        stack: Stack,
    }

    impl StateStore for MockState {
        fn is_initialized(&self) -> bool {
            true
        }
        fn init(&self) -> crate::Result<()> {
            Ok(())
        }
        fn rung_dir(&self) -> &Path {
            Path::new(".git/rung")
        }
        fn load_stack(&self) -> crate::Result<Stack> {
            Ok(self.stack.clone())
        }
        fn save_stack(&self, _stack: &Stack) -> crate::Result<()> {
            Ok(())
        }
        fn load_config(&self) -> crate::Result<Config> {
            Ok(Config::default())
        }
        fn save_config(&self, _config: &Config) -> crate::Result<()> {
            Ok(())
        }
        fn default_branch(&self) -> crate::Result<String> {
            Ok("main".to_string())
        }
        fn is_sync_in_progress(&self) -> bool {
            false
        }
        fn load_sync_state(&self) -> crate::Result<SyncState> {
            unimplemented!()
        }
        fn save_sync_state(&self, _state: &SyncState) -> crate::Result<()> {
            unimplemented!()
        }
        fn clear_sync_state(&self) -> crate::Result<()> {
            unimplemented!()
        }
        fn is_restack_in_progress(&self) -> bool {
            false
        }
        fn load_restack_state(&self) -> crate::Result<RestackState> {
            unimplemented!()
        }
        fn save_restack_state(&self, _state: &RestackState) -> crate::Result<()> {
            unimplemented!()
        }
        fn clear_restack_state(&self) -> crate::Result<()> {
            unimplemented!()
        }
        fn is_split_in_progress(&self) -> bool {
            false
        }
        fn load_split_state(&self) -> crate::Result<crate::state::SplitState> {
            Err(crate::Error::NoBackupFound)
        }
        fn save_split_state(&self, _state: &crate::state::SplitState) -> crate::Result<()> {
            Ok(())
        }
        fn clear_split_state(&self) -> crate::Result<()> {
            Ok(())
        }
        fn create_backup(&self, _branches: &[(&str, &str)]) -> crate::Result<String> {
            unimplemented!()
        }
        fn latest_backup(&self) -> crate::Result<String> {
            unimplemented!()
        }
        fn load_backup(&self, _backup_id: &str) -> crate::Result<Vec<(String, String)>> {
            unimplemented!()
        }
        fn delete_backup(&self, _backup_id: &str) -> crate::Result<()> {
            unimplemented!()
        }
        fn cleanup_backups(&self, _keep: usize) -> crate::Result<()> {
            unimplemented!()
        }
    }

    fn test_oid(n: u8) -> Oid {
        let hex = format!("{n:0>40}");
        Oid::from_str(&hex).unwrap()
    }

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

    #[test]
    fn test_create_plan_no_staged_changes() {
        let repo = MockRepo::default();
        let state = MockState::default();

        let plan = create_absorb_plan(&repo, &state, "main").unwrap();

        assert!(plan.actions.is_empty());
        assert!(plan.unmapped.is_empty());
    }

    #[test]
    fn test_create_plan_new_file_unmapped() {
        let mut repo = MockRepo::default();
        repo.hunks = vec![Hunk {
            file_path: "new_file.rs".to_string(),
            old_start: 0,
            old_lines: 0,
            new_start: 1,
            new_lines: 10,
            content: String::new(),
            is_new_file: true,
        }];
        repo.branch_commits.insert("main".to_string(), test_oid(1));
        repo.branch_commits
            .insert("origin/main".to_string(), test_oid(1));
        repo.branch_commits
            .insert("feature".to_string(), test_oid(2));

        let state = MockState::default();

        let plan = create_absorb_plan(&repo, &state, "main").unwrap();

        assert!(plan.actions.is_empty());
        assert_eq!(plan.unmapped.len(), 1);
        assert_eq!(plan.unmapped[0].reason, UnmapReason::NewFile);
    }

    #[test]
    fn test_create_plan_successful_mapping() {
        let target_commit = test_oid(3);

        let mut repo = MockRepo::default();
        repo.hunks = vec![Hunk {
            file_path: "src/lib.rs".to_string(),
            old_start: 10,
            old_lines: 5,
            new_start: 10,
            new_lines: 7,
            content: String::new(),
            is_new_file: false,
        }];
        repo.branch_commits.insert("main".to_string(), test_oid(1));
        repo.branch_commits
            .insert("origin/main".to_string(), test_oid(1));
        repo.branch_commits
            .insert("feature".to_string(), test_oid(2));
        repo.commits_between = vec![target_commit];
        repo.blame_results.insert(
            "src/lib.rs".to_string(),
            vec![BlameResult {
                commit: target_commit,
                message: "Add feature".to_string(),
            }],
        );

        let state = MockState::default();

        let plan = create_absorb_plan(&repo, &state, "main").unwrap();

        assert_eq!(plan.actions.len(), 1);
        assert!(plan.unmapped.is_empty());
        assert_eq!(plan.actions[0].target_commit, target_commit);
        assert_eq!(plan.actions[0].target_message, "Add feature");
    }

    #[test]
    fn test_create_plan_multiple_commits_unmapped() {
        let commit1 = test_oid(3);
        let commit2 = test_oid(4);

        let mut repo = MockRepo::default();
        repo.hunks = vec![Hunk {
            file_path: "src/lib.rs".to_string(),
            old_start: 10,
            old_lines: 5,
            new_start: 10,
            new_lines: 7,
            content: String::new(),
            is_new_file: false,
        }];
        repo.branch_commits.insert("main".to_string(), test_oid(1));
        repo.branch_commits
            .insert("origin/main".to_string(), test_oid(1));
        repo.branch_commits
            .insert("feature".to_string(), test_oid(2));
        repo.commits_between = vec![commit1, commit2];
        repo.blame_results.insert(
            "src/lib.rs".to_string(),
            vec![
                BlameResult {
                    commit: commit1,
                    message: "First commit".to_string(),
                },
                BlameResult {
                    commit: commit2,
                    message: "Second commit".to_string(),
                },
            ],
        );

        let state = MockState::default();

        let plan = create_absorb_plan(&repo, &state, "main").unwrap();

        assert!(plan.actions.is_empty());
        assert_eq!(plan.unmapped.len(), 1);
        assert_eq!(plan.unmapped[0].reason, UnmapReason::MultipleCommits);
    }

    #[test]
    fn test_create_plan_commit_not_in_stack() {
        let target_commit = test_oid(99); // Not in commits_between

        let mut repo = MockRepo::default();
        repo.hunks = vec![Hunk {
            file_path: "src/lib.rs".to_string(),
            old_start: 10,
            old_lines: 5,
            new_start: 10,
            new_lines: 7,
            content: String::new(),
            is_new_file: false,
        }];
        repo.branch_commits.insert("main".to_string(), test_oid(1));
        repo.branch_commits
            .insert("origin/main".to_string(), test_oid(1));
        repo.branch_commits
            .insert("feature".to_string(), test_oid(2));
        repo.commits_between = vec![test_oid(3)]; // Different commit
        repo.blame_results.insert(
            "src/lib.rs".to_string(),
            vec![BlameResult {
                commit: target_commit,
                message: "Old commit".to_string(),
            }],
        );

        let state = MockState::default();

        let plan = create_absorb_plan(&repo, &state, "main").unwrap();

        assert!(plan.actions.is_empty());
        assert_eq!(plan.unmapped.len(), 1);
        assert_eq!(plan.unmapped[0].reason, UnmapReason::CommitNotInStack);
    }

    #[test]
    fn test_create_plan_commit_on_base_branch() {
        let base_commit = test_oid(1);
        let target_commit = test_oid(99);

        let mut repo = MockRepo::default();
        repo.hunks = vec![Hunk {
            file_path: "src/lib.rs".to_string(),
            old_start: 10,
            old_lines: 5,
            new_start: 10,
            new_lines: 7,
            content: String::new(),
            is_new_file: false,
        }];
        repo.branch_commits.insert("main".to_string(), base_commit);
        repo.branch_commits
            .insert("origin/main".to_string(), base_commit);
        repo.branch_commits
            .insert("feature".to_string(), test_oid(2));
        repo.commits_between = vec![test_oid(3)];
        repo.blame_results.insert(
            "src/lib.rs".to_string(),
            vec![BlameResult {
                commit: target_commit,
                message: "Base commit".to_string(),
            }],
        );
        // Mark target as ancestor of base (on base branch)
        repo.is_ancestor_results
            .insert((target_commit, base_commit), true);

        let state = MockState::default();

        let plan = create_absorb_plan(&repo, &state, "main").unwrap();

        assert!(plan.actions.is_empty());
        assert_eq!(plan.unmapped.len(), 1);
        assert_eq!(plan.unmapped[0].reason, UnmapReason::CommitOnBaseBranch);
    }

    #[test]
    fn test_create_plan_blame_error() {
        let mut repo = MockRepo::default();
        repo.hunks = vec![Hunk {
            file_path: "src/lib.rs".to_string(),
            old_start: 10,
            old_lines: 5,
            new_start: 10,
            new_lines: 7,
            content: String::new(),
            is_new_file: false,
        }];
        repo.branch_commits.insert("main".to_string(), test_oid(1));
        repo.branch_commits
            .insert("origin/main".to_string(), test_oid(1));
        repo.branch_commits
            .insert("feature".to_string(), test_oid(2));
        repo.blame_errors
            .insert("src/lib.rs".to_string(), "file not found".to_string());

        let state = MockState::default();

        let plan = create_absorb_plan(&repo, &state, "main").unwrap();

        assert!(plan.actions.is_empty());
        assert_eq!(plan.unmapped.len(), 1);
        match &plan.unmapped[0].reason {
            UnmapReason::BlameError(msg) => assert!(msg.contains("file not found")),
            _ => panic!("Expected BlameError"),
        }
    }

    #[test]
    fn test_create_plan_empty_blame_result() {
        let mut repo = MockRepo::default();
        repo.hunks = vec![Hunk {
            file_path: "src/lib.rs".to_string(),
            old_start: 10,
            old_lines: 5,
            new_start: 10,
            new_lines: 7,
            content: String::new(),
            is_new_file: false,
        }];
        repo.branch_commits.insert("main".to_string(), test_oid(1));
        repo.branch_commits
            .insert("origin/main".to_string(), test_oid(1));
        repo.branch_commits
            .insert("feature".to_string(), test_oid(2));
        repo.blame_results.insert("src/lib.rs".to_string(), vec![]); // Empty blame results

        let state = MockState::default();

        let plan = create_absorb_plan(&repo, &state, "main").unwrap();

        assert!(plan.actions.is_empty());
        assert_eq!(plan.unmapped.len(), 1);
        match &plan.unmapped[0].reason {
            UnmapReason::BlameError(msg) => assert!(msg.contains("no blame results")),
            _ => panic!("Expected BlameError with 'no blame results'"),
        }
    }

    #[test]
    fn test_create_plan_insert_only_hunk() {
        // Insert-only hunks have old_lines = 0
        // They should still work by blaming the adjacent line
        let target_commit = test_oid(3);

        let mut repo = MockRepo::default();
        repo.hunks = vec![Hunk {
            file_path: "src/lib.rs".to_string(),
            old_start: 10, // Line where insertion happens
            old_lines: 0,  // Insert-only: no lines deleted
            new_start: 10,
            new_lines: 5, // 5 new lines inserted
            content: String::new(),
            is_new_file: false,
        }];
        repo.branch_commits.insert("main".to_string(), test_oid(1));
        repo.branch_commits
            .insert("origin/main".to_string(), test_oid(1));
        repo.branch_commits
            .insert("feature".to_string(), test_oid(2));
        repo.commits_between = vec![target_commit];
        repo.blame_results.insert(
            "src/lib.rs".to_string(),
            vec![BlameResult {
                commit: target_commit,
                message: "Target commit".to_string(),
            }],
        );

        let state = MockState::default();

        let plan = create_absorb_plan(&repo, &state, "main").unwrap();

        // Insert-only hunks should be mappable if adjacent line points to valid target
        assert_eq!(plan.actions.len(), 1);
        assert!(plan.unmapped.is_empty());
        assert_eq!(plan.actions[0].target_commit, target_commit);
    }

    #[test]
    fn test_execute_absorb_empty_plan() {
        let repo = MockRepo::default();
        let plan = AbsorbPlan {
            actions: vec![],
            unmapped: vec![],
        };

        let result = execute_absorb(&repo, &plan).unwrap();

        assert_eq!(result.fixups_created, 0);
        assert!(result.targeted_commits.is_empty());
    }

    #[test]
    fn test_execute_absorb_single_target() {
        let target_commit = test_oid(3);
        let repo = MockRepo::default();

        let plan = AbsorbPlan {
            actions: vec![AbsorbAction {
                hunk: Hunk {
                    file_path: "src/lib.rs".to_string(),
                    old_start: 10,
                    old_lines: 5,
                    new_start: 10,
                    new_lines: 7,
                    content: String::new(),
                    is_new_file: false,
                },
                target_commit,
                target_message: "Feature commit".to_string(),
            }],
            unmapped: vec![],
        };

        let result = execute_absorb(&repo, &plan).unwrap();

        assert_eq!(result.fixups_created, 1);
        assert_eq!(result.targeted_commits.len(), 1);
        assert_eq!(result.targeted_commits[0], target_commit);

        // Verify fixup commit was created
        let created = repo.fixup_commits_created.borrow();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0], target_commit);
    }

    #[test]
    fn test_execute_absorb_multiple_hunks_same_target() {
        let target_commit = test_oid(3);
        let repo = MockRepo::default();

        let plan = AbsorbPlan {
            actions: vec![
                AbsorbAction {
                    hunk: Hunk {
                        file_path: "src/lib.rs".to_string(),
                        old_start: 10,
                        old_lines: 5,
                        new_start: 10,
                        new_lines: 7,
                        content: String::new(),
                        is_new_file: false,
                    },
                    target_commit,
                    target_message: "Feature commit".to_string(),
                },
                AbsorbAction {
                    hunk: Hunk {
                        file_path: "src/main.rs".to_string(),
                        old_start: 20,
                        old_lines: 3,
                        new_start: 20,
                        new_lines: 5,
                        content: String::new(),
                        is_new_file: false,
                    },
                    target_commit,
                    target_message: "Feature commit".to_string(),
                },
            ],
            unmapped: vec![],
        };

        let result = execute_absorb(&repo, &plan).unwrap();

        // Multiple hunks targeting the same commit should create only one fixup
        assert_eq!(result.fixups_created, 1);
        assert_eq!(result.targeted_commits.len(), 1);
    }

    #[test]
    fn test_execute_absorb_multiple_targets_error() {
        let target1 = test_oid(3);
        let target2 = test_oid(4);
        let repo = MockRepo::default();

        let plan = AbsorbPlan {
            actions: vec![
                AbsorbAction {
                    hunk: Hunk {
                        file_path: "src/lib.rs".to_string(),
                        old_start: 10,
                        old_lines: 5,
                        new_start: 10,
                        new_lines: 7,
                        content: String::new(),
                        is_new_file: false,
                    },
                    target_commit: target1,
                    target_message: "First commit".to_string(),
                },
                AbsorbAction {
                    hunk: Hunk {
                        file_path: "src/main.rs".to_string(),
                        old_start: 20,
                        old_lines: 3,
                        new_start: 20,
                        new_lines: 5,
                        content: String::new(),
                        is_new_file: false,
                    },
                    target_commit: target2,
                    target_message: "Second commit".to_string(),
                },
            ],
            unmapped: vec![],
        };

        let result = execute_absorb(&repo, &plan);

        // Should error when hunks target different commits
        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(err_msg.contains("2 different commits"));
        assert!(err_msg.contains("selective hunk staging not supported"));
    }
}
