//! Repository wrapper providing high-level git operations.

use std::path::{Path, PathBuf};

use git2::{BranchType, Oid, RepositoryState, Signature};

use crate::error::{Error, Result};

/// High-level wrapper around a git repository.
pub struct Repository {
    inner: git2::Repository,
}

impl Repository {
    /// Open a repository at the given path.
    ///
    /// # Errors
    /// Returns error if no repository found at path or any parent.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let inner = git2::Repository::discover(path)?;
        Ok(Self { inner })
    }

    /// Open the repository containing the current directory.
    ///
    /// # Errors
    /// Returns error if not inside a git repository.
    pub fn open_current() -> Result<Self> {
        Self::open(".")
    }

    /// Get the path to the repository root (workdir).
    #[must_use]
    pub fn workdir(&self) -> Option<&Path> {
        self.inner.workdir()
    }

    /// Get the path to the .git directory.
    #[must_use]
    pub fn git_dir(&self) -> &Path {
        self.inner.path()
    }

    /// Get the current repository state.
    #[must_use]
    pub fn state(&self) -> RepositoryState {
        self.inner.state()
    }

    /// Check if there's a rebase in progress.
    #[must_use]
    pub fn is_rebasing(&self) -> bool {
        matches!(
            self.state(),
            RepositoryState::Rebase
                | RepositoryState::RebaseInteractive
                | RepositoryState::RebaseMerge
        )
    }

    // === Branch operations ===

    /// Get the name of the current branch.
    ///
    /// # Errors
    /// Returns error if HEAD is detached.
    pub fn current_branch(&self) -> Result<String> {
        let head = self.inner.head()?;
        if !head.is_branch() {
            return Err(Error::DetachedHead);
        }

        head.shorthand()
            .map(String::from)
            .ok_or(Error::DetachedHead)
    }

    /// Get the commit SHA for a branch.
    ///
    /// # Errors
    /// Returns error if branch doesn't exist.
    pub fn branch_commit(&self, branch_name: &str) -> Result<Oid> {
        let branch = self
            .inner
            .find_branch(branch_name, BranchType::Local)
            .map_err(|_| Error::BranchNotFound(branch_name.into()))?;

        branch
            .get()
            .target()
            .ok_or_else(|| Error::BranchNotFound(branch_name.into()))
    }

    /// Create a new branch at the current HEAD.
    ///
    /// # Errors
    /// Returns error if branch creation fails.
    pub fn create_branch(&self, name: &str) -> Result<Oid> {
        let head_commit = self.inner.head()?.peel_to_commit()?;
        let branch = self.inner.branch(name, &head_commit, false)?;

        branch
            .get()
            .target()
            .ok_or_else(|| Error::BranchNotFound(name.into()))
    }

    /// Checkout a branch.
    ///
    /// # Errors
    /// Returns error if checkout fails.
    pub fn checkout(&self, branch_name: &str) -> Result<()> {
        let branch = self
            .inner
            .find_branch(branch_name, BranchType::Local)
            .map_err(|_| Error::BranchNotFound(branch_name.into()))?;

        let reference = branch.get();
        let object = reference.peel(git2::ObjectType::Commit)?;

        self.inner.checkout_tree(&object, None)?;
        self.inner
            .set_head(&format!("refs/heads/{branch_name}"))?;

        Ok(())
    }

    /// List all local branches.
    ///
    /// # Errors
    /// Returns error if branch listing fails.
    pub fn list_branches(&self) -> Result<Vec<String>> {
        let branches = self.inner.branches(Some(BranchType::Local))?;

        let names: Vec<String> = branches
            .filter_map(|b| b.ok())
            .filter_map(|(b, _)| b.name().ok().flatten().map(String::from))
            .collect();

        Ok(names)
    }

    /// Check if a branch exists.
    #[must_use]
    pub fn branch_exists(&self, name: &str) -> bool {
        self.inner.find_branch(name, BranchType::Local).is_ok()
    }

    // === Working directory state ===

    /// Check if the working directory is clean.
    ///
    /// # Errors
    /// Returns error if status check fails.
    pub fn is_clean(&self) -> Result<bool> {
        let statuses = self.inner.statuses(None)?;
        Ok(statuses.is_empty())
    }

    /// Ensure working directory is clean, returning error if not.
    ///
    /// # Errors
    /// Returns `DirtyWorkingDirectory` if there are uncommitted changes.
    pub fn require_clean(&self) -> Result<()> {
        if self.is_clean()? {
            Ok(())
        } else {
            Err(Error::DirtyWorkingDirectory)
        }
    }

    // === Commit operations ===

    /// Get a commit by its SHA.
    ///
    /// # Errors
    /// Returns error if commit not found.
    pub fn find_commit(&self, oid: Oid) -> Result<git2::Commit<'_>> {
        Ok(self.inner.find_commit(oid)?)
    }

    /// Get the merge base between two commits.
    ///
    /// # Errors
    /// Returns error if merge base calculation fails.
    pub fn merge_base(&self, one: Oid, two: Oid) -> Result<Oid> {
        Ok(self.inner.merge_base(one, two)?)
    }

    /// Count commits between two points.
    ///
    /// # Errors
    /// Returns error if revwalk fails.
    pub fn count_commits_between(&self, from: Oid, to: Oid) -> Result<usize> {
        let mut revwalk = self.inner.revwalk()?;
        revwalk.push(to)?;
        revwalk.hide(from)?;

        Ok(revwalk.count())
    }

    // === Reset operations ===

    /// Hard reset a branch to a specific commit.
    ///
    /// # Errors
    /// Returns error if reset fails.
    pub fn reset_branch(&self, branch_name: &str, target: Oid) -> Result<()> {
        let commit = self.inner.find_commit(target)?;
        let reference_name = format!("refs/heads/{branch_name}");

        self.inner.reference(
            &reference_name,
            target,
            true, // force
            &format!("rung: reset to {}", &target.to_string()[..8]),
        )?;

        // If this is the current branch, also update working directory
        if self.current_branch().ok().as_deref() == Some(branch_name) {
            self.inner
                .reset(commit.as_object(), git2::ResetType::Hard, None)?;
        }

        Ok(())
    }

    // === Signature ===

    /// Get the default signature for commits.
    ///
    /// # Errors
    /// Returns error if git config doesn't have user.name/email.
    pub fn signature(&self) -> Result<Signature<'_>> {
        Ok(self.inner.signature()?)
    }

    // === Low-level access ===

    /// Get a reference to the underlying git2 repository.
    ///
    /// Use sparingly - prefer high-level methods.
    #[must_use]
    pub fn inner(&self) -> &git2::Repository {
        &self.inner
    }
}

