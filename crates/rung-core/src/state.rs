//! State persistence for .git/rung/ directory.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::stack::Stack;

/// Manages the .git/rung/ directory state.
#[derive(Debug)]
pub struct State {
    /// Path to the .git/rung/ directory.
    rung_dir: PathBuf,
}

impl State {
    /// File names within .git/rung/
    const STACK_FILE: &'static str = "stack.json";
    const CONFIG_FILE: &'static str = "config.toml";
    const SYNC_STATE_FILE: &'static str = "sync_state";
    const REFS_DIR: &'static str = "refs";

    /// Create a new State instance for the given repository.
    ///
    /// # Errors
    /// Returns error if the path doesn't contain a .git directory.
    pub fn new(repo_path: impl AsRef<Path>) -> Result<Self> {
        let git_dir = repo_path.as_ref().join(".git");
        if !git_dir.exists() {
            return Err(Error::NotARepository);
        }

        Ok(Self {
            rung_dir: git_dir.join("rung"),
        })
    }

    /// Initialize the .git/rung/ directory structure.
    ///
    /// # Errors
    /// Returns error if directory creation fails.
    pub fn init(&self) -> Result<()> {
        fs::create_dir_all(&self.rung_dir)?;
        fs::create_dir_all(self.rung_dir.join(Self::REFS_DIR))?;

        // Create empty stack if it doesn't exist
        if !self.stack_path().exists() {
            self.save_stack(&Stack::new())?;
        }

        Ok(())
    }

    /// Check if rung is initialized in this repository.
    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.rung_dir.exists() && self.stack_path().exists()
    }

    /// Get the path to the rung directory.
    #[must_use]
    pub fn rung_dir(&self) -> &Path {
        &self.rung_dir
    }

    // === Stack operations ===

    fn stack_path(&self) -> PathBuf {
        self.rung_dir.join(Self::STACK_FILE)
    }

    /// Load the stack from disk.
    ///
    /// # Errors
    /// Returns error if file doesn't exist or can't be parsed.
    pub fn load_stack(&self) -> Result<Stack> {
        if !self.is_initialized() {
            return Err(Error::NotInitialized);
        }

        let content = fs::read_to_string(self.stack_path())?;
        let stack: Stack = serde_json::from_str(&content)?;
        Ok(stack)
    }

    /// Save the stack to disk.
    ///
    /// # Errors
    /// Returns error if serialization or write fails.
    pub fn save_stack(&self, stack: &Stack) -> Result<()> {
        let content = serde_json::to_string_pretty(stack)?;
        fs::write(self.stack_path(), content)?;
        Ok(())
    }

    // === Sync state operations ===

    fn sync_state_path(&self) -> PathBuf {
        self.rung_dir.join(Self::SYNC_STATE_FILE)
    }

    /// Check if a sync is in progress.
    #[must_use]
    pub fn is_sync_in_progress(&self) -> bool {
        self.sync_state_path().exists()
    }

    /// Load the current sync state.
    ///
    /// # Errors
    /// Returns error if no sync is in progress or file can't be read.
    pub fn load_sync_state(&self) -> Result<SyncState> {
        if !self.is_sync_in_progress() {
            return Err(Error::NoBackupFound);
        }

        let content = fs::read_to_string(self.sync_state_path())?;
        let state: SyncState = serde_json::from_str(&content)?;
        Ok(state)
    }

    /// Save sync state (called during sync operation).
    ///
    /// # Errors
    /// Returns error if serialization or write fails.
    pub fn save_sync_state(&self, state: &SyncState) -> Result<()> {
        let content = serde_json::to_string_pretty(state)?;
        fs::write(self.sync_state_path(), content)?;
        Ok(())
    }

    /// Clear sync state (called when sync completes or aborts).
    ///
    /// # Errors
    /// Returns error if file removal fails.
    pub fn clear_sync_state(&self) -> Result<()> {
        let path = self.sync_state_path();
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    // === Backup operations ===

    fn refs_dir(&self) -> PathBuf {
        self.rung_dir.join(Self::REFS_DIR)
    }

    /// Create a backup of branch refs.
    ///
    /// Returns the backup ID (timestamp).
    ///
    /// # Errors
    /// Returns error if directory creation or file write fails.
    pub fn create_backup(&self, branches: &[(&str, &str)]) -> Result<String> {
        let backup_id = Utc::now().timestamp().to_string();
        let backup_dir = self.refs_dir().join(&backup_id);
        fs::create_dir_all(&backup_dir)?;

        for (branch_name, commit_sha) in branches {
            let safe_name = branch_name.replace('/', "-");
            fs::write(backup_dir.join(safe_name), commit_sha)?;
        }

        Ok(backup_id)
    }

    /// Get the most recent backup ID.
    ///
    /// # Errors
    /// Returns error if no backups exist.
    pub fn latest_backup(&self) -> Result<String> {
        let refs_dir = self.refs_dir();
        if !refs_dir.exists() {
            return Err(Error::NoBackupFound);
        }

        let mut backups: Vec<_> = fs::read_dir(&refs_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().to_str().map(String::from))
            .filter_map(|name| name.parse::<i64>().ok().map(|ts| (ts, name)))
            .collect();

        backups.sort_by_key(|(ts, _)| std::cmp::Reverse(*ts));

        backups
            .into_iter()
            .next()
            .map(|(_, name)| name)
            .ok_or(Error::NoBackupFound)
    }

    /// Load a backup's branch refs.
    ///
    /// Returns a vec of (branch_name, commit_sha) pairs.
    ///
    /// # Errors
    /// Returns error if backup doesn't exist or can't be read.
    pub fn load_backup(&self, backup_id: &str) -> Result<Vec<(String, String)>> {
        let backup_dir = self.refs_dir().join(backup_id);
        if !backup_dir.exists() {
            return Err(Error::NoBackupFound);
        }

        let mut refs = vec![];
        for entry in fs::read_dir(&backup_dir)? {
            let entry = entry?;
            if entry.path().is_file() {
                let name = entry
                    .file_name()
                    .to_str()
                    .ok_or_else(|| Error::StateParseError {
                        file: entry.path(),
                        message: "invalid filename".into(),
                    })?
                    .replace('-', "/");
                let sha = fs::read_to_string(entry.path())?.trim().to_string();
                refs.push((name, sha));
            }
        }

        Ok(refs)
    }

    /// Delete a backup.
    ///
    /// # Errors
    /// Returns error if deletion fails.
    pub fn delete_backup(&self, backup_id: &str) -> Result<()> {
        let backup_dir = self.refs_dir().join(backup_id);
        if backup_dir.exists() {
            fs::remove_dir_all(backup_dir)?;
        }
        Ok(())
    }

    /// Clean up old backups, keeping only the most recent N.
    ///
    /// # Errors
    /// Returns error if cleanup fails.
    pub fn cleanup_backups(&self, keep: usize) -> Result<()> {
        let refs_dir = self.refs_dir();
        if !refs_dir.exists() {
            return Ok(());
        }

        let mut backups: Vec<_> = fs::read_dir(&refs_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| {
                e.file_name()
                    .to_str()
                    .and_then(|s| s.parse::<i64>().ok())
                    .map(|ts| (ts, e.path()))
            })
            .collect();

        backups.sort_by_key(|(ts, _)| std::cmp::Reverse(*ts));

        for (_, path) in backups.into_iter().skip(keep) {
            fs::remove_dir_all(path)?;
        }

        Ok(())
    }
}

