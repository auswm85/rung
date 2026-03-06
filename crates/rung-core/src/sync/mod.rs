//! Sync engine for rebasing stack branches.
//!
//! This module contains the core logic for the `rung sync` command,
//! which recursively rebases all branches in a stack when the base moves.

pub mod execute;
pub mod plan;
pub mod reconcile;
pub mod types;
pub mod undo;

// Re-export all public types
pub use types::*;

// Re-export all public functions
pub use execute::{abort_sync, continue_sync, execute_sync};
pub use plan::create_sync_plan;
pub use reconcile::{reconcile_merged, remove_stale_branches};
pub use undo::undo_sync;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::stack::{Stack, StackBranch};
    use crate::state::State;
    use std::fs;
    use tempfile::TempDir;

    /// Create a test repository with an initial commit
    fn init_test_repo() -> (TempDir, rung_git::Repository, git2::Repository) {
        let temp = TempDir::new().unwrap();
        let git_repo = git2::Repository::init(temp.path()).unwrap();

        // Create initial commit
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        fs::write(temp.path().join("README.md"), "# Test").unwrap();

        let mut index = git_repo.index().unwrap();
        index.add_path(std::path::Path::new("README.md")).unwrap();
        index.write().unwrap();

        let tree_id = index.write_tree().unwrap();
        let tree = git_repo.find_tree(tree_id).unwrap();
        git_repo
            .commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
            .unwrap();
        drop(tree);

        let rung_repo = rung_git::Repository::open(temp.path()).unwrap();
        (temp, rung_repo, git_repo)
    }

    /// Add a commit to the current branch
    fn add_commit(temp: &TempDir, git_repo: &git2::Repository, filename: &str, message: &str) {
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        fs::write(temp.path().join(filename), "content").unwrap();

        let mut index = git_repo.index().unwrap();
        index.add_path(std::path::Path::new(filename)).unwrap();
        index.write().unwrap();

        let tree_id = index.write_tree().unwrap();
        let tree = git_repo.find_tree(tree_id).unwrap();
        let parent = git_repo.head().unwrap().peel_to_commit().unwrap();

        git_repo
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
            .unwrap();
    }

    #[test]
    fn test_sync_plan_empty_when_synced() {
        let (_temp, rung_repo, git_repo) = init_test_repo();

        // Get main branch name
        let main_branch = rung_repo.current_branch().unwrap();

        // Create feature branch at current HEAD
        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo.branch("feature-a", &head, false).unwrap();

        // Create stack with feature-a based on main
        let mut stack = Stack::new();
        stack.add_branch(StackBranch::try_new("feature-a", Some(main_branch.clone())).unwrap());

        // Plan should be empty - feature-a is at same commit as main
        let plan = create_sync_plan(&rung_repo, &stack, &main_branch).unwrap();
        assert!(plan.is_empty());
    }

    #[test]
    fn test_sync_plan_detects_divergence() {
        let (temp, rung_repo, git_repo) = init_test_repo();

        // Get main branch name
        let main_branch = rung_repo.current_branch().unwrap();

        // Create feature branch at current HEAD
        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo.branch("feature-a", &head, false).unwrap();

        // Add a commit to main (making feature-a diverge)
        add_commit(&temp, &git_repo, "main-update.txt", "Update main");

        // Create stack with feature-a based on main
        let mut stack = Stack::new();
        stack.add_branch(StackBranch::try_new("feature-a", Some(main_branch.clone())).unwrap());

        // Plan should have one action - rebase feature-a
        let plan = create_sync_plan(&rung_repo, &stack, &main_branch).unwrap();
        assert_eq!(plan.branches.len(), 1);
        assert_eq!(plan.branches[0].branch, "feature-a");
    }

    #[test]
    fn test_sync_plan_chain() {
        let (temp, rung_repo, git_repo) = init_test_repo();

        let main_branch = rung_repo.current_branch().unwrap();

        // Create feature-a at current HEAD
        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo.branch("feature-a", &head, false).unwrap();

        // Checkout feature-a and add a commit
        git_repo.set_head("refs/heads/feature-a").unwrap();
        git_repo
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        add_commit(&temp, &git_repo, "feature-a.txt", "Feature A commit");

        // Create feature-b based on feature-a
        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo.branch("feature-b", &head, false).unwrap();

        // Go back to main and add a commit
        git_repo
            .set_head(&format!("refs/heads/{main_branch}"))
            .unwrap();
        git_repo
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        add_commit(&temp, &git_repo, "main-update.txt", "Update main");

        // Create stack
        let mut stack = Stack::new();
        stack.add_branch(StackBranch::try_new("feature-a", Some(main_branch.clone())).unwrap());
        stack.add_branch(StackBranch::try_new("feature-b", Some("feature-a")).unwrap());

        // Plan should cascade: feature-a needs rebase, so feature-b is also included
        // This ensures one sync handles the entire stack
        let plan = create_sync_plan(&rung_repo, &stack, &main_branch).unwrap();
        assert_eq!(plan.branches.len(), 2);
        assert_eq!(plan.branches[0].branch, "feature-a");
        assert_eq!(plan.branches[1].branch, "feature-b");
    }

    #[test]
    fn test_sync_plan_cascade_deep_stack() {
        let (temp, rung_repo, git_repo) = init_test_repo();

        let main_branch = rung_repo.current_branch().unwrap();

        // Create a 4-branch deep stack: main → A → B → C → D
        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo.branch("feature-a", &head, false).unwrap();

        git_repo.set_head("refs/heads/feature-a").unwrap();
        git_repo
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        add_commit(&temp, &git_repo, "a.txt", "A commit");

        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo.branch("feature-b", &head, false).unwrap();

        git_repo.set_head("refs/heads/feature-b").unwrap();
        git_repo
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        add_commit(&temp, &git_repo, "b.txt", "B commit");

        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo.branch("feature-c", &head, false).unwrap();

        git_repo.set_head("refs/heads/feature-c").unwrap();
        git_repo
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        add_commit(&temp, &git_repo, "c.txt", "C commit");

        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo.branch("feature-d", &head, false).unwrap();

        // Go back to main and add a commit (causes cascade)
        git_repo
            .set_head(&format!("refs/heads/{main_branch}"))
            .unwrap();
        git_repo
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        add_commit(&temp, &git_repo, "main-update.txt", "Update main");

        // Create stack
        let mut stack = Stack::new();
        stack.add_branch(StackBranch::try_new("feature-a", Some(main_branch.clone())).unwrap());
        stack.add_branch(StackBranch::try_new("feature-b", Some("feature-a")).unwrap());
        stack.add_branch(StackBranch::try_new("feature-c", Some("feature-b")).unwrap());
        stack.add_branch(StackBranch::try_new("feature-d", Some("feature-c")).unwrap());

        // Plan should cascade through entire stack in one pass
        let plan = create_sync_plan(&rung_repo, &stack, &main_branch).unwrap();
        assert_eq!(plan.branches.len(), 4);
        assert_eq!(plan.branches[0].branch, "feature-a");
        assert_eq!(plan.branches[1].branch, "feature-b");
        assert_eq!(plan.branches[2].branch, "feature-c");
        assert_eq!(plan.branches[3].branch, "feature-d");
    }

    #[test]
    fn test_remove_stale_branches() {
        let (temp, rung_repo, git_repo) = init_test_repo();
        let state = State::new(temp.path()).unwrap();
        state.init().unwrap();

        let main_branch = rung_repo.current_branch().unwrap();

        // Create two branches
        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo.branch("feature-a", &head, false).unwrap();
        git_repo.branch("feature-b", &head, false).unwrap();

        // Create stack: main -> feature-a -> feature-b
        let mut stack = Stack::new();
        stack.add_branch(StackBranch::try_new("feature-a", Some(main_branch.clone())).unwrap());
        stack.add_branch(StackBranch::try_new("feature-b", Some("feature-a")).unwrap());
        state.save_stack(&stack).unwrap();

        // Delete feature-a git (making stale)
        rung_repo.delete_branch("feature-a").unwrap();

        // Run stale branch removal
        let result = remove_stale_branches(&rung_repo, &state).unwrap();

        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0], "feature-a");

        // Verify stack was updated: feature-b should now point to main

        let updated_stack = state.load_stack().unwrap();
        assert_eq!(updated_stack.len(), 1);
        let b = updated_stack.find_branch("feature-b").unwrap();
        assert_eq!(b.parent.as_ref().unwrap().as_str(), main_branch.as_str());
    }

    #[test]
    fn test_execute_sync_with_conflict() {
        let (temp, rung_repo, git_repo) = init_test_repo();
        let state = State::new(temp.path()).unwrap();
        state.init().unwrap();

        let main_branch = rung_repo.current_branch().unwrap();

        // Setup a conflict: modify same line in the main feature-a
        fs::write(temp.path().join("conflict.txt"), "Original\n").unwrap();
        add_commit(&temp, &git_repo, "conflict.txt", "Initial");

        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo.branch("feature-a", &head, false).unwrap();

        // Change on main
        {
            let sig = git2::Signature::now("Test", "test@example.com").unwrap();
            fs::write(temp.path().join("conflict.txt"), "Main content\n").unwrap(); // UNIQUE
            let mut index = git_repo.index().unwrap();
            index
                .add_path(std::path::Path::new("conflict.txt"))
                .unwrap();
            index.write().unwrap();
            let tree_id = index.write_tree().unwrap();
            let tree = git_repo.find_tree(tree_id).unwrap();
            let parent = git_repo.head().unwrap().peel_to_commit().unwrap();
            git_repo
                .commit(Some("HEAD"), &sig, &sig, "Main change", &tree, &[&parent])
                .unwrap();
        }

        // Change on feature-a
        git_repo.set_head("refs/heads/feature-a").unwrap();

        git_repo
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        {
            let sig = git2::Signature::now("Test", "test@example.com").unwrap();
            fs::write(temp.path().join("conflict.txt"), "Feature content\n").unwrap(); // UNIQUE
            let mut index = git_repo.index().unwrap();
            index
                .add_path(std::path::Path::new("conflict.txt"))
                .unwrap();
            index.write().unwrap();
            let tree_id = index.write_tree().unwrap();
            let tree = git_repo.find_tree(tree_id).unwrap();
            let parent = git_repo.head().unwrap().peel_to_commit().unwrap();
            git_repo
                .commit(
                    Some("HEAD"),
                    &sig,
                    &sig,
                    "Feature-a change",
                    &tree,
                    &[&parent],
                )
                .unwrap();
        }

        // Create stack
        let mut stack = Stack::new();
        stack.add_branch(StackBranch::try_new("feature-a", Some(main_branch.clone())).unwrap());
        state.save_stack(&stack).unwrap();

        // Plan sync
        let plan = create_sync_plan(&rung_repo, &stack, &main_branch).unwrap();

        // Execute sync - should be paused by conflict
        let result = execute_sync(&rung_repo, &state, plan).unwrap();
        match result {
            SyncResult::Paused {
                at_branch,
                conflict_files,
                ..
            } => {
                assert_eq!(at_branch, "feature-a");
                assert!(conflict_files.contains(&"conflict.txt".to_string()));
            }
            _ => panic!("Expected sync to be paused by conflict, but got {result:?}"),
        }

        assert!(state.is_sync_in_progress());
    }

    #[test]
    fn test_reconcile_merged_empty() {
        let (temp, _rung_repo, _git_repo) = init_test_repo();
        let state = State::new(temp.path()).unwrap();
        state.init().unwrap();

        // Empty merged list should return empty result
        let result = reconcile_merged(&state, &[]).unwrap();
        assert!(result.merged.is_empty());
        assert!(result.reparented.is_empty());
        assert!(result.repaired.is_empty());
    }

    #[test]
    fn test_reconcile_merged_with_children() {
        let (temp, rung_repo, git_repo) = init_test_repo();
        let state = State::new(temp.path()).unwrap();
        state.init().unwrap();

        let main_branch = rung_repo.current_branch().unwrap();

        // Create feature-a at current HEAD
        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo.branch("feature-a", &head, false).unwrap();

        // Create stack: main -> feature-a -> feature-b
        let mut stack = Stack::new();
        stack.add_branch(StackBranch::try_new("feature-a", Some(main_branch.as_str())).unwrap());
        let mut branch_b = StackBranch::try_new("feature-b", Some("feature-a")).unwrap();
        branch_b.pr = Some(456);
        stack.add_branch(branch_b);
        state.save_stack(&stack).unwrap();

        // Simulate feature-a being merged into main
        let merged_prs = vec![ExternalMergeInfo {
            branch_name: "feature-a".to_string(),
            pr_number: 123,
            merged_into: main_branch.clone(),
        }];

        let result = reconcile_merged(&state, &merged_prs).unwrap();

        // feature-a should be in merged list
        assert_eq!(result.merged.len(), 1);
        assert_eq!(result.merged[0].name, "feature-a");
        assert_eq!(result.merged[0].pr_number, 123);

        // feature-b should be reparented to main
        assert_eq!(result.reparented.len(), 1);
        assert_eq!(result.reparented[0].name, "feature-b");
        assert_eq!(result.reparented[0].old_parent, "feature-a");
        assert_eq!(result.reparented[0].new_parent, main_branch.as_str());
        assert_eq!(result.reparented[0].pr_number, Some(456));

        // Verify stack was updated
        let updated_stack = state.load_stack().unwrap();
        assert!(updated_stack.find_branch("feature-a").is_none());
        let b = updated_stack.find_branch("feature-b").unwrap();
        assert_eq!(b.parent.as_ref().unwrap().as_str(), main_branch.as_str());
    }

    #[test]
    fn test_undo_sync() {
        let (temp, rung_repo, git_repo) = init_test_repo();
        let state = State::new(temp.path()).unwrap();
        state.init().unwrap();

        let main_branch = rung_repo.current_branch().unwrap();

        // Create feature-a at current HEAD
        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo.branch("feature-a", &head, false).unwrap();

        // Get the original SHA of feature-a
        let _original_sha = head.id().to_string();

        // Checkout feature-a and add a commit
        git_repo.set_head("refs/heads/feature-a").unwrap();
        git_repo
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        add_commit(&temp, &git_repo, "feature-a.txt", "Feature A commit");

        // Get the new SHA
        let new_sha = git_repo
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .id()
            .to_string();

        // Create a backup (simulating what sync does)
        let backup_refs = vec![("feature-a", new_sha.as_str())];
        let backup_id = state.create_backup(&backup_refs).unwrap();

        // Modify the backup to point to original SHA (simulating pre-sync state)
        // Actually, let's just test that undo_sync works with a valid backup
        // by creating a backup with current state and verifying it can be restored

        // Create stack
        let mut stack = Stack::new();
        stack.add_branch(StackBranch::try_new("feature-a", Some(main_branch)).unwrap());
        state.save_stack(&stack).unwrap();

        // Now undo should restore from the backup
        let result = undo_sync(&rung_repo, &state).unwrap();

        assert_eq!(result.branches_restored, 1);
        assert_eq!(result.backup_id, backup_id);
    }

    #[test]
    fn test_sync_plan_base_branch_not_found() {
        let (_temp, rung_repo, git_repo) = init_test_repo();

        let _main_branch = rung_repo.current_branch().unwrap();

        // Create feature-a
        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo.branch("feature-a", &head, false).unwrap();

        // Create stack with feature-a pointing to non-existent base
        let mut stack = Stack::new();
        stack.add_branch(StackBranch::try_new("feature-a", None::<&str>).unwrap());

        // Plan with non-existent base branch should error
        let result = create_sync_plan(&rung_repo, &stack, "nonexistent-branch");
        assert!(result.is_err());
    }

    #[test]
    fn test_sync_plan_skips_stale_branches() {
        let (_temp, rung_repo, git_repo) = init_test_repo();

        let main_branch = rung_repo.current_branch().unwrap();

        // Create only feature-a in git
        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo.branch("feature-a", &head, false).unwrap();

        // But stack has both feature-a and feature-b (stale)
        let mut stack = Stack::new();
        stack.add_branch(StackBranch::try_new("feature-a", Some(main_branch.clone())).unwrap());
        stack.add_branch(StackBranch::try_new("feature-b", Some("feature-a")).unwrap());

        // Plan should only include feature-a (feature-b is stale)
        let plan = create_sync_plan(&rung_repo, &stack, &main_branch).unwrap();
        // feature-a is at same commit as main, so plan is empty
        // but importantly, it didn't error on the stale feature-b
        assert!(plan.is_empty());
    }
}
