//! Integration tests for the rung CLI.
//!
//! These tests verify the CLI commands work correctly end-to-end.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::process::Command as StdCommand;
use tempfile::TempDir;

/// Helper to create a git repository in a temp directory.
fn setup_git_repo() -> TempDir {
    let temp = TempDir::new().expect("Failed to create temp dir");

    StdCommand::new("git")
        .args(["init"])
        .current_dir(&temp)
        .output()
        .expect("Failed to init git repo");

    StdCommand::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(&temp)
        .output()
        .expect("Failed to set git email");

    StdCommand::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(&temp)
        .output()
        .expect("Failed to set git name");

    StdCommand::new("git")
        .args(["config", "core.editor", "true"])
        .current_dir(&temp)
        .output()
        .expect("Failed to set git editor");

    // Create initial commit so we have a valid HEAD
    let readme = temp.path().join("README.md");
    fs::write(&readme, "# Test Repo\n").expect("Failed to write README");

    StdCommand::new("git")
        .args(["add", "."])
        .current_dir(&temp)
        .output()
        .expect("Failed to git add");

    StdCommand::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&temp)
        .output()
        .expect("Failed to create initial commit");

    // Rename branch to main (in case default is master)
    StdCommand::new("git")
        .args(["branch", "-M", "main"])
        .current_dir(&temp)
        .output()
        .expect("Failed to rename branch to main");

    temp
}

/// Helper to create a git commit
fn git_commit(msg: &str, dir: &TempDir) {
    let file = dir.path().join("feature.txt");
    let mut current = fs::read_to_string(&file).unwrap_or_default();
    current.push_str("\nnew line");
    fs::write(&file, &current).expect("Failed to write file");

    StdCommand::new("git")
        .args(["add", "."])
        .current_dir(dir)
        .output()
        .expect("Failed to git add");

    StdCommand::new("git")
        .args(["commit", "-m", msg])
        .current_dir(dir)
        .output()
        .expect("Failed to commit");
}

/// Helper to get rung command.
fn rung() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rung"))
}

// ============================================================================
// Basic CLI tests
// ============================================================================

#[test]
fn test_version_flag() {
    rung()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("rung"));
}

#[test]
fn test_help_flag() {
    rung()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("stacked PRs"))
        .stdout(predicate::str::contains("init"))
        .stdout(predicate::str::contains("create"))
        .stdout(predicate::str::contains("status"))
        .stdout(predicate::str::contains("sync"))
        .stdout(predicate::str::contains("move"));
}

#[test]
fn test_no_subcommand_shows_help() {
    rung()
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage"));
}

// ============================================================================
// Init command tests
// ============================================================================

#[test]
fn test_init_success() {
    let temp = setup_git_repo();

    rung()
        .arg("init")
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized"));

    // Verify .git/rung directory was created
    assert!(temp.path().join(".git/rung").exists());
    assert!(temp.path().join(".git/rung/stack.json").exists());
}

#[test]
fn test_init_already_initialized() {
    let temp = setup_git_repo();

    // First init
    rung().arg("init").current_dir(&temp).assert().success();

    // Second init should warn (exits 0 but shows warning on stderr)
    rung()
        .arg("init")
        .current_dir(&temp)
        .assert()
        .success()
        .stderr(predicate::str::contains("already initialized"));
}

#[test]
fn test_init_not_in_git_repo() {
    let temp = TempDir::new().expect("Failed to create temp dir");

    rung()
        .arg("init")
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("git repository"));
}

// ============================================================================
// Status command tests
// ============================================================================

#[test]
fn test_status_not_initialized() {
    let temp = setup_git_repo();

    rung()
        .arg("status")
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not initialized"));
}

#[test]
fn test_status_empty_stack() {
    let temp = setup_git_repo();

    // Initialize rung
    rung().arg("init").current_dir(&temp).assert().success();

    // Status should indicate no branches yet
    rung()
        .arg("status")
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("No branches in stack"));
}

#[test]
fn test_status_json_output() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    let output = rung()
        .args(["status", "--json"])
        .current_dir(&temp)
        .assert()
        .success();

    // Verify it's valid JSON
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(
        serde_json::from_str::<serde_json::Value>(&stdout).is_ok(),
        "Status --json should produce valid JSON"
    );
}

// ============================================================================
// Create command tests
// ============================================================================

