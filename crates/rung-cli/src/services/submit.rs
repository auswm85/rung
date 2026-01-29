//! Submit service for pushing branches and creating/updating PRs.
//!
//! This service encapsulates the business logic for the submit command,
//! accepting trait-based dependencies for testability.

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
        Self { actions: vec![] }
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
    pub fn is_empty(&self) -> bool {
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
            if config.current_branch.as_deref() == Some(branch_name.as_str()) {
                if let Some(custom) = config.custom_title {
                    title = custom.to_string();
                }
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
                    {
                        if stack_branch.pr.is_none() {
                            stack_branch.pr = Some(*pr_number);
                        }
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
            loop {
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

    loop {
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
}
