//! Submit service for pushing branches and creating/updating PRs.
//!
//! This service encapsulates the business logic for the submit command,
//! accepting trait-based dependencies for testability.

use std::collections::HashSet;
use std::fmt::Write;

use anyhow::{Context, Result};
use rung_core::stack::Stack;
use rung_git::GitOps;
use rung_github::{CreateComment, CreatePullRequest, GitHubApi, UpdateComment, UpdatePullRequest};
use serde::Serialize;

/// A planned action for a single branch.
#[derive(Debug, Clone)]
pub enum PlannedBranchAction {
    /// Update an existing PR (push branch, update base).
    Update {
        branch: String,
        pr_number: u64,
        pr_url: String,
        base: String,
    },
    /// Create a new PR.
    Create {
        branch: String,
        title: String,
        body: String,
        base: String,
        draft: bool,
    },
}

/// The complete submit plan describing what will happen.
#[derive(Debug, Clone)]
pub struct SubmitPlan {
    pub actions: Vec<PlannedBranchAction>,
}

impl SubmitPlan {
    /// Create an empty plan.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            actions: Vec::new(),
        }
    }

    /// Count the number of PR creates in this plan.
    #[must_use]
    pub fn count_creates(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| matches!(a, PlannedBranchAction::Create { .. }))
            .count()
    }

    /// Count the number of PR updates in this plan.
    #[must_use]
    pub fn count_updates(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| matches!(a, PlannedBranchAction::Update { .. }))
            .count()
    }

    /// Check if this plan is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

/// Result of executing a submit action for a branch.
#[derive(Debug, Clone, Serialize)]
pub struct BranchSubmitResult {
    pub branch: String,
    pub pr_number: u64,
    pub pr_url: String,
    pub action: SubmitAction,
}

/// The type of action taken for a branch.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmitAction {
    Created,
    Updated,
}

/// Configuration for creating a submit plan.
pub struct SubmitConfig<'a> {
    /// Create PRs as drafts.
    pub draft: bool,
    /// Custom title for the current branch's PR.
    pub custom_title: Option<&'a str>,
    /// Current branch name (for custom title matching).
    pub current_branch: Option<String>,
    /// Default base branch (from config, falls back to "main").
    pub default_branch: String,
}

/// Service for submit operations with injected dependencies.
///
/// This service encapsulates the business logic for:
/// - Creating a submit plan (determining what PRs to create/update)
/// - Executing the plan (pushing branches, creating/updating PRs)
/// - Updating stack navigation comments on PRs
pub struct SubmitService<'a, G, H>
where
    G: GitOps,
    H: GitHubApi,
{
    git: &'a G,
    github: &'a H,
    owner: String,
    repo: String,
}