#[test]
fn test_create_branch() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a new branch in the stack
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("feature-1"));

    // Verify we're on the new branch
    let output = StdCommand::new("git")
        .args(["branch", "--show-current"])
        .current_dir(&temp)
        .output()
        .expect("Failed to get current branch");

    let current_branch = String::from_utf8_lossy(&output.stdout);
    assert!(
        current_branch.trim() == "feature-1",
        "Should be on feature-1 branch"
    );

    // Status should show the branch in the stack
    rung()
        .arg("status")
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("feature-1"));
}

#[test]
fn test_create_stacked_branches() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create first branch
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();

    // Make a commit on feature-1
    let file = temp.path().join("feature1.txt");
    fs::write(&file, "feature 1 content").expect("Failed to write file");

    StdCommand::new("git")
        .args(["add", "."])
        .current_dir(&temp)
        .output()
        .expect("Failed to git add");

    StdCommand::new("git")
        .args(["commit", "-m", "Add feature 1"])
        .current_dir(&temp)
        .output()
        .expect("Failed to commit");

    // Create second branch stacked on first
    rung()
        .args(["create", "feature-2"])
        .current_dir(&temp)
        .assert()
        .success();

    // Status should show both branches
    rung()
        .arg("status")
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("feature-1"))
        .stdout(predicate::str::contains("feature-2"));
}

#[test]
fn test_create_alias() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Use 'c' alias instead of 'create'
    rung()
        .args(["c", "feature-alias"])
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("feature-alias"));
}

// ============================================================================
// Navigation command tests
// ============================================================================

#[test]
fn test_navigate_up_down() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a branch
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();

    // Navigate to parent (main)
    rung().arg("prv").current_dir(&temp).assert().success();

    // Verify we're on main
    let output = StdCommand::new("git")
        .args(["branch", "--show-current"])
        .current_dir(&temp)
        .output()
        .expect("Failed to get current branch");

    let current_branch = String::from_utf8_lossy(&output.stdout);
    assert!(current_branch.trim() == "main", "Should be on main branch");

    // Navigate to child (feature-1)
    rung().arg("nxt").current_dir(&temp).assert().success();

    let output = StdCommand::new("git")
        .args(["branch", "--show-current"])
        .current_dir(&temp)
        .output()
        .expect("Failed to get current branch");

    let current_branch = String::from_utf8_lossy(&output.stdout);
    assert!(
        current_branch.trim() == "feature-1",
        "Should be on feature-1 branch"
    );
}

#[test]
fn test_navigate_no_parent() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Try to navigate to parent from main (exits 0 with info message)
    rung()
        .arg("prv")
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("no parent"));
}

#[test]
fn test_navigate_no_child() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Try to navigate to child from main with no children (exits 0 with info message)
    rung()
        .arg("nxt")
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("no children"));
}

// Note: Interactive move command tests are limited because inquire
// requires a TTY which is not available in the test environment.
// The command is tested via help output only.

#[test]
fn test_move_in_help() {
    // Verify move command is registered and shows in main help
    rung()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("move"))
        .stdout(predicate::str::contains("Interactive branch picker"));
}

// ============================================================================
// Doctor command tests
// ============================================================================

#[test]
fn test_doctor_healthy_repo() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    rung()
        .arg("doctor")
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("✓").or(predicate::str::contains("OK")));
}

#[test]
fn test_doctor_not_initialized() {
    let temp = setup_git_repo();

    // Doctor on uninitialized repo reports the issue (exits 0 with diagnostic info)
    rung()
        .arg("doctor")
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("not initialized"));
}

// ============================================================================
// Sync command tests
// ============================================================================

#[test]
fn test_sync_dry_run() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a branch
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();

    // Dry run should succeed without making changes
    // Note: --base main is required since there's no origin remote in tests
    rung()
        .args(["sync", "--dry-run", "--base", "main"])
        .current_dir(&temp)
        .assert()
        .success();
}

#[test]
fn test_sync_nothing_to_sync() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a branch
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();

    // Sync when already up to date
    // Note: --base main is required since there's no origin remote in tests
    rung()
        .args(["sync", "--base", "main"])
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("up-to-date"));
}

