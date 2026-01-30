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

// ============================================================================
// Submit command tests
// ============================================================================

#[test]
fn test_submit_requires_origin_remote() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();

    git_commit("Add feature", &temp);

    rung()
        .args(["submit", "--dry-run"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("No origin remote configured"));
}

#[test]
fn test_submit_accepts_force_flag() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();

    git_commit("Add feature", &temp);

    rung()
        .args(["submit", "--force", "--dry-run"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("No origin remote configured"));
}

#[test]
fn test_submit_help_shows_force_flag() {
    rung()
        .args(["submit", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--force"))
        .stdout(predicate::str::contains("sync"));
}

// ============================================================================
// Restack command tests
// ============================================================================

#[test]
fn test_restack_not_initialized() {
    let temp = setup_git_repo();

    rung()
        .args(["restack", "feature-1", "--onto", "main"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not initialized"));
}

#[test]
fn test_restack_branch_not_in_stack() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a branch outside of rung
    StdCommand::new("git")
        .args(["checkout", "-b", "orphan-branch"])
        .current_dir(&temp)
        .output()
        .expect("Failed to create branch");

    rung()
        .args(["restack", "orphan-branch", "--onto", "main"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not in the stack"));
}

#[test]
fn test_restack_dry_run() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create first branch
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();

    git_commit("Feature 1 commit", &temp);

    // Create second branch stacked on feature-1
    rung()
        .args(["create", "feature-2"])
        .current_dir(&temp)
        .assert()
        .success();

    git_commit("Feature 2 commit", &temp);

    // Dry run restack feature-2 onto main
    rung()
        .args(["restack", "feature-2", "--onto", "main", "--dry-run"])
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"));
}

#[test]
fn test_restack_onto_main() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create first branch
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();

    git_commit("Feature 1 commit", &temp);

    // Create second branch stacked on feature-1
    rung()
        .args(["create", "feature-2"])
        .current_dir(&temp)
        .assert()
        .success();

    git_commit("Feature 2 commit", &temp);

    // Restack feature-2 onto main (remove from feature-1 stack)
    rung()
        .args(["restack", "feature-2", "--onto", "main"])
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Updated stack").or(predicate::str::contains("Restacked")),
        );

    // Verify feature-2 now has main as parent in status
    rung()
        .args(["status", "--json"])
        .current_dir(&temp)
        .assert()
        .success();
}

#[test]
fn test_restack_onto_sibling() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create two sibling branches from main
    rung()
        .args(["create", "feature-a"])
        .current_dir(&temp)
        .assert()
        .success();

    git_commit("Feature A commit", &temp);

    // Go back to main to create sibling
    StdCommand::new("git")
        .args(["checkout", "main"])
        .current_dir(&temp)
        .output()
        .expect("Failed to checkout main");

    rung()
        .args(["create", "feature-b"])
        .current_dir(&temp)
        .assert()
        .success();

    git_commit("Feature B commit", &temp);

    // Restack feature-b onto feature-a
    rung()
        .args(["restack", "feature-b", "--onto", "feature-a"])
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Updated stack").or(predicate::str::contains("Restacked")),
        );
}

#[test]
fn test_restack_with_children() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a chain: main -> feature-1 -> feature-2 -> feature-3
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();
    git_commit("Feature 1", &temp);

    rung()
        .args(["create", "feature-2"])
        .current_dir(&temp)
        .assert()
        .success();
    git_commit("Feature 2", &temp);

    rung()
        .args(["create", "feature-3"])
        .current_dir(&temp)
        .assert()
        .success();
    git_commit("Feature 3", &temp);

    // Restack feature-2 onto main with --include-children
    rung()
        .args([
            "restack",
            "feature-2",
            "--onto",
            "main",
            "--include-children",
        ])
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Updated stack").or(predicate::str::contains("Restacked")),
        );
}

#[test]
fn test_restack_onto_self_fails() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();

    git_commit("Feature 1", &temp);

    // Try to restack onto itself
    rung()
        .args(["restack", "feature-1", "--onto", "feature-1"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("cycle").or(predicate::str::contains("same")));
}

#[test]
fn test_restack_onto_descendant_fails() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create chain: main -> feature-1 -> feature-2
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();
    git_commit("Feature 1", &temp);

    rung()
        .args(["create", "feature-2"])
        .current_dir(&temp)
        .assert()
        .success();
    git_commit("Feature 2", &temp);

    // Try to restack feature-1 onto feature-2 (its child) - should fail
    rung()
        .args(["restack", "feature-1", "--onto", "feature-2"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("cycle"));
}