/// State tracked during an in-progress sync operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncState {
    /// When the sync started.
    pub started_at: DateTime<Utc>,

    /// Backup ID for this sync.
    pub backup_id: String,

    /// Branch currently being rebased.
    pub current_branch: String,

    /// Branches that have been successfully rebased.
    pub completed: Vec<String>,

    /// Branches remaining to be rebased.
    pub remaining: Vec<String>,
}

impl SyncState {
    /// Create a new sync state.
    #[must_use]
    pub fn new(backup_id: String, branches: Vec<String>) -> Self {
        let current = branches.first().cloned().unwrap_or_default();
        let remaining = branches.into_iter().skip(1).collect();

        Self {
            started_at: Utc::now(),
            backup_id,
            current_branch: current,
            completed: vec![],
            remaining,
        }
    }

    /// Mark current branch as complete and move to next.
    pub fn advance(&mut self) {
        if !self.current_branch.is_empty() {
            self.completed.push(self.current_branch.clone());
        }
        self.current_branch = self.remaining.first().cloned().unwrap_or_default();
        if !self.remaining.is_empty() {
            self.remaining.remove(0);
        }
    }

    /// Check if sync is complete.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.current_branch.is_empty() && self.remaining.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_repo() -> (TempDir, State) {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        let state = State::new(temp.path()).unwrap();
        (temp, state)
    }

    #[test]
    fn test_init_and_check() {
        let (_temp, state) = setup_test_repo();

        assert!(!state.is_initialized());
        state.init().unwrap();
        assert!(state.is_initialized());
    }

    #[test]
    fn test_stack_persistence() {
        let (_temp, state) = setup_test_repo();
        state.init().unwrap();

        let mut stack = Stack::new();
        stack.add_branch(crate::stack::StackBranch::new(
            "feature/test",
            Some("main".into()),
        ));

        state.save_stack(&stack).unwrap();
        let loaded = state.load_stack().unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.branches[0].name, "feature/test");
    }

    #[test]
    fn test_backup_operations() {
        let (_temp, state) = setup_test_repo();
        state.init().unwrap();

        let branches = vec![("feature/a", "abc123"), ("feature/b", "def456")];
        let backup_id = state.create_backup(&branches).unwrap();

        let loaded = state.load_backup(&backup_id).unwrap();
        assert_eq!(loaded.len(), 2);

        let latest = state.latest_backup().unwrap();
        assert_eq!(latest, backup_id);

        state.delete_backup(&backup_id).unwrap();
        assert!(state.latest_backup().is_err());
    }
}