#[test]
fn test_sync_conflict_and_continue() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a base commit
    let file = temp.path().join("test.txt");
    fs::write(&file, "test").expect("Failed to write file");
    StdCommand::new("git")
        .args(["add", "."])
        .current_dir(&temp)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["commit", "-m", "Base commit"])
        .current_dir(&temp)
        .output()
        .unwrap();

    // Create a feature branch
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();
    fs::write(&file, "Feature change\n").expect("Failed to write file");
    StdCommand::new("git")
        .args(["add", "."])
        .current_dir(&temp)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["commit", "-m", "Feature commit"])
        .current_dir(&temp)
        .output()
        .unwrap();

    // Create conflict in main
    StdCommand::new("git")
        .args(["checkout", "main"])
        .current_dir(&temp)
        .output()
        .unwrap();
    fs::write(&file, "Main change\n").expect("Failed to write file");
    StdCommand::new("git")
        .args(["add", "."])
        .current_dir(&temp)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["commit", "-m", "Main commit"])
        .current_dir(&temp)
        .output()
        .unwrap();

    // Try to sync (should fail with conflic)
    rung()
        .args(["sync", "--base", "main"])
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("Conflict").or(predicate::str::contains("Paused")));

    // Resolve conflic manually
    fs::write(&file, "Resolved content\n").expect("Failed to write file");
    StdCommand::new("git")
        .args(["add", "."])
        .current_dir(&temp)
        .output()
        .unwrap();

    // Continue sync
    rung()
        .args(["sync", "--continue"])
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("Synced"));
}

#[test]
fn test_sync_abort_restores_branches() {
    let temp = setup_git_repo();
    rung().arg("init").current_dir(&temp).assert().success();

    // Setup conflict
    let file = temp.path().join("test.txt");
    fs::write(&file, "base").unwrap();
    StdCommand::new("git")
        .args(["add", "."])
        .current_dir(&temp)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["commit", "-m", "base"])
        .current_dir(&temp)
        .output()
        .unwrap();

    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();
    fs::write(&file, "feature").unwrap();
    StdCommand::new("git")
        .args(["add", "."])
        .current_dir(&temp)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["commit", "-m", "feature"])
        .current_dir(&temp)
        .output()
        .unwrap();
    let original_sha = fs::read_to_string(temp.path().join(".git/refs/heads/feature-1")).unwrap();

    StdCommand::new("git")
        .args(["checkout", "main"])
        .current_dir(&temp)
        .output()
        .unwrap();
    fs::write(&file, "main").unwrap();
    StdCommand::new("git")
        .args(["add", "."])
        .current_dir(&temp)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["commit", "-m", "main"])
        .current_dir(&temp)
        .output()
        .unwrap();

    // Sync pauses on conflict
    rung()
        .args(["sync", "--base", "main"])
        .current_dir(&temp)
        .assert()
        .success();

    // Abort sync
    rung()
        .args(["sync", "--abort"])
        .current_dir(&temp)
        .assert()
        .success();

    // Verify original state is restored
    let restored_sha = fs::read_to_string(temp.path().join(".git/refs/heads/feature-1")).unwrap();
    assert_eq!(
        original_sha, restored_sha,
        "Abort should restore branches to pre-sync state"
    );
}

// ============================================================================
// Undo command tests
// ============================================================================

#[test]
fn test_undo_no_backup() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Undo with no sync to undo
    rung()
        .arg("undo")
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("backup").or(predicate::str::contains("nothing to undo")));
}

// ============================================================================
// Log command tests
// ============================================================================

#[test]
fn test_log_output() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create first branch
    rung()
        .args(["create", "feature"])
        .current_dir(&temp)
        .assert()
        .success();

    // Make a commit on feature
    git_commit("Add feature", &temp);

    rung()
        .arg("log")
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicates::str::contains("Add feature"));
}

#[test]
fn test_log_json_output() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create first branch
    rung()
        .args(["create", "feature"])
        .current_dir(&temp)
        .assert()
        .success();

    // Make a commit on feature
    git_commit("Add feature", &temp);

    let output = rung()
        .args(["log", "--json"])
        .current_dir(&temp)
        .assert()
        .success();

    // Verify it's valid JSON
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(
        serde_json::from_str::<serde_json::Value>(&stdout).is_ok(),
        "Log --json should produce valid JSON"
    );
}

// ============================================================================
// Error handling tests
// ============================================================================

#[test]
fn test_command_outside_git_repo() {
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Commands should fail gracefully outside a git repo
    // Status should fail with error
    rung()
        .arg("status")
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("git repository"));

    // Create should fail with error
    rung()
        .args(["create", "test"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("git repository"));

    // Sync should fail with error
    rung()
        .arg("sync")
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("git repository"));

    // Doctor outputs to stderr but may exit 0 (diagnostic tool)
    rung()
        .arg("doctor")
        .current_dir(&temp)
        .assert()
        .stderr(predicate::str::contains("git repository"));
}