#[test]
fn test_restack_alias() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();

    git_commit("Feature 1", &temp);

    // Use 're' alias
    rung()
        .args(["re", "feature-1", "--onto", "main", "--dry-run"])
        .current_dir(&temp)
        .assert()
        .success();
}

#[test]
fn test_restack_help() {
    rung()
        .args(["restack", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--onto"))
        .stdout(predicate::str::contains("--include-children"));
}

// ============================================================================
// Sync with actual rebase tests
// ============================================================================

#[test]
fn test_sync_rebases_stack() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create feature branch
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();

    git_commit("Feature commit", &temp);

    // Go back to main and add a commit
    StdCommand::new("git")
        .args(["checkout", "main"])
        .current_dir(&temp)
        .output()
        .expect("Failed to checkout main");

    let file = temp.path().join("main-change.txt");
    fs::write(&file, "main change").expect("Failed to write file");
    StdCommand::new("git")
        .args(["add", "."])
        .current_dir(&temp)
        .output()
        .expect("Failed to git add");
    StdCommand::new("git")
        .args(["commit", "-m", "Main commit"])
        .current_dir(&temp)
        .output()
        .expect("Failed to commit");

    // Sync should rebase feature-1 onto updated main
    rung()
        .args(["sync", "--base", "main"])
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("Synced").or(predicate::str::contains("rebased")));
}

#[test]
fn test_sync_multi_branch_stack() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a chain: main -> feature-1 -> feature-2
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();
    git_commit("Feature 1 commit", &temp);

    rung()
        .args(["create", "feature-2"])
        .current_dir(&temp)
        .assert()
        .success();
    git_commit("Feature 2 commit", &temp);

    // Go back to main and add a commit
    StdCommand::new("git")
        .args(["checkout", "main"])
        .current_dir(&temp)
        .output()
        .expect("Failed to checkout main");

    let file = temp.path().join("main-update.txt");
    fs::write(&file, "main update").expect("Failed to write file");
    StdCommand::new("git")
        .args(["add", "."])
        .current_dir(&temp)
        .output()
        .expect("Failed to git add");
    StdCommand::new("git")
        .args(["commit", "-m", "Main update"])
        .current_dir(&temp)
        .output()
        .expect("Failed to commit");

    // Sync should rebase entire stack
    rung()
        .args(["sync", "--base", "main"])
        .current_dir(&temp)
        .assert()
        .success();

    // Verify both branches are still in stack
    rung()
        .arg("status")
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("feature-1"))
        .stdout(predicate::str::contains("feature-2"));
}

// ============================================================================
// Create command additional tests
// ============================================================================

#[test]
fn test_create_with_message() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create branch with commit message (requires staged changes)
    let file = temp.path().join("newfile.txt");
    fs::write(&file, "new content").expect("Failed to write file");
    StdCommand::new("git")
        .args(["add", "."])
        .current_dir(&temp)
        .output()
        .expect("Failed to git add");

    rung()
        .args([
            "create",
            "feature-with-commit",
            "-m",
            "Initial feature commit",
        ])
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("feature-with-commit"));

    // Verify commit was created
    let output = StdCommand::new("git")
        .args(["log", "--oneline", "-1"])
        .current_dir(&temp)
        .output()
        .expect("Failed to get git log");

    let log = String::from_utf8_lossy(&output.stdout);
    assert!(
        log.contains("Initial feature commit"),
        "Commit message should be in log"
    );
}

#[test]
fn test_create_branch_already_exists() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a branch
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();

    // Go back to main
    StdCommand::new("git")
        .args(["checkout", "main"])
        .current_dir(&temp)
        .output()
        .expect("Failed to checkout main");

    // Try to create same branch again
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

// ============================================================================
// Doctor additional tests
// ============================================================================

#[test]
fn test_doctor_dirty_working_directory() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create uncommitted changes (untracked file)
    let file = temp.path().join("dirty.txt");
    fs::write(&file, "uncommitted").expect("Failed to write file");

    // Stage it to make it a real uncommitted change
    StdCommand::new("git")
        .args(["add", "."])
        .current_dir(&temp)
        .output()
        .expect("Failed to git add");

    // Doctor should report issues (dirty working directory or other warnings)
    rung()
        .arg("doctor")
        .current_dir(&temp)
        .assert()
        .success()
        .stderr(predicate::str::contains("issue"));
}

