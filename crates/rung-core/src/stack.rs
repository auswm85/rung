//! Stack data model representing a chain of dependent branches.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::BranchName;

/// A stack of dependent branches forming a PR chain.
// TODO(long-term): For large stacks (>20 branches), consider adding a HashMap<String, usize>
// index for O(1) lookup in find_branch() and find_branch_mut() instead of linear search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stack {
    /// Ordered list of branches from base to tip.
    pub branches: Vec<StackBranch>,

    /// Branches that have been merged (for preserving history in PR comments).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merged: Vec<MergedBranch>,
}

impl Stack {
    /// Create a new empty stack.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            branches: Vec::new(),
            merged: Vec::new(),
        }
    }

    /// Find a branch by name.
    #[must_use]
    pub fn find_branch(&self, name: &str) -> Option<&StackBranch> {
        self.branches.iter().find(|b| b.name == name)
    }

    /// Find a branch by name (mutable).
    pub fn find_branch_mut(&mut self, name: &str) -> Option<&mut StackBranch> {
        self.branches.iter_mut().find(|b| b.name == name)
    }

    /// Add a new branch to the stack.
    pub fn add_branch(&mut self, branch: StackBranch) {
        self.branches.push(branch);
    }

    /// Remove a branch from the stack.
    pub fn remove_branch(&mut self, name: &str) -> Option<StackBranch> {
        if let Some(pos) = self.branches.iter().position(|b| b.name == name) {
            Some(self.branches.remove(pos))
        } else {
            None
        }
    }

    /// Mark a branch as merged, moving it from active to merged list.
    ///
    /// This preserves the branch info for stack comment history,
    /// including the original parent for ancestry chain traversal.
    /// Returns the removed branch if found.
    pub fn mark_merged(&mut self, name: &str) -> Option<StackBranch> {
        let branch = self.remove_branch(name)?;

        if let Some(pr) = branch.pr {
            self.merged.push(MergedBranch {
                name: branch.name.clone(),
                parent: branch.parent.clone(),
                pr,
                merged_at: Utc::now(),
            });
        }

        Some(branch)
    }

    /// Find a merged branch by name.
    #[must_use]
    pub fn find_merged(&self, name: &str) -> Option<&MergedBranch> {
        self.merged.iter().find(|b| b.name == name)
    }

    /// Find a merged branch by PR number.
    #[must_use]
    pub fn find_merged_by_pr(&self, pr: u64) -> Option<&MergedBranch> {
        self.merged.iter().find(|b| b.pr == pr)
    }

    /// Clear merged branches when stack is empty.
    ///
    /// This should be called after merge operations to clean up
    /// when the entire stack has been merged.
    pub fn clear_merged_if_empty(&mut self) {
        if self.branches.is_empty() {
            self.merged.clear();
        }
    }

    /// Get all children of a branch.
    #[must_use]
    pub fn children_of(&self, name: &str) -> Vec<&StackBranch> {
        self.branches
            .iter()
            .filter(|b| b.parent.as_deref() == Some(name))
            .collect()
    }

    /// Get all descendants of a branch in topological order (parents before children).
    ///
    /// This includes children, grandchildren, etc. The branch itself is NOT included.
    #[must_use]
    pub fn descendants(&self, name: &str) -> Vec<&StackBranch> {
        let mut result = Vec::new();
        let mut stack = vec![name];

        while let Some(current_parent) = stack.pop() {
            for branch in &self.branches {
                if branch.parent.as_deref() == Some(current_parent) {
                    result.push(branch);
                    stack.push(&branch.name);
                }
            }
        }
        result
    }

    /// Get the ancestry chain for a branch (from root to the branch).
    #[must_use]
    pub fn ancestry(&self, name: &str) -> Vec<&StackBranch> {
        let mut chain = vec![];
        let mut current = name;

        while let Some(branch) = self.find_branch(current) {
            chain.push(branch);
            match &branch.parent {
                Some(parent) if self.find_branch(parent).is_some() => {
                    current = parent;
                }
                _ => break,
            }
        }

        chain.reverse();
        chain
    }

    /// Check if the stack is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.branches.is_empty()
    }

    /// Get the number of branches in the stack.
    #[must_use]
    pub fn len(&self) -> usize {
        self.branches.len()
    }
}

impl Default for Stack {
    fn default() -> Self {
        Self::new()
    }
}

/// A branch within a stack.
///
/// Branch names are validated at construction time to prevent:
/// - Path traversal attacks (`../`)
/// - Shell metacharacters (`$`, `;`, `|`, etc.)
/// - Invalid git branch name characters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackBranch {
    /// Branch name (validated).
    pub name: BranchName,

    /// Parent branch name (None for root branches based on main/master).
    pub parent: Option<BranchName>,

    /// Associated PR number (if submitted).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr: Option<u64>,

    /// When this branch was added to the stack.
    pub created: DateTime<Utc>,
}

