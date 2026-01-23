//! Repository wrapper providing high-level git operations.

use std::path::Path;

use git2::{BranchType, Oid, RepositoryState, Signature};

use crate::error::{Error, Result};

/// Divergence state between a local branch and its tracking remote (upstream, falls back to origin).
///
/// This is distinct from `BranchState::Diverged` which tracks divergence from the
/// *parent branch* (needs sync). `RemoteDivergence` tracks local vs remote (needs push/pull).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteDivergence {
    /// Local and remote are at the same commit.
    InSync,
    /// Local has commits not on remote (safe push).
    Ahead {
        /// Number of commits local is ahead of remote.
        commits: usize,
    },
    /// Remote has commits not on local (need pull).
    Behind {
        /// Number of commits local is behind remote.
        commits: usize,
    },
    /// Both have unique commits (need force push after rebase).
    Diverged {
        /// Number of commits local is ahead of remote.
        ahead: usize,
        /// Number of commits local is behind remote.
        behind: usize,
    },
    /// No remote tracking branch exists (first push).
    NoRemote,
}

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

    /// Check if HEAD is detached (not pointing at a branch).
    ///
    /// # Errors
    /// Returns error if HEAD cannot be read (e.g. unborn repo).
    pub fn head_detached(&self) -> Result<bool> {
        let head = self.inner.head()?;
        Ok(!head.is_branch())
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

    /// Get the commit ID of a remote branch tip.
    ///
    /// Uses the configured upstream if set, otherwise falls back to `origin/<branch>`.
    ///
    /// # Errors
    /// Returns error if branch not found.
    pub fn remote_branch_commit(&self, branch_name: &str) -> Result<Oid> {
        // Try configured upstream first, fall back to origin/<branch>
        let remote_ref = self
            .branch_upstream_ref(branch_name)
            .unwrap_or_else(|| format!("refs/remotes/origin/{branch_name}"));

        let reference = self
            .inner
            .find_reference(&remote_ref)
            .map_err(|_| Error::BranchNotFound(remote_ref.clone()))?;

        reference.target().ok_or(Error::BranchNotFound(remote_ref))
    }

    /// Get the configured upstream ref for a branch, if any.
    ///
    /// Returns `None` if no upstream is configured or the branch doesn't exist.
    /// Uses `branch_upstream_name` to read from git config, which works even when
    /// the remote-tracking ref doesn't exist locally.
    fn branch_upstream_ref(&self, branch_name: &str) -> Option<String> {
        let refname = format!("refs/heads/{branch_name}");
        let upstream_buf = self.inner.branch_upstream_name(&refname).ok()?;
        upstream_buf.as_str().map(String::from)
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
        self.inner.set_head(&format!("refs/heads/{branch_name}"))?;

        Ok(())
    }

    /// List all local branches.
    ///
    /// # Errors
    /// Returns error if branch listing fails.
    pub fn list_branches(&self) -> Result<Vec<String>> {
        let branches = self.inner.branches(Some(BranchType::Local))?;

        let names: Vec<String> = branches
            .filter_map(std::result::Result::ok)
            .filter_map(|(b, _)| b.name().ok().flatten().map(String::from))
            .collect();

        Ok(names)
    }

    /// Check if a branch exists.
    #[must_use]
    pub fn branch_exists(&self, name: &str) -> bool {
        self.inner.find_branch(name, BranchType::Local).is_ok()
    }

    /// Delete a local branch.
    ///
    /// # Errors
    /// Returns error if branch deletion fails.
    pub fn delete_branch(&self, name: &str) -> Result<()> {
        let mut branch = self.inner.find_branch(name, BranchType::Local)?;
        branch.delete()?;
        Ok(())
    }

    // === Working directory state ===

    /// Check if the working directory is clean (no modified or staged files).
    ///
    /// Untracked files are ignored - only tracked files that have been
    /// modified or staged count as "dirty".
    ///
    /// # Errors
    /// Returns error if status check fails.
    pub fn is_clean(&self) -> Result<bool> {
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(false)
            .include_ignored(false)
            .include_unmodified(false)
            .exclude_submodules(true);
        let statuses = self.inner.statuses(Some(&mut opts))?;

        // Check if any status indicates modified/staged files
        for entry in statuses.iter() {
            let status = entry.status();
            // These indicate actual changes to tracked files
            if status.intersects(
                git2::Status::INDEX_NEW
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::INDEX_DELETED
                    | git2::Status::INDEX_RENAMED
                    | git2::Status::INDEX_TYPECHANGE
                    | git2::Status::WT_MODIFIED
                    | git2::Status::WT_DELETED
                    | git2::Status::WT_TYPECHANGE
                    | git2::Status::WT_RENAMED,
            ) {
                return Ok(false);
            }
        }
        Ok(true)
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

    // === Staging operations ===

    /// Stage all changes (tracked and untracked files).
    ///
    /// Equivalent to `git add -A`.
    ///
    /// # Errors
    /// Returns error if staging fails.
    pub fn stage_all(&self) -> Result<()> {
        let workdir = self.workdir().ok_or(Error::NotARepository)?;

        let output = std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(workdir)
            .output()
            .map_err(|e| Error::Git2(git2::Error::from_str(&e.to_string())))?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(Error::Git2(git2::Error::from_str(&stderr)))
        }
    }

    /// Check if there are staged changes ready to commit.
    ///
    /// # Errors
    /// Returns error if status check fails.
    pub fn has_staged_changes(&self) -> Result<bool> {
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(false)
            .include_ignored(false)
            .include_unmodified(false);
        let statuses = self.inner.statuses(Some(&mut opts))?;

        for entry in statuses.iter() {
            let status = entry.status();
            if status.intersects(
                git2::Status::INDEX_NEW
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::INDEX_DELETED
                    | git2::Status::INDEX_RENAMED
                    | git2::Status::INDEX_TYPECHANGE,
            ) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Create a commit with the given message on HEAD.
    ///
    /// Handles both normal commits (with parent) and initial commits (no parent).
    ///
    /// # Errors
    /// Returns error if commit creation fails.
    pub fn create_commit(&self, message: &str) -> Result<Oid> {
        let sig = self.signature()?;
        let mut index = self.inner.index()?;
        let tree_id = index.write_tree()?;
        let tree = self.inner.find_tree(tree_id)?;

        // Handle initial commit case (unborn HEAD)
        let oid = match self.inner.head().and_then(|h| h.peel_to_commit()) {
            Ok(parent) => {
                self.inner
                    .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])?
            }
            Err(_) => {
                // Initial commit - no parent
                self.inner
                    .commit(Some("HEAD"), &sig, &sig, message, &tree, &[])?
            }
        };

        Ok(oid)
    }

    // === Commit operations ===

    /// Get a commit by its SHA.
    ///
    /// # Errors
    /// Returns error if commit not found.
    pub fn find_commit(&self, oid: Oid) -> Result<git2::Commit<'_>> {
        Ok(self.inner.find_commit(oid)?)
    }

    /// Get the commit message from a branch's tip commit.
    ///
    /// # Errors
    /// Returns error if branch doesn't exist or has no commits.
    pub fn branch_commit_message(&self, branch_name: &str) -> Result<String> {
        let oid = self.branch_commit(branch_name)?;
        let commit = self.inner.find_commit(oid)?;
        commit
            .message()
            .map(String::from)
            .ok_or_else(|| Error::Git2(git2::Error::from_str("commit has no message")))
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

    /// Get commits between two points.
    ///
    /// # Errors
    /// Return error if revwalk fails.
    pub fn commits_between(&self, from: Oid, to: Oid) -> Result<Vec<Oid>> {
        let mut revwalk = self.inner.revwalk()?;
        revwalk.push(to)?;
        revwalk.hide(from)?;

        let mut commits = Vec::new();
        for oid in revwalk {
            let oid = oid?;
            commits.push(oid);
        }

        Ok(commits)
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

    // === Rebase operations ===

    /// Rebase the current branch onto a target commit.
    ///
    /// Returns `Ok(())` on success, or `Err(RebaseConflict)` if there are conflicts.
    ///
    /// # Errors
    /// Returns error if rebase fails or conflicts occur.
    pub fn rebase_onto(&self, target: Oid) -> Result<()> {
        let workdir = self.workdir().ok_or(Error::NotARepository)?;

        let output = std::process::Command::new("git")
            .args(["rebase", &target.to_string()])
            .current_dir(workdir)
            .output()
            .map_err(|e| Error::RebaseFailed(e.to_string()))?;

        if output.status.success() {
            return Ok(());
        }

        // Check if it's a conflict
        if self.is_rebasing() {
            let conflicts = self.conflicting_files()?;
            return Err(Error::RebaseConflict(conflicts));
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(Error::RebaseFailed(stderr.to_string()))
    }

    /// Rebase the current branch onto a new base, replaying only commits after `old_base`.
    ///
    /// This is equivalent to `git rebase --onto <new_base> <old_base>`.
    /// Use this when the `old_base` was squash-merged and you want to bring only
    /// the unique commits from the current branch.
    ///
    /// # Errors
    /// Returns error if rebase fails or conflicts occur.
    pub fn rebase_onto_from(&self, new_base: Oid, old_base: Oid) -> Result<()> {
        let workdir = self.workdir().ok_or(Error::NotARepository)?;

        let output = std::process::Command::new("git")
            .args([
                "rebase",
                "--onto",
                &new_base.to_string(),
                &old_base.to_string(),
            ])
            .current_dir(workdir)
            .output()
            .map_err(|e| Error::RebaseFailed(e.to_string()))?;

        if output.status.success() {
            return Ok(());
        }

        // Check if it's a conflict
        if self.is_rebasing() {
            let conflicts = self.conflicting_files()?;
            return Err(Error::RebaseConflict(conflicts));
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(Error::RebaseFailed(stderr.to_string()))
    }

    /// Get list of files with conflicts.
    ///
    /// # Errors
    /// Returns error if status check fails.
    pub fn conflicting_files(&self) -> Result<Vec<String>> {
        let statuses = self.inner.statuses(None)?;
        let conflicts: Vec<String> = statuses
            .iter()
            .filter(|s| s.status().is_conflicted())
            .filter_map(|s| s.path().map(String::from))
            .collect();
        Ok(conflicts)
    }

    /// Abort an in-progress rebase.
    ///
    /// # Errors
    /// Returns error if abort fails.
    pub fn rebase_abort(&self) -> Result<()> {
        let workdir = self.workdir().ok_or(Error::NotARepository)?;

        let output = std::process::Command::new("git")
            .args(["rebase", "--abort"])
            .current_dir(workdir)
            .output()
            .map_err(|e| Error::RebaseFailed(e.to_string()))?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(Error::RebaseFailed(stderr.to_string()))
        }
    }

    /// Continue an in-progress rebase.
    ///
    /// # Errors
    /// Returns error if continue fails or new conflicts occur.
    pub fn rebase_continue(&self) -> Result<()> {
        let workdir = self.workdir().ok_or(Error::NotARepository)?;

        let output = std::process::Command::new("git")
            .args(["rebase", "--continue"])
            .current_dir(workdir)
            .output()
            .map_err(|e| Error::RebaseFailed(e.to_string()))?;

        if output.status.success() {
            return Ok(());
        }

        // Check if it's a conflict
        if self.is_rebasing() {
            let conflicts = self.conflicting_files()?;
            return Err(Error::RebaseConflict(conflicts));
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(Error::RebaseFailed(stderr.to_string()))
    }

    // === Remote operations ===

    /// Check how a local branch relates to its remote counterpart.
    ///
    /// Uses the configured upstream if set, otherwise falls back to `origin/<branch>`.
    /// Compares the local branch tip with the remote tracking branch to determine
    /// if the local branch is ahead, behind, diverged, or in sync with the remote.
    ///
    /// Uses `graph_ahead_behind` for efficient single-traversal computation.
    ///
    /// # Errors
    /// Returns error if branch doesn't exist or git operations fail.
    pub fn remote_divergence(&self, branch: &str) -> Result<RemoteDivergence> {
        let local = self.branch_commit(branch)?;

        // Try to get remote - NoRemote if doesn't exist
        let remote = match self.remote_branch_commit(branch) {
            Ok(oid) => oid,
            Err(Error::BranchNotFound(_)) => return Ok(RemoteDivergence::NoRemote),
            Err(e) => return Err(e),
        };

        if local == remote {
            return Ok(RemoteDivergence::InSync);
        }

        // Use graph_ahead_behind for efficient single-traversal computation.
        // NotFound means no merge base (unrelated histories) - treat as (0, 0).
        let (ahead, behind) = match self.inner.graph_ahead_behind(local, remote) {
            Ok(counts) => counts,
            Err(e) if e.code() == git2::ErrorCode::NotFound => (0, 0),
            Err(e) => return Err(Error::Git2(e)),
        };

        // Unrelated histories: (0, 0) but local != remote. Count all commits on each side.
        if ahead == 0 && behind == 0 {
            return Ok(RemoteDivergence::Diverged {
                ahead: self.count_all_commits(local)?,
                behind: self.count_all_commits(remote)?,
            });
        }

        Ok(match (ahead, behind) {
            (a, 0) => RemoteDivergence::Ahead { commits: a },
            (0, b) => RemoteDivergence::Behind { commits: b },
            (a, b) => RemoteDivergence::Diverged {
                ahead: a,
                behind: b,
            },
        })
    }

    /// Count all commits reachable from a given commit.
    ///
    /// Used for unrelated histories where there's no merge base.
    fn count_all_commits(&self, from: Oid) -> Result<usize> {
        let mut revwalk = self.inner.revwalk()?;
        revwalk.push(from)?;
        Ok(revwalk.count())
    }

    /// Get the URL of the origin remote.
    ///
    /// # Errors
    /// Returns error if origin remote is not found.
    pub fn origin_url(&self) -> Result<String> {
        let remote = self
            .inner
            .find_remote("origin")
            .map_err(|_| Error::RemoteNotFound("origin".into()))?;

        remote
            .url()
            .map(String::from)
            .ok_or_else(|| Error::RemoteNotFound("origin".into()))
    }

    /// Detect the default branch from the remote's HEAD.
    ///
    /// Checks `refs/remotes/origin/HEAD` to determine the remote's default branch.
    /// Returns `None` if the remote HEAD is not set (e.g., fresh clone without `--set-upstream`).
    #[must_use]
    pub fn detect_default_branch(&self) -> Option<String> {
        // Try to resolve refs/remotes/origin/HEAD which points to the default branch
        let reference = self.inner.find_reference("refs/remotes/origin/HEAD").ok()?;

        // Resolve the symbolic reference to get the actual branch
        let resolved = reference.resolve().ok()?;
        let name = resolved.name()?;

        // Extract branch name from "refs/remotes/origin/main" -> "main"
        name.strip_prefix("refs/remotes/origin/").map(String::from)
    }

    /// Parse owner and repo name from a GitHub URL.
    ///
    /// Supports both HTTPS and SSH URLs:
    /// - `https://github.com/owner/repo.git`
    /// - `git@github.com:owner/repo.git`
    ///
    /// # Errors
    /// Returns error if URL cannot be parsed.
    pub fn parse_github_remote(url: &str) -> Result<(String, String)> {
        // SSH format: git@github.com:owner/repo.git
        if let Some(rest) = url.strip_prefix("git@github.com:") {
            let path = rest.strip_suffix(".git").unwrap_or(rest);
            if let Some((owner, repo)) = path.split_once('/') {
                return Ok((owner.to_string(), repo.to_string()));
            }
        }

        // HTTPS format: https://github.com/owner/repo.git
        if let Some(rest) = url
            .strip_prefix("https://github.com/")
            .or_else(|| url.strip_prefix("http://github.com/"))
        {
            let path = rest.strip_suffix(".git").unwrap_or(rest);
            if let Some((owner, repo)) = path.split_once('/') {
                return Ok((owner.to_string(), repo.to_string()));
            }
        }

        Err(Error::InvalidRemoteUrl(url.to_string()))
    }

    /// Push a branch to the remote.
    ///
    /// # Errors
    /// Returns error if push fails.
    pub fn push(&self, branch: &str, force: bool) -> Result<()> {
        let workdir = self.workdir().ok_or(Error::NotARepository)?;

        let mut args = vec!["push", "-u", "origin", branch];
        if force {
            args.insert(1, "--force-with-lease");
        }

        let output = std::process::Command::new("git")
            .args(&args)
            .current_dir(workdir)
            .output()
            .map_err(|e| Error::PushFailed(e.to_string()))?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(Error::PushFailed(stderr.to_string()))
        }
    }

    /// Fetch a branch from origin.
    ///
    /// # Errors
    /// Returns error if fetch fails.
    pub fn fetch(&self, branch: &str) -> Result<()> {
        let workdir = self.workdir().ok_or(Error::NotARepository)?;

        // Use refspec to update both remote tracking branch and local branch
        // Format: origin/branch:refs/heads/branch
        let refspec = format!("{branch}:refs/heads/{branch}");
        let output = std::process::Command::new("git")
            .args(["fetch", "origin", &refspec])
            .current_dir(workdir)
            .output()
            .map_err(|e| Error::FetchFailed(e.to_string()))?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(Error::FetchFailed(stderr.to_string()))
        }
    }

    /// Pull (fast-forward only) the current branch from origin.
    ///
    /// This fetches and merges `origin/<branch>` into the current branch,
    /// but only if it can be fast-forwarded.
    ///
    /// # Errors
    /// Returns error if pull fails or fast-forward is not possible.
    pub fn pull_ff(&self) -> Result<()> {
        let workdir = self.workdir().ok_or(Error::NotARepository)?;

        let output = std::process::Command::new("git")
            .args(["pull", "--ff-only"])
            .current_dir(workdir)
            .output()
            .map_err(|e| Error::FetchFailed(e.to_string()))?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(Error::FetchFailed(stderr.to_string()))
        }
    }

    // === Low-level access ===

    /// Get a reference to the underlying git2 repository.
    ///
    /// Use sparingly - prefer high-level methods.
    #[must_use]
    pub const fn inner(&self) -> &git2::Repository {
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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn init_test_repo() -> (TempDir, Repository) {
        let temp = TempDir::new().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();

        // Create initial commit with owned signature (avoids borrowing repo)
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
            .unwrap();
        drop(tree);

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

        // Create and commit a tracked file
        fs::write(temp.path().join("test.txt"), "initial").unwrap();
        {
            let mut index = repo.inner.index().unwrap();
            index.add_path(std::path::Path::new("test.txt")).unwrap();
            index.write().unwrap();
            let tree_id = index.write_tree().unwrap();
            let tree = repo.inner.find_tree(tree_id).unwrap();
            let parent = repo.inner.head().unwrap().peel_to_commit().unwrap();
            let sig = git2::Signature::now("Test", "test@example.com").unwrap();
            repo.inner
                .commit(Some("HEAD"), &sig, &sig, "Add test file", &tree, &[&parent])
                .unwrap();
        }

        // Should still be clean after commit
        assert!(repo.is_clean().unwrap());

        // Modify tracked file
        fs::write(temp.path().join("test.txt"), "modified").unwrap();
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