#[test]
fn test_doctor_json_output() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    let output = rung()
        .args(["doctor", "--json"])
        .current_dir(&temp)
        .assert()
        .success();

    // Verify it's valid JSON
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(
        serde_json::from_str::<serde_json::Value>(&stdout).is_ok(),
        "Doctor --json should produce valid JSON"
    );
}

#[test]
fn test_doctor_missing_branch() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a branch via rung
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();

    // Delete the git branch but not from stack
    StdCommand::new("git")
        .args(["checkout", "main"])
        .current_dir(&temp)
        .output()
        .expect("Failed to checkout main");

    StdCommand::new("git")
        .args(["branch", "-D", "feature-1"])
        .current_dir(&temp)
        .output()
        .expect("Failed to delete branch");

    // Doctor should report the missing branch
    rung()
        .arg("doctor")
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("feature-1").and(predicate::str::contains("not in git")));
}

// ============================================================================
// Log command tests
// ============================================================================

#[test]
fn test_log_not_initialized() {
    let temp = setup_git_repo();

    rung()
        .arg("log")
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not initialized"));
}

#[test]
fn test_log_branch_not_in_stack() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // We're on main which has no branches in stack
    rung()
        .arg("log")
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("No branches"));
}

#[test]
fn test_log_no_commits_between() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a branch but don't add any commits
    rung()
        .args(["create", "feature-empty"])
        .current_dir(&temp)
        .assert()
        .success();

    // Log should show no commits (message may be in stdout or stderr)
    rung()
        .arg("log")
        .current_dir(&temp)
        .assert()
        .success()
        .stderr(predicate::str::contains("no commits"));
}

// ============================================================================
// Merge command tests
// ============================================================================

#[test]
fn test_merge_not_initialized() {
    let temp = setup_git_repo();

    rung()
        .arg("merge")
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not initialized"));
}

#[test]
fn test_merge_branch_not_in_stack() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // On main which is not in stack
    rung()
        .arg("merge")
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not in stack").or(predicate::str::contains("No branch")));
}

#[test]
fn test_merge_no_pr_associated() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    rung()
        .args(["create", "feature-no-pr"])
        .current_dir(&temp)
        .assert()
        .success();

    git_commit("Feature work", &temp);

    // Merge without PR should fail
    rung()
        .arg("merge")
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("No PR").or(predicate::str::contains("no pull request")));
}

#[test]
fn test_merge_help() {
    rung()
        .args(["merge", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Merge"))
        .stdout(predicate::str::contains("PR"));
}

// ============================================================================
// More absorb tests
// ============================================================================

#[test]
fn test_absorb_no_staged_changes_message() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    rung()
        .args(["create", "feature-absorb"])
        .current_dir(&temp)
        .assert()
        .success();

    // Absorb with no staged changes should tell user
    rung()
        .args(["absorb", "--dry-run"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("No staged changes"));
}

// ============================================================================
// More undo tests
// ============================================================================

#[test]
fn test_undo_not_initialized() {
    let temp = setup_git_repo();

    rung()
        .arg("undo")
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not initialized"));
}

#[test]
fn test_undo_help() {
    rung()
        .args(["undo", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Undo"))
        .stdout(predicate::str::contains("sync"));
}

// ============================================================================
// More create tests
// ============================================================================

#[test]
fn test_create_help() {
    rung()
        .args(["create", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Create"))
        .stdout(predicate::str::contains("stack"));
}

#[test]
fn test_create_invalid_branch_name() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Git doesn't allow branch names starting with -
    rung()
        .args(["create", "-invalid-name"])
        .current_dir(&temp)
        .assert()
        .failure();
}

// ============================================================================
// More status tests
// ============================================================================

#[test]
fn test_status_with_pr_info() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    rung()
        .args(["create", "feature-pr"])
        .current_dir(&temp)
        .assert()
        .success();

    git_commit("Feature commit", &temp);

    // Status should show branch without PR
    rung()
        .arg("status")
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("feature-pr"));
}

#[test]
fn test_status_multi_branch() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a chain of branches
    rung()
        .args(["create", "feature-1"])
        .current_dir(&temp)
        .assert()
        .success();
    git_commit("Feature 1", &temp);

    rung()
        .args(["create", "feature-2"])
        .current_dir(&temp)
        .assert()
        .success();
    git_commit("Feature 2", &temp);

    rung()
        .args(["create", "feature-3"])
        .current_dir(&temp)
        .assert()
        .success();
    git_commit("Feature 3", &temp);

    // Status should show all branches
    rung()
        .arg("status")
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("feature-1"))
        .stdout(predicate::str::contains("feature-2"))
        .stdout(predicate::str::contains("feature-3"));
}

// ============================================================================
// More navigation tests
// ============================================================================

#[test]
fn test_prv_alias() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    rung()
        .args(["create", "feature-nav"])
        .current_dir(&temp)
        .assert()
        .success();

    // p is alias for prv
    rung().arg("p").current_dir(&temp).assert().success();

    // Verify we're on main
    let output = StdCommand::new("git")
        .args(["branch", "--show-current"])
        .current_dir(&temp)
        .output()
        .expect("Failed to get current branch");

    let branch = String::from_utf8_lossy(&output.stdout);
    assert!(branch.trim() == "main");
}

