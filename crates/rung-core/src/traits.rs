//! Trait abstractions for state storage operations.
//!
//! This module defines the `StateStore` trait which abstracts state persistence,
//! enabling dependency injection and testability.

use std::path::Path;

use crate::Result;
use crate::config::Config;
use crate::stack::Stack;
use crate::state::{RestackState, SyncState};

/// Trait for state storage operations.
///
/// This trait abstracts state persistence, allowing for:
/// - Dependency injection in commands/services
/// - Mock implementations for testing
/// - Alternative implementations (e.g., in-memory state)
#[allow(clippy::missing_errors_doc)]
pub trait StateStore {
    // === Initialization ===

    /// Check if rung is initialized in this repository.
    fn is_initialized(&self) -> bool;

    /// Initialize the .git/rung/ directory structure.
    fn init(&self) -> Result<()>;

    /// Get the path to the rung directory.
    fn rung_dir(&self) -> &Path;

    // === Stack Operations ===

    /// Load the stack from disk.
    fn load_stack(&self) -> Result<Stack>;

    /// Save the stack to disk.
    fn save_stack(&self, stack: &Stack) -> Result<()>;

    // === Config Operations ===

    /// Load the config from disk.
    fn load_config(&self) -> Result<Config>;

    /// Save the config to disk.
    fn save_config(&self, config: &Config) -> Result<()>;

    /// Get the default branch name from config, falling back to "main".
    ///
    /// # Errors
    /// Returns error if the config file cannot be read or parsed.
    fn default_branch(&self) -> Result<String>;

    // === Sync State Operations ===

    /// Check if a sync is in progress.
    fn is_sync_in_progress(&self) -> bool;

    /// Load the current sync state.
    fn load_sync_state(&self) -> Result<SyncState>;

    /// Save sync state (called during sync operation).
    fn save_sync_state(&self, state: &SyncState) -> Result<()>;

    /// Clear sync state (called when sync completes or aborts).
    fn clear_sync_state(&self) -> Result<()>;

    // === Restack State Operations ===

    /// Check if a restack is in progress.
    fn is_restack_in_progress(&self) -> bool;

    /// Load the current restack state.
    fn load_restack_state(&self) -> Result<RestackState>;

    /// Save restack state (called during restack operation).
    fn save_restack_state(&self, state: &RestackState) -> Result<()>;

    /// Clear restack state (called when restack completes or aborts).
    fn clear_restack_state(&self) -> Result<()>;

    // === Backup Operations ===

    /// Create a backup of branch refs.
    ///
    /// Returns the backup ID (timestamp).
    fn create_backup(&self, branches: &[(&str, &str)]) -> Result<String>;

    /// Get the most recent backup ID.
    fn latest_backup(&self) -> Result<String>;

    /// Load a backup's branch refs.
    ///
    /// Returns a vec of (`branch_name`, `commit_sha`) pairs.
    fn load_backup(&self, backup_id: &str) -> Result<Vec<(String, String)>>;

    /// Delete a backup.
    fn delete_backup(&self, backup_id: &str) -> Result<()>;

    /// Clean up old backups, keeping only the most recent N.
    fn cleanup_backups(&self, keep: usize) -> Result<()>;
}