impl std::fmt::Debug for Repository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Repository")
            .field("path", &self.git_dir())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn init_test_repo() -> (TempDir, Repository) {
        let temp = TempDir::new().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();

        // Create initial commit (scoped to drop borrows before moving repo)
        {
            let sig = repo.signature().unwrap();
            let tree_id = repo.index().unwrap().write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
                .unwrap();
        }

        let wrapped = Repository { inner: repo };
        (temp, wrapped)
    }

    #[test]
    fn test_current_branch() {
        let (_temp, repo) = init_test_repo();
        // Default branch after init
        let branch = repo.current_branch().unwrap();
        assert!(branch == "main" || branch == "master");
    }

    #[test]
    fn test_create_and_checkout_branch() {
        let (_temp, repo) = init_test_repo();

        repo.create_branch("feature/test").unwrap();
        assert!(repo.branch_exists("feature/test"));

        repo.checkout("feature/test").unwrap();
        assert_eq!(repo.current_branch().unwrap(), "feature/test");
    }

    #[test]
    fn test_is_clean() {
        let (temp, repo) = init_test_repo();

        assert!(repo.is_clean().unwrap());

        // Create untracked file
        fs::write(temp.path().join("new_file.txt"), "content").unwrap();
        assert!(!repo.is_clean().unwrap());
    }

    #[test]
    fn test_list_branches() {
        let (_temp, repo) = init_test_repo();

        repo.create_branch("feature/a").unwrap();
        repo.create_branch("feature/b").unwrap();

        let branches = repo.list_branches().unwrap();
        assert!(branches.len() >= 3); // main/master + 2 features
        assert!(branches.iter().any(|b| b == "feature/a"));
        assert!(branches.iter().any(|b| b == "feature/b"));
    }
}