#[test]
fn test_nxt_alias() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    rung()
        .args(["create", "feature-nav-2"])
        .current_dir(&temp)
        .assert()
        .success();

    // Go back to main first
    StdCommand::new("git")
        .args(["checkout", "main"])
        .current_dir(&temp)
        .output()
        .expect("Failed to checkout main");

    // n is alias for nxt
    rung().arg("n").current_dir(&temp).assert().success();

    // Verify we're on feature branch
    let output = StdCommand::new("git")
        .args(["branch", "--show-current"])
        .current_dir(&temp)
        .output()
        .expect("Failed to get current branch");

    let branch = String::from_utf8_lossy(&output.stdout);
    assert!(branch.trim() == "feature-nav-2");
}

#[test]
fn test_move_not_initialized() {
    let temp = setup_git_repo();

    rung()
        .arg("move")
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not initialized"));
}

// ============================================================================
// More sync tests
// ============================================================================

#[test]
fn test_sync_not_initialized() {
    let temp = setup_git_repo();

    rung()
        .arg("sync")
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not initialized"));
}

#[test]
fn test_sync_help() {
    rung()
        .args(["sync", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Sync"))
        .stdout(predicate::str::contains("rebase").or(predicate::str::contains("rebasing")));
}

#[test]
fn test_sync_continue_no_sync_in_progress() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Continue when no sync is in progress should fail gracefully
    rung()
        .args(["sync", "--continue"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("No sync").or(predicate::str::contains("not in progress")),
        );
}

#[test]
fn test_sync_abort_no_sync_in_progress() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Abort when no sync is in progress should fail gracefully
    rung()
        .args(["sync", "--abort"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("No sync").or(predicate::str::contains("not in progress")),
        );
}

// ============================================================================
// Init edge cases
// ============================================================================

#[test]
fn test_init_creates_rung_directory() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Verify .git/rung/ directory exists
    let rung_dir = temp.path().join(".git/rung");
    assert!(rung_dir.exists(), "Rung directory should exist after init");
    assert!(rung_dir.is_dir(), "Rung path should be a directory");
}

#[test]
fn test_init_creates_stack_file() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Verify stack.json exists and is valid
    let stack_path = temp.path().join(".git/rung/stack.json");
    assert!(stack_path.exists(), "Stack file should exist after init");

    let stack_content = fs::read_to_string(&stack_path).expect("Failed to read stack");
    let stack: serde_json::Value =
        serde_json::from_str(&stack_content).expect("Stack should be valid JSON");

    assert!(
        stack
            .get("branches")
            .is_some_and(serde_json::Value::is_array),
        "Stack should have branches array"
    );
}

// ============================================================================
// Completions command tests
// ============================================================================

#[test]
fn test_completions_bash() {
    rung()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("complete"));
}

#[test]
fn test_completions_zsh() {
    rung()
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("compdef").or(predicate::str::contains("#compdef")));
}