impl StackBranch {
    /// Create a new stack branch with pre-validated names.
    #[must_use]
    pub fn new(name: BranchName, parent: Option<BranchName>) -> Self {
        Self {
            name,
            parent,
            pr: None,
            created: Utc::now(),
        }
    }

    /// Create a new stack branch, validating the names.
    ///
    /// # Errors
    ///
    /// Returns an error if the branch name or parent name is invalid.
    pub fn try_new(
        name: impl Into<String>,
        parent: Option<impl Into<String>>,
    ) -> crate::Result<Self> {
        let name = BranchName::new(name)?;
        let parent = parent.map(BranchName::new).transpose()?;
        Ok(Self::new(name, parent))
    }
}

/// A branch that has been merged (for preserving history in PR comments).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergedBranch {
    /// Branch name.
    pub name: BranchName,

    /// Original parent branch name (preserved for ancestry chain).
    #[serde(default)]
    pub parent: Option<BranchName>,

    /// PR number that was merged.
    pub pr: u64,

    /// When this branch was merged.
    pub merged_at: DateTime<Utc>,
}

/// Synchronization state of a branch relative to its parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BranchState {
    /// Branch is up-to-date with its parent.
    Synced,

    /// Parent has moved forward, branch needs rebase.
    Diverged {
        /// Number of commits the parent is ahead.
        commits_behind: usize,
    },

    /// Rebase resulted in conflicts that need resolution.
    Conflict {
        /// Files with conflicts.
        files: Vec<String>,
    },

    /// Parent branch was deleted or renamed.
    Detached,
}

impl BranchState {
    /// Check if the branch needs syncing.
    #[must_use]
    pub const fn needs_sync(&self) -> bool {
        matches!(self, Self::Diverged { .. })
    }

    /// Check if the branch has conflicts.
    #[must_use]
    pub const fn has_conflicts(&self) -> bool {
        matches!(self, Self::Conflict { .. })
    }

    /// Check if the branch is healthy (synced).
    #[must_use]
    pub const fn is_healthy(&self) -> bool {
        matches!(self, Self::Synced)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_stack_operations() {
        let mut stack = Stack::new();
        assert!(stack.is_empty());

        stack.add_branch(StackBranch::try_new("feature/auth", Some("main")).unwrap());
        stack.add_branch(StackBranch::try_new("feature/auth-ui", Some("feature/auth")).unwrap());

        assert_eq!(stack.len(), 2);
        assert!(stack.find_branch("feature/auth").is_some());

        let children = stack.children_of("feature/auth");
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "feature/auth-ui");
    }

    #[test]
    fn test_ancestry() {
        let mut stack = Stack::new();
        stack.add_branch(StackBranch::try_new("a", Some("main")).unwrap());
        stack.add_branch(StackBranch::try_new("b", Some("a")).unwrap());
        stack.add_branch(StackBranch::try_new("c", Some("b")).unwrap());

        let ancestry = stack.ancestry("c");
        assert_eq!(ancestry.len(), 3);
        assert_eq!(ancestry[0].name, "a");
        assert_eq!(ancestry[1].name, "b");
        assert_eq!(ancestry[2].name, "c");
    }

    #[test]
    fn test_descendants() {
        let mut stack = Stack::new();
        // Create tree: main → a → b → c
        //                    ↘ d
        stack.add_branch(StackBranch::try_new("a", Some("main")).unwrap());
        stack.add_branch(StackBranch::try_new("b", Some("a")).unwrap());
        stack.add_branch(StackBranch::try_new("c", Some("b")).unwrap());
        stack.add_branch(StackBranch::try_new("d", Some("a")).unwrap());

        // Descendants of "a" should be b, c, d (in some order based on traversal)
        let descendants = stack.descendants("a");
        assert_eq!(descendants.len(), 3);
        let names: Vec<&str> = descendants.iter().map(|b| b.name.as_str()).collect();
        assert!(names.contains(&"b"));
        assert!(names.contains(&"c"));
        assert!(names.contains(&"d"));

        // Descendants of "b" should only be c
        let descendants = stack.descendants("b");
        assert_eq!(descendants.len(), 1);
        assert_eq!(descendants[0].name, "c");

        // Descendants of "c" (leaf) should be empty
        let descendants = stack.descendants("c");
        assert!(descendants.is_empty());
    }

    #[test]
    fn test_branch_state() {
        assert!(BranchState::Synced.is_healthy());
        assert!(!BranchState::Synced.needs_sync());

        let diverged = BranchState::Diverged { commits_behind: 3 };
        assert!(diverged.needs_sync());
        assert!(!diverged.is_healthy());

        let conflict = BranchState::Conflict {
            files: vec!["src/main.rs".into()],
        };
        assert!(conflict.has_conflicts());
    }
}
