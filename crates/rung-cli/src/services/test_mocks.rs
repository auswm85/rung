//! Mock implementations for testing services.
//!
//! These mocks implement the traits from rung-git and rung-core
//! to enable unit testing of service logic without real git repos.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rung_core::config::Config;
use rung_core::stack::Stack;
use rung_core::state::{RestackState, SyncState};
use rung_core::{Result as CoreResult, StateStore};
use rung_git::{GitOps, Oid, RemoteDivergence, Result as GitResult};

/// Mock implementation of `GitOps` for testing.
pub struct MockGitOps {
    pub current_branch: RefCell<String>,
    pub branches: RefCell<HashMap<String, Oid>>,
    pub branch_exists_map: RefCell<HashMap<String, bool>>,
    pub remote_divergence_map: RefCell<HashMap<String, RemoteDivergence>>,
    pub is_clean: RefCell<bool>,
    pub is_rebasing: RefCell<bool>,
    pub push_results: RefCell<HashMap<String, bool>>,
    pub has_staged_changes: RefCell<bool>,
    pub rebase_should_fail: RefCell<bool>,
}

impl Default for MockGitOps {
    fn default() -> Self {
        Self::new()
    }
}

impl MockGitOps {
    pub fn new() -> Self {
        Self {
            current_branch: RefCell::new("main".to_string()),
            branches: RefCell::new(HashMap::new()),
            branch_exists_map: RefCell::new(HashMap::new()),
            remote_divergence_map: RefCell::new(HashMap::new()),
            is_clean: RefCell::new(true),
            is_rebasing: RefCell::new(false),
            push_results: RefCell::new(HashMap::new()),
            has_staged_changes: RefCell::new(false),
            rebase_should_fail: RefCell::new(false),
        }
    }

    #[allow(dead_code)]
    pub fn with_staged_changes(self, has_staged: bool) -> Self {
        *self.has_staged_changes.borrow_mut() = has_staged;
        self
    }

    #[allow(dead_code)]
    pub fn with_clean(self, clean: bool) -> Self {
        *self.is_clean.borrow_mut() = clean;
        self
    }

    pub fn with_branch(self, name: &str, oid: Oid) -> Self {
        self.branches.borrow_mut().insert(name.to_string(), oid);
        self.branch_exists_map
            .borrow_mut()
            .insert(name.to_string(), true);
        self
    }

    #[allow(dead_code)]
    pub fn with_current_branch(self, name: &str) -> Self {
        *self.current_branch.borrow_mut() = name.to_string();
        self
    }

    pub fn with_push_result(self, branch: &str, success: bool) -> Self {
        self.push_results
            .borrow_mut()
            .insert(branch.to_string(), success);
        self
    }

    #[allow(dead_code)]
    pub fn with_rebase_failure(self) -> Self {
        *self.rebase_should_fail.borrow_mut() = true;
        self
    }
}

impl GitOps for MockGitOps {
    fn workdir(&self) -> Option<&Path> {
        None
    }

    fn current_branch(&self) -> GitResult<String> {
        Ok(self.current_branch.borrow().clone())
    }

    fn head_detached(&self) -> GitResult<bool> {
        Ok(false)
    }

    fn is_rebasing(&self) -> bool {
        *self.is_rebasing.borrow()
    }

    fn branch_exists(&self, name: &str) -> bool {
        self.branch_exists_map
            .borrow()
            .get(name)
            .copied()
            .unwrap_or(false)
    }

    fn create_branch(&self, name: &str) -> GitResult<Oid> {
        let oid = Oid::zero();
        self.branches.borrow_mut().insert(name.to_string(), oid);
        self.branch_exists_map
            .borrow_mut()
            .insert(name.to_string(), true);
        Ok(oid)
    }

    fn checkout(&self, branch: &str) -> GitResult<()> {
        *self.current_branch.borrow_mut() = branch.to_string();
        Ok(())
    }

    fn delete_branch(&self, name: &str) -> GitResult<()> {
        self.branches.borrow_mut().remove(name);
        self.branch_exists_map.borrow_mut().remove(name);
        Ok(())
    }

    fn list_branches(&self) -> GitResult<Vec<String>> {
        let mut branches: Vec<String> = self.branches.borrow().keys().cloned().collect();
        branches.sort();
        Ok(branches)
    }