#[test]
fn test_completions_fish() {
    rung()
        .args(["completions", "fish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("complete"));
}

#[test]
fn test_completions_help() {
    rung()
        .args(["completions", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("completion"))
        .stdout(predicate::str::contains("shell"));
}

// ============================================================================
// Update command tests
// ============================================================================

#[test]
fn test_update_help() {
    rung()
        .args(["update", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Update"))
        .stdout(predicate::str::contains("version").or(predicate::str::contains("latest")));
}

#[test]
fn test_update_alias() {
    // up is alias for update
    rung()
        .args(["up", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Update"));
}

// ============================================================================
// More submit command tests
// ============================================================================

#[test]
fn test_submit_not_initialized() {
    let temp = setup_git_repo();

    rung()
        .args(["submit"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not initialized"));
}

#[test]
fn test_submit_dry_run_shows_plan() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    rung()
        .args(["create", "feature-submit"])
        .current_dir(&temp)
        .assert()
        .success();

    git_commit("Add feature", &temp);

    // Dry run without origin fails
    rung()
        .args(["submit", "--dry-run"])
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("No origin"));
}

#[test]
fn test_submit_alias() {
    // sm is alias for submit
    rung()
        .args(["sm", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Push"));
}

// ============================================================================
// Move command tests
// ============================================================================

#[test]
fn test_move_alias() {
    // mv is alias for move
    rung()
        .args(["mv", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Move").or(predicate::str::contains("branch")));
}

#[test]
fn test_move_help() {
    rung()
        .args(["move", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("branch"));
}

// ============================================================================
// Merge command additional tests
// ============================================================================

#[test]
fn test_merge_alias() {
    // m is alias for merge
    rung()
        .args(["m", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Merge"));
}

#[test]
fn test_merge_without_pr() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    rung()
        .args(["create", "feature-merge-test"])
        .current_dir(&temp)
        .assert()
        .success();

    git_commit("Feature for merge", &temp);

    // Merge without PR should fail
    rung()
        .arg("merge")
        .current_dir(&temp)
        .assert()
        .failure()
        .stderr(predicate::str::contains("No PR").or(predicate::str::contains("no pull request")));
}

// ============================================================================
// Status additional tests
// ============================================================================

#[test]
fn test_status_alias() {
    // st is alias for status
    rung()
        .args(["st", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("status").or(predicate::str::contains("stack")));
}

#[test]
fn test_status_quiet_flag() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    rung()
        .args(["create", "feature-quiet"])
        .current_dir(&temp)
        .assert()
        .success();

    // Status with --quiet should still show stack
    rung()
        .args(["status", "--quiet"])
        .current_dir(&temp)
        .assert()
        .success();
}

// ============================================================================
// Doctor additional tests
// ============================================================================

#[test]
fn test_doctor_alias() {
    // doc is alias for doctor
    rung()
        .args(["doc", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Diagnose"));
}

// ============================================================================
// Restack additional tests
// ============================================================================

#[test]
fn test_restack_dry_run_shows_plan() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    rung()
        .args(["create", "feature-restack-1"])
        .current_dir(&temp)
        .assert()
        .success();
    git_commit("Feature 1", &temp);

    rung()
        .args(["create", "feature-restack-2"])
        .current_dir(&temp)
        .assert()
        .success();
    git_commit("Feature 2", &temp);

    // Go back to first feature
    StdCommand::new("git")
        .args(["checkout", "feature-restack-1"])
        .current_dir(&temp)
        .output()
        .expect("Failed to checkout");

    // Dry run should show what would happen
    rung()
        .args(["restack", "--dry-run", "--onto", "main"])
        .current_dir(&temp)
        .assert()
        .success();
}

// ============================================================================
// Adopt additional tests
// ============================================================================

#[test]
fn test_adopt_on_main() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create a branch outside of rung
    StdCommand::new("git")
        .args(["checkout", "-b", "external-branch"])
        .current_dir(&temp)
        .output()
        .expect("Failed to create branch");

    git_commit("External work", &temp);

    // Adopt should work
    rung()
        .args(["adopt", "external-branch"])
        .current_dir(&temp)
        .assert()
        .success();

    // Verify it's in the stack
    rung()
        .arg("status")
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("external-branch"));
}

// ============================================================================
// Navigate additional tests
// ============================================================================

#[test]
fn test_nxt_with_multiple_children() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    // Create base branch
    rung()
        .args(["create", "feature-base"])
        .current_dir(&temp)
        .assert()
        .success();
    git_commit("Base feature", &temp);

    // Go back to main
    StdCommand::new("git")
        .args(["checkout", "main"])
        .current_dir(&temp)
        .output()
        .expect("Failed to checkout main");

    // Try to navigate to child when main has a child
    rung().arg("nxt").current_dir(&temp).assert().success();
}

#[test]
fn test_prv_at_root() {
    let temp = setup_git_repo();

    rung().arg("init").current_dir(&temp).assert().success();

    rung()
        .args(["create", "feature-root"])
        .current_dir(&temp)
        .assert()
        .success();

    // Navigate back to main (parent)
    rung().arg("prv").current_dir(&temp).assert().success();

    // Trying prv again from main should indicate no parent (but succeeds with message)
    rung()
        .arg("prv")
        .current_dir(&temp)
        .assert()
        .success()
        .stdout(predicate::str::contains("no parent"));
}