#[test]
fn test_invalid_subcommand() {
    rung()
        .arg("invalid-command")
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid"));
}

// ============================================================================
// Absorb command tests
// ============================================================================

#[test]
fn test_absorb_not_initialized() {
    let temp = setup_git_repo();

    rung()
        .arg("absorb")
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not initialized"));
}

#[test]
fn test_absorb_no_staged_changes() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a branch first (absorb requires being on a stack branch)
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();

    // Absorb with no staged changes should fail
    rung()
        .args(["absorb", "--base", "main"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("No staged changes"));
}

#[test]
fn test_absorb_dry_run_no_staged_changes() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a branch
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();

    // Absorb dry-run with no staged changes should still fail
    rung()
        .args(["absorb", "--dry-run", "--base", "main"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("No staged changes"));
}

#[test]
fn test_absorb_alias() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a branch
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();

    // Test alias 'ab' works (should fail with no staged changes)
    rung()
        .args(["ab", "--base", "main"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("No staged changes"));
}

#[test]
fn test_absorb_help_shows_in_main_help() {
    rung()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("absorb"));
}

// ============================================================================
// Adopt command tests
// ============================================================================

#[test]
fn test_adopt_not_initialized() {
    let temp = setup_git_repo();

    // Create a branch to adopt
    StdCommand::new("git")
        .args(["checkout", "-b", "feature-to-adopt"])
        .current_dir(&temp)
        .output()
        .expect("Failed to create branch");

    rung()
        .arg("adopt")
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not initialized"));
}

#[test]
fn test_adopt_branch_not_exist() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    rung()
        .args(["adopt", "nonexistent-branch", "--parent", "main"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
}

#[test]
fn test_adopt_with_explicit_parent() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a branch outside of rung
    StdCommand::new("git")
        .args(["checkout", "-b", "feature-to-adopt"])
        .current_dir(&temp)
        .output()
        .expect("Failed to create branch");

    // Adopt it with explicit parent
    rung()
        .args(["adopt", "--parent", "main"])
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("Adopted"));

    // Verify it's in the stack
    rung()
        .arg("status")
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("feature-to-adopt"));
}

#[test]
fn test_adopt_already_in_stack() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a branch via rung (adds to stack)
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();

    // Try to adopt the same branch
    rung()
        .args(["adopt", "feature-1", "--parent", "main"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("already in the stack"));
}

#[test]
fn test_adopt_dry_run() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a branch outside of rung
    StdCommand::new("git")
        .args(["checkout", "-b", "feature-to-adopt"])
        .current_dir(&temp)
        .output()
        .expect("Failed to create branch");

    // Dry run should not add to stack
    rung()
        .args(["adopt", "--parent", "main", "--dry-run"])
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("Would adopt"));

    // Verify it's NOT in the stack
    rung()
        .arg("status")
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("No branches in stack"));
}

#[test]
fn test_adopt_alias() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a branch outside of rung
    StdCommand::new("git")
        .args(["checkout", "-b", "feature-to-adopt"])
        .current_dir(&temp)
        .output()
        .expect("Failed to create branch");

    // Use 'ad' alias
    rung()
        .args(["ad", "--parent", "main"])
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("Adopted"));
}

#[test]
fn test_adopt_invalid_parent() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a branch outside of rung
    StdCommand::new("git")
        .args(["checkout", "-b", "feature-to-adopt"])
        .current_dir(&temp)
        .output()
        .expect("Failed to create branch");

    // Try to adopt with non-existent parent
    rung()
        .args(["adopt", "--parent", "nonexistent"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
}

#[test]
fn test_adopt_parent_not_in_stack() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create two branches outside of rung
    StdCommand::new("git")
        .args(["checkout", "-b", "parent-branch"])
        .current_dir(&temp)
        .output()
        .expect("Failed to create parent branch");

    StdCommand::new("git")
        .args(["checkout", "-b", "child-branch"])
        .current_dir(&temp)
        .output()
        .expect("Failed to create child branch");

    // Try to adopt child with parent that's not in stack
    rung()
        .args(["adopt", "--parent", "parent-branch"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not in the stack"));
}

#[test]
fn test_adopt_help_shows_in_main_help() {
    rung()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("adopt"))
        .stdout(predicate::str::contains("Adopt an existing branch"));
}