    fn branch_commit(&self, branch: &str) -> GitResult<Oid> {
        self.branches
            .borrow()
            .get(branch)
            .copied()
            .ok_or_else(|| rung_git::Error::BranchNotFound(branch.to_string()))
    }

    fn remote_branch_commit(&self, branch: &str) -> GitResult<Oid> {
        self.branch_commit(branch)
    }

    fn branch_commit_message(&self, _branch: &str) -> GitResult<String> {
        Ok("Test commit message".to_string())
    }

    fn merge_base(&self, one: Oid, _two: Oid) -> GitResult<Oid> {
        Ok(one)
    }

    fn commits_between(&self, _from: Oid, _to: Oid) -> GitResult<Vec<Oid>> {
        Ok(vec![])
    }

    fn count_commits_between(&self, _from: Oid, _to: Oid) -> GitResult<usize> {
        Ok(0)
    }

    fn is_clean(&self) -> GitResult<bool> {
        Ok(*self.is_clean.borrow())
    }

    fn require_clean(&self) -> GitResult<()> {
        if *self.is_clean.borrow() {
            Ok(())
        } else {
            Err(rung_git::Error::DirtyWorkingDirectory)
        }
    }

    fn stage_all(&self) -> GitResult<()> {
        Ok(())
    }

    fn has_staged_changes(&self) -> GitResult<bool> {
        Ok(*self.has_staged_changes.borrow())
    }

    fn create_commit(&self, _message: &str) -> GitResult<Oid> {
        Ok(Oid::zero())
    }

    fn rebase_onto(&self, _target: Oid) -> GitResult<()> {
        if *self.rebase_should_fail.borrow() {
            *self.is_rebasing.borrow_mut() = true;
            return Err(rung_git::Error::RebaseConflict(vec![
                "conflict.rs".to_string(),
            ]));
        }
        Ok(())
    }

    fn rebase_onto_from(&self, _onto: Oid, _from: Oid) -> GitResult<()> {
        if *self.rebase_should_fail.borrow() {
            *self.is_rebasing.borrow_mut() = true;
            return Err(rung_git::Error::RebaseConflict(vec![
                "conflict.rs".to_string(),
            ]));
        }
        Ok(())
    }

    fn conflicting_files(&self) -> GitResult<Vec<String>> {
        if *self.rebase_should_fail.borrow() {
            Ok(vec!["conflict.rs".to_string()])
        } else {
            Ok(vec![])
        }
    }

    fn rebase_abort(&self) -> GitResult<()> {
        *self.is_rebasing.borrow_mut() = false;
        Ok(())
    }

    fn rebase_continue(&self) -> GitResult<()> {
        *self.is_rebasing.borrow_mut() = false;
        Ok(())
    }

    fn origin_url(&self) -> GitResult<String> {
        Ok("https://github.com/test/repo.git".to_string())
    }

    fn remote_divergence(&self, branch: &str) -> GitResult<RemoteDivergence> {
        Ok(self
            .remote_divergence_map
            .borrow()
            .get(branch)
            .cloned()
            .unwrap_or(RemoteDivergence::InSync))
    }

    fn detect_default_branch(&self) -> Option<String> {
        Some("main".to_string())
    }

    fn push(&self, branch: &str, _force: bool) -> GitResult<()> {
        if self
            .push_results
            .borrow()
            .get(branch)
            .copied()
            .unwrap_or(true)
        {
            Ok(())
        } else {
            Err(rung_git::Error::PushFailed("mock push failed".to_string()))
        }
    }

    fn fetch_all(&self) -> GitResult<()> {
        Ok(())
    }

    fn fetch(&self, _branch: &str) -> GitResult<()> {
        Ok(())
    }

    fn pull_ff(&self) -> GitResult<()> {
        Ok(())
    }

    fn reset_branch(&self, branch: &str, commit: Oid) -> GitResult<()> {
        self.branches
            .borrow_mut()
            .insert(branch.to_string(), commit);
        self.branch_exists_map
            .borrow_mut()
            .insert(branch.to_string(), true);
        Ok(())
    }
}

/// Mock implementation of `StateStore` for testing.
pub struct MockStateStore {
    pub stack: RefCell<Stack>,
    pub config: RefCell<Config>,
    pub initialized: bool,
    pub default_branch: String,
    pub rung_dir: PathBuf,
    pub sync_in_progress: RefCell<bool>,
    pub sync_state: RefCell<Option<SyncState>>,
    pub restack_in_progress: RefCell<bool>,
    pub restack_state: RefCell<Option<RestackState>>,
}