#[allow(clippy::future_not_send)] // Git operations are sync; futures don't need to be Send
impl<'a, G, H> SubmitService<'a, G, H>
where
    G: GitOps,
    H: GitHubApi,
{
    /// Create a new submit service.
    pub const fn new(git: &'a G, github: &'a H, owner: String, repo: String) -> Self {
        Self {
            git,
            github,
            owner,
            repo,
        }
    }

    /// Create a submit plan by analyzing the stack and checking existing PRs.
    ///
    /// This is a read-only operation that determines what actions would be taken.
    ///
    /// # Errors
    /// Returns error if GitHub API calls fail.
    pub async fn create_plan(
        &self,
        stack: &Stack,
        config: &SubmitConfig<'_>,
    ) -> Result<SubmitPlan> {
        let mut actions = Vec::new();

        for branch in &stack.branches {
            let branch_name = &branch.name;
            let base_branch = branch
                .parent
                .as_deref()
                .unwrap_or(&config.default_branch)
                .to_string();

            // Get title and body from commit message
            let (mut title, body) = self.get_pr_title_and_body(branch_name);
            if config.current_branch.as_deref() == Some(branch_name.as_str())
                && let Some(custom) = config.custom_title
            {
                title = custom.to_string();
            }

            // Check if PR already exists
            if let Some(pr_number) = branch.pr {
                let pr_url = format!(
                    "https://github.com/{}/{}/pull/{pr_number}",
                    self.owner, self.repo
                );
                actions.push(PlannedBranchAction::Update {
                    branch: branch_name.to_string(),
                    pr_number,
                    pr_url,
                    base: base_branch,
                });
            } else {
                let existing = self
                    .github
                    .find_pr_for_branch(&self.owner, &self.repo, branch_name)
                    .await
                    .context("Failed to check for existing PR")?;

                if let Some(pr) = existing {
                    actions.push(PlannedBranchAction::Update {
                        branch: branch_name.to_string(),
                        pr_number: pr.number,
                        pr_url: pr.html_url,
                        base: base_branch,
                    });
                } else {
                    actions.push(PlannedBranchAction::Create {
                        branch: branch_name.to_string(),
                        title,
                        body,
                        base: base_branch,
                        draft: config.draft,
                    });
                }
            }
        }

        Ok(SubmitPlan { actions })
    }

    /// Execute a submit plan, pushing branches and creating/updating PRs.
    ///
    /// Returns information about each submitted branch.
    ///
    /// # Errors
    /// Returns error if git or GitHub operations fail.
    pub async fn execute(
        &self,
        stack: &mut Stack,
        plan: &SubmitPlan,
        force: bool,
    ) -> Result<Vec<BranchSubmitResult>> {
        let mut results = Vec::new();

        for action in &plan.actions {
            match action {
                PlannedBranchAction::Update {
                    branch,
                    pr_number,
                    pr_url,
                    base,
                } => {
                    // Push branch
                    self.git
                        .push(branch, force)
                        .with_context(|| format!("Failed to push {branch}"))?;

                    // Update PR base
                    let update = UpdatePullRequest {
                        title: None,
                        body: None,
                        base: Some(base.clone()),
                    };
                    self.github
                        .update_pr(&self.owner, &self.repo, *pr_number, update)
                        .await
                        .with_context(|| format!("Failed to update PR #{pr_number}"))?;

                    // Persist PR number if discovered during planning
                    if let Some(stack_branch) =
                        stack.branches.iter_mut().find(|b| &b.name == branch)
                        && stack_branch.pr.is_none()
                    {
                        stack_branch.pr = Some(*pr_number);
                    }

                    results.push(BranchSubmitResult {
                        branch: branch.clone(),
                        pr_number: *pr_number,
                        pr_url: pr_url.clone(),
                        action: SubmitAction::Updated,
                    });
                }
                PlannedBranchAction::Create {
                    branch,
                    title,
                    body,
                    base,
                    draft,
                } => {
                    // Push branch
                    self.git
                        .push(branch, force)
                        .with_context(|| format!("Failed to push {branch}"))?;

                    // Check if PR was created between planning and execution
                    let existing = self
                        .github
                        .find_pr_for_branch(&self.owner, &self.repo, branch)
                        .await
                        .context("Failed to check for existing PR")?;

                    let (pr_number, pr_url, was_created) = if let Some(pr) = existing {
                        // Update existing PR
                        let update = UpdatePullRequest {
                            title: None,
                            body: None,
                            base: Some(base.clone()),
                        };
                        self.github
                            .update_pr(&self.owner, &self.repo, pr.number, update)
                            .await
                            .with_context(|| format!("Failed to update PR #{}", pr.number))?;

                        (pr.number, pr.html_url, false)
                    } else {
                        // Create new PR
                        let create = CreatePullRequest {
                            title: title.clone(),
                            body: body.clone(),
                            head: branch.clone(),
                            base: base.clone(),
                            draft: *draft,
                        };
                        let pr = self
                            .github
                            .create_pr(&self.owner, &self.repo, create)
                            .await
                            .with_context(|| format!("Failed to create PR for {branch}"))?;

                        (pr.number, pr.html_url, true)
                    };

                    // Update stack state
                    if let Some(stack_branch) =
                        stack.branches.iter_mut().find(|b| &b.name == branch)
                    {
                        stack_branch.pr = Some(pr_number);
                    }

                    results.push(BranchSubmitResult {
                        branch: branch.clone(),
                        pr_number,
                        pr_url,
                        action: if was_created {
                            SubmitAction::Created
                        } else {
                            SubmitAction::Updated
                        },
                    });
                }
            }
        }

        Ok(results)
    }

    /// Update stack navigation comments on all PRs.
    ///
    /// # Errors
    /// Returns error if GitHub API calls fail.
    pub async fn update_stack_comments(&self, stack: &Stack, default_branch: &str) -> Result<()> {
        for branch in &stack.branches {
            let Some(pr_number) = branch.pr else {
                continue;
            };

            let comment_body = generate_stack_comment(stack, pr_number, default_branch);

            // Find existing rung comment
            let comments = self
                .github
                .list_pr_comments(&self.owner, &self.repo, pr_number)
                .await
                .with_context(|| format!("Failed to list comments on PR #{pr_number}"))?;

            let existing_comment = comments.iter().find(|c| {
                c.body
                    .as_ref()
                    .is_some_and(|b| b.contains(STACK_COMMENT_MARKER))
            });

            if let Some(comment) = existing_comment {
                let update = UpdateComment { body: comment_body };
                self.github
                    .update_pr_comment(&self.owner, &self.repo, comment.id, update)
                    .await
                    .with_context(|| format!("Failed to update comment on PR #{pr_number}"))?;
            } else {
                let create = CreateComment { body: comment_body };
                self.github
                    .create_pr_comment(&self.owner, &self.repo, pr_number, create)
                    .await
                    .with_context(|| format!("Failed to create comment on PR #{pr_number}"))?;
            }
        }

        Ok(())
    }

    /// Get PR title and body from the branch's tip commit message.
    fn get_pr_title_and_body(&self, branch_name: &str) -> (String, String) {
        if let Ok(message) = self.git.branch_commit_message(branch_name) {
            let mut lines = message.lines();
            let title = lines.next().unwrap_or("").trim().to_string();

            let body: String = lines
                .skip_while(|line| line.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n")
                .trim_end()
                .to_string();

            if !title.is_empty() {
                return (title, body);
            }
        }

        (generate_title(branch_name), String::new())
    }
}

// === Helper Functions ===

/// Marker to identify rung stack comments.
const STACK_COMMENT_MARKER: &str = "<!-- rung-stack -->";

/// Generate PR title from branch name.
fn generate_title(branch_name: &str) -> String {
    let base = branch_name
        .split('/')
        .next_back()
        .unwrap_or(branch_name)
        .replace(['-', '_'], " ");

    base.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |c| {
                c.to_uppercase().collect::<String>() + chars.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Generate stack comment for a PR.
fn generate_stack_comment(stack: &Stack, current_pr: u64, default_branch: &str) -> String {
    let mut comment = String::from(STACK_COMMENT_MARKER);
    comment.push('\n');

    let branches = &stack.branches;
    let current_branch = branches.iter().find(|b| b.pr == Some(current_pr));
    let current_name = current_branch.map_or("", |b| b.name.as_str());

    let chain = build_branch_chain(stack, current_name);

    for branch_name in chain.iter().rev() {
        let is_current = branch_name == current_name;
        let pointer = if is_current { " 👈" } else { "" };

        if let Some(merged) = stack.find_merged(branch_name) {
            let _ = writeln!(comment, "* ~~**#{}**~~ ✓{pointer}", merged.pr);
        } else if let Some(b) = branches.iter().find(|b| &b.name == branch_name) {
            if let Some(pr_num) = b.pr {
                let _ = writeln!(comment, "* **#{pr_num}**{pointer}");
            } else {
                let _ = writeln!(comment, "* *(pending)* `{branch_name}`{pointer}");
            }
        }
    }

    let base = current_branch
        .and_then(|b| {
            let mut current = b;
            let mut visited = HashSet::new();
            loop {
                // Cycle detection: if we've seen this branch, stop
                if !visited.insert(current.name.as_str()) {
                    return Some(default_branch);
                }
                if let Some(ref parent) = current.parent {
                    if let Some(p) = branches.iter().find(|br| &br.name == parent) {
                        current = p;
                    } else {
                        // Parent not in active branches (may be merged or base branch)
                        return Some(parent.as_str());
                    }
                } else {
                    return Some(default_branch);
                }
            }
        })
        .unwrap_or(default_branch);

    let _ = writeln!(comment, "* `{base}`");
    comment.push_str("\n---\n*Managed by [rung](https://github.com/auswm85/rung)*");

    comment
}

/// Build a chain of branches from root ancestor to all descendants.
fn build_branch_chain(stack: &Stack, current_name: &str) -> Vec<String> {
    let branches = &stack.branches;
    let mut ancestors: Vec<String> = vec![];
    let mut current = current_name.to_string();
    let mut visited = HashSet::new();

    loop {
        // Cycle detection: if we've seen this branch, stop
        if !visited.insert(current.clone()) {
            break;
        }

        let parent = branches
            .iter()
            .find(|b| b.name == current)
            .and_then(|b| b.parent.as_ref())
            .or_else(|| stack.find_merged(&current).and_then(|m| m.parent.as_ref()))
            .map(ToString::to_string);

        let Some(parent_name) = parent else {
            break;
        };

        let in_active = branches.iter().any(|b| b.name == parent_name);
        let in_merged = stack.find_merged(&parent_name).is_some();

        if in_active || in_merged {
            ancestors.push(parent_name.clone());
            current = parent_name;
        } else {
            break;
        }
    }

    ancestors.reverse();

    let mut chain = ancestors;
    chain.push(current_name.to_string());

    let mut i = 0;
    while i < chain.len() {
        let parent_name = chain[i].clone();
        for branch in branches {
            if branch.parent.as_ref().is_some_and(|p| p == &parent_name)
                && !chain.contains(&branch.name.to_string())
            {
                chain.push(branch.name.to_string());
            }
        }
        i += 1;
    }

    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_title() {
        assert_eq!(generate_title("feature/add-auth"), "Add Auth");
        assert_eq!(generate_title("fix_bug_123"), "Fix Bug 123");
        assert_eq!(generate_title("simple"), "Simple");
    }

    #[test]
    fn test_generate_title_edge_cases() {
        // Nested paths
        assert_eq!(generate_title("user/feature/add-auth"), "Add Auth");
        // All underscores
        assert_eq!(generate_title("add_user_auth"), "Add User Auth");
        // Mixed separators
        assert_eq!(generate_title("fix-bug_report"), "Fix Bug Report");
        // Empty last segment
        assert_eq!(generate_title(""), "");
    }

    #[test]
    fn test_submit_plan_counts() {
        let plan = SubmitPlan {
            actions: vec![
                PlannedBranchAction::Create {
                    branch: "a".into(),
                    title: "A".into(),
                    body: String::new(),
                    base: "main".into(),
                    draft: false,
                },
                PlannedBranchAction::Update {
                    branch: "b".into(),
                    pr_number: 1,
                    pr_url: "url".into(),
                    base: "main".into(),
                },
                PlannedBranchAction::Create {
                    branch: "c".into(),
                    title: "C".into(),
                    body: String::new(),
                    base: "a".into(),
                    draft: true,
                },
            ],
        };

        assert_eq!(plan.count_creates(), 2);
        assert_eq!(plan.count_updates(), 1);
        assert!(!plan.is_empty());
    }

    #[test]
    fn test_empty_plan() {
        let plan = SubmitPlan { actions: vec![] };
        assert!(plan.is_empty());
        assert_eq!(plan.count_creates(), 0);
        assert_eq!(plan.count_updates(), 0);
    }

    #[test]
    fn test_submit_plan_empty_const() {
        let plan = SubmitPlan::empty();
        assert!(plan.is_empty());
        assert_eq!(plan.actions.len(), 0);
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_branch_submit_result_serializes() {
        let result = BranchSubmitResult {
            branch: "feature/auth".to_string(),
            pr_number: 42,
            pr_url: "https://github.com/owner/repo/pull/42".to_string(),
            action: SubmitAction::Created,
        };
        let json = serde_json::to_string(&result).expect("serialization should succeed");
        assert!(json.contains("feature/auth"));
        assert!(json.contains("42"));
        assert!(json.contains("created"));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_submit_action_serializes() {
        let created = SubmitAction::Created;
        let json = serde_json::to_string(&created).expect("serialization should succeed");
        assert!(json.contains("created"));

        let updated = SubmitAction::Updated;
        let json = serde_json::to_string(&updated).expect("serialization should succeed");
        assert!(json.contains("updated"));
    }

    #[test]
    fn test_planned_branch_action_create() {
        let action = PlannedBranchAction::Create {
            branch: "feature/test".into(),
            title: "Test Feature".into(),
            body: "Description".into(),
            base: "main".into(),
            draft: true,
        };
        assert!(matches!(
            action,
            PlannedBranchAction::Create { draft: true, .. }
        ));
    }

    #[test]
    fn test_planned_branch_action_update() {
        let action = PlannedBranchAction::Update {
            branch: "feature/test".into(),
            pr_number: 123,
            pr_url: "https://github.com/owner/repo/pull/123".into(),
            base: "main".into(),
        };
        assert!(matches!(
            action,
            PlannedBranchAction::Update { pr_number: 123, .. }
        ));
    }

    #[test]
    fn test_generate_title_various_formats() {
        // Simple hyphenated
        assert_eq!(generate_title("add-feature"), "Add Feature");
        // CamelCase-like with hyphens
        assert_eq!(generate_title("AddNew-feature"), "AddNew Feature");
        // Numbers in name
        assert_eq!(generate_title("fix-issue-42"), "Fix Issue 42");
        // Single word
        assert_eq!(generate_title("hotfix"), "Hotfix");
        // Path with multiple segments
        assert_eq!(generate_title("user/john/feature/auth"), "Auth");
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_build_branch_chain_single_branch() {
        use rung_core::{BranchName, Stack, stack::StackBranch};

        let mut stack = Stack::default();
        let name = BranchName::new("feature-1").expect("valid branch name");
        let parent = BranchName::new("main").expect("valid branch name");
        stack.add_branch(StackBranch::new(name, Some(parent)));

        let chain = build_branch_chain(&stack, "feature-1");
        assert_eq!(chain, vec!["feature-1"]);
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_build_branch_chain_with_parent() {
        use rung_core::{BranchName, Stack, stack::StackBranch};

        let mut stack = Stack::default();
        let f1 = BranchName::new("feature-1").expect("valid");
        let main = BranchName::new("main").expect("valid");
        stack.add_branch(StackBranch::new(f1.clone(), Some(main)));

        let f2 = BranchName::new("feature-2").expect("valid");
        stack.add_branch(StackBranch::new(f2, Some(f1)));

        let chain = build_branch_chain(&stack, "feature-2");
        assert!(chain.contains(&"feature-1".to_string()));
        assert!(chain.contains(&"feature-2".to_string()));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_build_branch_chain_complex() {
        use rung_core::{BranchName, Stack, stack::StackBranch};

        let mut stack = Stack::default();
        let a = BranchName::new("a").expect("valid");
        let b = BranchName::new("b").expect("valid");
        let c = BranchName::new("c").expect("valid");
        let main = BranchName::new("main").expect("valid");

        stack.add_branch(StackBranch::new(a.clone(), Some(main)));
        stack.add_branch(StackBranch::new(b.clone(), Some(a)));
        stack.add_branch(StackBranch::new(c, Some(b)));

        let chain = build_branch_chain(&stack, "c");
        // Should contain all branches in the chain
        assert!(chain.contains(&"a".to_string()));
        assert!(chain.contains(&"b".to_string()));
        assert!(chain.contains(&"c".to_string()));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_generate_stack_comment_single_branch() {
        use rung_core::{BranchName, Stack, stack::StackBranch};

        let mut stack = Stack::default();
        let name = BranchName::new("feature-1").expect("valid");
        let parent = BranchName::new("main").expect("valid");
        stack.add_branch(StackBranch::new(name, Some(parent)));

        if let Some(b) = stack
            .branches
            .iter_mut()
            .find(|b| b.name.as_str() == "feature-1")
        {
            b.pr = Some(42);
        }

        let comment = generate_stack_comment(&stack, 42, "main");
        assert!(comment.contains(STACK_COMMENT_MARKER));
        assert!(comment.contains("#42"));
        assert!(comment.contains("main"));
        assert!(comment.contains("rung"));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_generate_stack_comment_with_chain() {
        use rung_core::{BranchName, Stack, stack::StackBranch};

        let mut stack = Stack::default();
        let f1 = BranchName::new("feature-1").expect("valid");
        let f2 = BranchName::new("feature-2").expect("valid");
        let main = BranchName::new("main").expect("valid");

        stack.add_branch(StackBranch::new(f1.clone(), Some(main)));
        stack.add_branch(StackBranch::new(f2, Some(f1)));

        if let Some(b) = stack
            .branches
            .iter_mut()
            .find(|b| b.name.as_str() == "feature-1")
        {
            b.pr = Some(10);
        }
        if let Some(b) = stack
            .branches
            .iter_mut()
            .find(|b| b.name.as_str() == "feature-2")
        {
            b.pr = Some(20);
        }

        let comment = generate_stack_comment(&stack, 20, "main");
        assert!(comment.contains("#10"));
        assert!(comment.contains("#20"));
        assert!(comment.contains("👈")); // Current PR marker
    }

    #[test]
    fn test_submit_plan_all_updates() {
        let plan = SubmitPlan {
            actions: vec![
                PlannedBranchAction::Update {
                    branch: "a".into(),
                    pr_number: 1,
                    pr_url: "url1".into(),
                    base: "main".into(),
                },
                PlannedBranchAction::Update {
                    branch: "b".into(),
                    pr_number: 2,
                    pr_url: "url2".into(),
                    base: "a".into(),
                },
            ],
        };

        assert_eq!(plan.count_creates(), 0);
        assert_eq!(plan.count_updates(), 2);
        assert!(!plan.is_empty());
    }

    #[test]
    fn test_submit_plan_all_creates() {
        let plan = SubmitPlan {
            actions: vec![
                PlannedBranchAction::Create {
                    branch: "a".into(),
                    title: "A".into(),
                    body: String::new(),
                    base: "main".into(),
                    draft: false,
                },
                PlannedBranchAction::Create {
                    branch: "b".into(),
                    title: "B".into(),
                    body: String::new(),
                    base: "a".into(),
                    draft: false,
                },
            ],
        };

        assert_eq!(plan.count_creates(), 2);
        assert_eq!(plan.count_updates(), 0);
    }

    #[test]
    fn test_stack_comment_marker_constant() {
        assert!(STACK_COMMENT_MARKER.starts_with("<!--"));
        assert!(STACK_COMMENT_MARKER.ends_with("-->"));
        assert!(STACK_COMMENT_MARKER.contains("rung"));
    }

    // Tests using mock implementations
    #[allow(clippy::manual_async_fn, clippy::unwrap_used)]
    mod mock_tests {
        use super::*;
        use crate::services::test_mocks::MockGitOps;
        use rung_core::stack::{Stack, StackBranch};
        use rung_git::Oid;

        // Mock GitHubApi for submit testing
        struct MockGitHubClient {
            find_pr_result: Option<rung_github::PullRequest>,
        }

        impl MockGitHubClient {
            fn new() -> Self {
                Self {
                    find_pr_result: None,
                }
            }

            #[allow(dead_code)]
            fn with_existing_pr(mut self, pr: rung_github::PullRequest) -> Self {
                self.find_pr_result = Some(pr);
                self
            }
        }

        impl rung_github::GitHubApi for MockGitHubClient {
            fn get_pr(
                &self,
                _owner: &str,
                _repo: &str,
                number: u64,
            ) -> impl std::future::Future<Output = rung_github::Result<rung_github::PullRequest>> + Send
            {
                async move { Err(rung_github::Error::PrNotFound(number)) }
            }

            fn get_prs_batch(
                &self,
                _owner: &str,
                _repo: &str,
                _numbers: &[u64],
            ) -> impl std::future::Future<
                Output = rung_github::Result<
                    std::collections::HashMap<u64, rung_github::PullRequest>,
                >,
            > + Send {
                async { Ok(std::collections::HashMap::new()) }
            }

            fn find_pr_for_branch(
                &self,
                _owner: &str,
                _repo: &str,
                _branch: &str,
            ) -> impl std::future::Future<
                Output = rung_github::Result<Option<rung_github::PullRequest>>,
            > + Send {
                let result = self.find_pr_result.clone();
                async move { Ok(result) }
            }

            fn create_pr(
                &self,
                _owner: &str,
                _repo: &str,
                params: rung_github::CreatePullRequest,
            ) -> impl std::future::Future<Output = rung_github::Result<rung_github::PullRequest>> + Send
            {
                async move {
                    Ok(rung_github::PullRequest {
                        number: 100,
                        title: params.title,
                        body: Some(params.body),
                        state: rung_github::PullRequestState::Open,
                        base_branch: params.base,
                        head_branch: params.head,
                        html_url: "https://github.com/test/repo/pull/100".to_string(),
                        mergeable: None,
                        mergeable_state: None,
                        draft: params.draft,
                    })
                }
            }

            fn update_pr(
                &self,
                _owner: &str,
                _repo: &str,
                number: u64,
                _params: rung_github::UpdatePullRequest,
            ) -> impl std::future::Future<Output = rung_github::Result<rung_github::PullRequest>> + Send
            {
                async move {
                    Ok(rung_github::PullRequest {
                        number,
                        title: "Updated".to_string(),
                        body: None,
                        state: rung_github::PullRequestState::Open,
                        base_branch: "main".to_string(),
                        head_branch: "feature".to_string(),
                        html_url: format!("https://github.com/test/repo/pull/{number}"),
                        mergeable: None,
                        mergeable_state: None,
                        draft: false,
                    })
                }
            }

            fn get_check_runs(
                &self,
                _owner: &str,
                _repo: &str,
                _commit_sha: &str,
            ) -> impl std::future::Future<Output = rung_github::Result<Vec<rung_github::CheckRun>>> + Send
            {
                async { Ok(vec![]) }
            }

            fn merge_pr(
                &self,
                _owner: &str,
                _repo: &str,
                _number: u64,
                _params: rung_github::MergePullRequest,
            ) -> impl std::future::Future<Output = rung_github::Result<rung_github::MergeResult>> + Send
            {
                async {
                    Ok(rung_github::MergeResult {
                        sha: "abc123".to_string(),
                        merged: true,
                        message: "Merged".to_string(),
                    })
                }
            }

            fn delete_ref(
                &self,
                _owner: &str,
                _repo: &str,
                _ref_name: &str,
            ) -> impl std::future::Future<Output = rung_github::Result<()>> + Send {
                async { Ok(()) }
            }

            fn get_default_branch(
                &self,
                _owner: &str,
                _repo: &str,
            ) -> impl std::future::Future<Output = rung_github::Result<String>> + Send {
                async { Ok("main".to_string()) }
            }

            fn list_pr_comments(
                &self,
                _owner: &str,
                _repo: &str,
                _pr_number: u64,
            ) -> impl std::future::Future<
                Output = rung_github::Result<Vec<rung_github::IssueComment>>,
            > + Send {
                async { Ok(vec![]) }
            }

            fn create_pr_comment(
                &self,
                _owner: &str,
                _repo: &str,
                _pr_number: u64,
                _comment: rung_github::CreateComment,
            ) -> impl std::future::Future<Output = rung_github::Result<rung_github::IssueComment>> + Send
            {
                async {
                    Ok(rung_github::IssueComment {
                        id: 1,
                        body: Some(String::new()),
                    })
                }
            }

            fn update_pr_comment(
                &self,
                _owner: &str,
                _repo: &str,
                _comment_id: u64,
                _comment: rung_github::UpdateComment,
            ) -> impl std::future::Future<Output = rung_github::Result<rung_github::IssueComment>> + Send
            {
                async {
                    Ok(rung_github::IssueComment {
                        id: 1,
                        body: Some(String::new()),
                    })
                }
            }
        }

        #[tokio::test]
        async fn test_create_plan_empty_stack() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("main", oid);
            let github = MockGitHubClient::new();

            let service =
                SubmitService::new(&git, &github, "owner".to_string(), "repo".to_string());
            let stack = Stack::default();
            let config = SubmitConfig {
                draft: false,
                custom_title: None,
                current_branch: None,
                default_branch: "main".to_string(),
            };

            let plan = service.create_plan(&stack, &config).await.unwrap();
            assert!(plan.is_empty());
        }

        #[tokio::test]
        async fn test_create_plan_creates_new_prs() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("main", oid)
                .with_branch("feature/a", oid);
            let github = MockGitHubClient::new();

            let service =
                SubmitService::new(&git, &github, "owner".to_string(), "repo".to_string());

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/a", None::<&str>).unwrap());

            let config = SubmitConfig {
                draft: false,
                custom_title: None,
                current_branch: None,
                default_branch: "main".to_string(),
            };

            let plan = service.create_plan(&stack, &config).await.unwrap();
            assert_eq!(plan.count_creates(), 1);
            assert_eq!(plan.count_updates(), 0);
        }

        #[tokio::test]
        async fn test_create_plan_updates_existing_prs() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("main", oid)
                .with_branch("feature/a", oid);
            let github = MockGitHubClient::new();

            let service =
                SubmitService::new(&git, &github, "owner".to_string(), "repo".to_string());

            let mut stack = Stack::default();
            let mut branch = StackBranch::try_new("feature/a", None::<&str>).unwrap();
            branch.pr = Some(42);
            stack.add_branch(branch);

            let config = SubmitConfig {
                draft: false,
                custom_title: None,
                current_branch: None,
                default_branch: "main".to_string(),
            };

            let plan = service.create_plan(&stack, &config).await.unwrap();
            assert_eq!(plan.count_creates(), 0);
            assert_eq!(plan.count_updates(), 1);
        }

        #[tokio::test]
        async fn test_create_plan_with_draft() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("main", oid)
                .with_branch("feature/a", oid);
            let github = MockGitHubClient::new();

            let service =
                SubmitService::new(&git, &github, "owner".to_string(), "repo".to_string());

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/a", None::<&str>).unwrap());

            let config = SubmitConfig {
                draft: true,
                custom_title: None,
                current_branch: None,
                default_branch: "main".to_string(),
            };

            let plan = service.create_plan(&stack, &config).await.unwrap();
            assert_eq!(plan.count_creates(), 1);

            if let PlannedBranchAction::Create { draft, .. } = &plan.actions[0] {
                assert!(draft);
            } else {
                panic!("Expected Create action");
            }
        }

        #[tokio::test]
        async fn test_get_pr_title_and_body() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("feature/test", oid);
            let github = MockGitHubClient::new();

            let service =
                SubmitService::new(&git, &github, "owner".to_string(), "repo".to_string());

            // MockGitOps returns "Test commit message" for branch_commit_message
            let (title, body) = service.get_pr_title_and_body("feature/test");
            assert_eq!(title, "Test commit message");
            assert!(body.is_empty());
        }

        #[test]
        fn test_submit_service_creation() {
            let git = MockGitOps::new();
            let github = MockGitHubClient::new();

            let _service =
                SubmitService::new(&git, &github, "owner".to_string(), "repo".to_string());
            // Service is created successfully
        }

        #[tokio::test]
        async fn test_execute_create_pr() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("main", oid)
                .with_branch("feature/a", oid)
                .with_push_result("feature/a", true);
            let github = MockGitHubClient::new();

            let service =
                SubmitService::new(&git, &github, "owner".to_string(), "repo".to_string());

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/a", None::<&str>).unwrap());

            let plan = SubmitPlan {
                actions: vec![PlannedBranchAction::Create {
                    branch: "feature/a".to_string(),
                    title: "Feature A".to_string(),
                    body: "Description".to_string(),
                    base: "main".to_string(),
                    draft: false,
                }],
            };

            let results = service.execute(&mut stack, &plan, false).await.unwrap();

            assert_eq!(results.len(), 1);
            assert_eq!(results[0].branch, "feature/a");
            assert_eq!(results[0].pr_number, 100); // MockGitHubClient returns 100
            assert!(matches!(results[0].action, SubmitAction::Created));

            // Check that PR number was persisted to stack
            assert_eq!(stack.branches[0].pr, Some(100));
        }

        #[tokio::test]
        async fn test_execute_update_pr() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("main", oid)
                .with_branch("feature/a", oid)
                .with_push_result("feature/a", true);
            let github = MockGitHubClient::new();

            let service =
                SubmitService::new(&git, &github, "owner".to_string(), "repo".to_string());

            let mut stack = Stack::default();
            let mut branch = StackBranch::try_new("feature/a", None::<&str>).unwrap();
            branch.pr = Some(42);
            stack.add_branch(branch);

            let plan = SubmitPlan {
                actions: vec![PlannedBranchAction::Update {
                    branch: "feature/a".to_string(),
                    pr_number: 42,
                    pr_url: "https://github.com/owner/repo/pull/42".to_string(),
                    base: "main".to_string(),
                }],
            };

            let results = service.execute(&mut stack, &plan, false).await.unwrap();

            assert_eq!(results.len(), 1);
            assert_eq!(results[0].branch, "feature/a");
            assert_eq!(results[0].pr_number, 42);
            assert!(matches!(results[0].action, SubmitAction::Updated));
        }

        #[tokio::test]
        async fn test_execute_multiple_actions() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("main", oid)
                .with_branch("feature/a", oid)
                .with_branch("feature/b", oid)
                .with_push_result("feature/a", true)
                .with_push_result("feature/b", true);
            let github = MockGitHubClient::new();

            let service =
                SubmitService::new(&git, &github, "owner".to_string(), "repo".to_string());

            let mut stack = Stack::default();
            let mut branch_a = StackBranch::try_new("feature/a", None::<&str>).unwrap();
            branch_a.pr = Some(10);
            stack.add_branch(branch_a);
            stack.add_branch(StackBranch::try_new("feature/b", Some("feature/a")).unwrap());

            let plan = SubmitPlan {
                actions: vec![
                    PlannedBranchAction::Update {
                        branch: "feature/a".to_string(),
                        pr_number: 10,
                        pr_url: "https://github.com/owner/repo/pull/10".to_string(),
                        base: "main".to_string(),
                    },
                    PlannedBranchAction::Create {
                        branch: "feature/b".to_string(),
                        title: "Feature B".to_string(),
                        body: "Description".to_string(),
                        base: "feature/a".to_string(),
                        draft: true,
                    },
                ],
            };

            let results = service.execute(&mut stack, &plan, false).await.unwrap();

            assert_eq!(results.len(), 2);
            assert!(matches!(results[0].action, SubmitAction::Updated));
            assert!(matches!(results[1].action, SubmitAction::Created));

            // Check PR number persisted for newly created PR
            assert_eq!(stack.branches[1].pr, Some(100));
        }

        #[tokio::test]
        async fn test_execute_with_force_push() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("main", oid)
                .with_branch("feature/a", oid)
                .with_push_result("feature/a", true);
            let github = MockGitHubClient::new();

            let service =
                SubmitService::new(&git, &github, "owner".to_string(), "repo".to_string());

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/a", None::<&str>).unwrap());

            let plan = SubmitPlan {
                actions: vec![PlannedBranchAction::Create {
                    branch: "feature/a".to_string(),
                    title: "Feature A".to_string(),
                    body: String::new(),
                    base: "main".to_string(),
                    draft: false,
                }],
            };

            // Execute with force=true
            let results = service.execute(&mut stack, &plan, true).await.unwrap();
            assert_eq!(results.len(), 1);
        }
    }
}