impl Default for MockStateStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MockStateStore {
    pub fn new() -> Self {
        Self {
            stack: RefCell::new(Stack::default()),
            config: RefCell::new(Config::default()),
            initialized: true,
            default_branch: "main".to_string(),
            rung_dir: std::env::temp_dir().join("mock-rung"),
            sync_in_progress: RefCell::new(false),
            sync_state: RefCell::new(None),
            restack_in_progress: RefCell::new(false),
            restack_state: RefCell::new(None),
        }
    }

    pub fn with_stack(self, stack: Stack) -> Self {
        *self.stack.borrow_mut() = stack;
        self
    }

    #[allow(dead_code)]
    pub fn with_restack_state(self, state: RestackState) -> Self {
        *self.restack_state.borrow_mut() = Some(state);
        *self.restack_in_progress.borrow_mut() = true;
        self
    }
}

impl StateStore for MockStateStore {
    fn is_initialized(&self) -> bool {
        self.initialized
    }

    fn init(&self) -> CoreResult<()> {
        Ok(())
    }

    fn rung_dir(&self) -> &Path {
        &self.rung_dir
    }

    fn load_stack(&self) -> CoreResult<Stack> {
        Ok(self.stack.borrow().clone())
    }

    fn save_stack(&self, stack: &Stack) -> CoreResult<()> {
        *self.stack.borrow_mut() = stack.clone();
        Ok(())
    }

    fn load_config(&self) -> CoreResult<Config> {
        Ok(self.config.borrow().clone())
    }

    fn save_config(&self, config: &Config) -> CoreResult<()> {
        *self.config.borrow_mut() = config.clone();
        Ok(())
    }

    fn default_branch(&self) -> CoreResult<String> {
        Ok(self.default_branch.clone())
    }

    fn is_sync_in_progress(&self) -> bool {
        *self.sync_in_progress.borrow()
    }

    fn load_sync_state(&self) -> CoreResult<SyncState> {
        // Return custom state if set, otherwise return a default
        if let Some(state) = self.sync_state.borrow().as_ref() {
            return Ok(state.clone());
        }
        Ok(SyncState::new("test-backup".to_string(), vec![]))
    }

    fn save_sync_state(&self, state: &SyncState) -> CoreResult<()> {
        *self.sync_state.borrow_mut() = Some(state.clone());
        *self.sync_in_progress.borrow_mut() = true;
        Ok(())
    }

    fn clear_sync_state(&self) -> CoreResult<()> {
        *self.sync_state.borrow_mut() = None;
        *self.sync_in_progress.borrow_mut() = false;
        Ok(())
    }

    fn is_restack_in_progress(&self) -> bool {
        *self.restack_in_progress.borrow()
    }

    fn load_restack_state(&self) -> CoreResult<RestackState> {
        // Return custom state if set, otherwise return a default
        if let Some(state) = self.restack_state.borrow().as_ref() {
            return Ok(state.clone());
        }
        Ok(RestackState::new(
            "test-backup".to_string(),
            "feature".to_string(),
            "main".to_string(),
            Some("develop".to_string()),
            "main".to_string(),
            vec![],
            vec![],
        ))
    }

    fn save_restack_state(&self, state: &RestackState) -> CoreResult<()> {
        *self.restack_state.borrow_mut() = Some(state.clone());
        *self.restack_in_progress.borrow_mut() = true;
        Ok(())
    }

    fn clear_restack_state(&self) -> CoreResult<()> {
        *self.restack_state.borrow_mut() = None;
        *self.restack_in_progress.borrow_mut() = false;
        Ok(())
    }

    fn create_backup(&self, _refs: &[(&str, &str)]) -> CoreResult<String> {
        Ok("mock-backup-id".to_string())
    }

    fn latest_backup(&self) -> CoreResult<String> {
        Ok("mock-backup-id".to_string())
    }

    fn load_backup(&self, _id: &str) -> CoreResult<Vec<(String, String)>> {
        Ok(vec![])
    }

    fn delete_backup(&self, _id: &str) -> CoreResult<()> {
        Ok(())
    }

    fn cleanup_backups(&self, _keep: usize) -> CoreResult<()> {
        Ok(())
    }
}
