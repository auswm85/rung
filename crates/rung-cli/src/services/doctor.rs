//! Doctor service for diagnosing stack and repository issues.
//!
//! This module contains the diagnostic logic separated from CLI concerns,
//! enabling testing and reuse.

use anyhow::{Context, Result};
use rung_core::Stack;
use rung_git::Repository;
use rung_github::{Auth, GitHubClient, PullRequestState};
use serde::Serialize;

/// Diagnostic issue severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// A diagnostic issue found by the doctor.
#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub severity: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

impl Issue {
    /// Create an error issue.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            suggestion: None,
        }
    }

    /// Create a warning issue.
    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            suggestion: None,
        }
    }

    /// Add a suggestion to the issue.
    #[must_use]
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }
}

/// Result of a diagnostic check category.
#[derive(Debug, Default)]
pub struct CheckResult {
    pub issues: Vec<Issue>,
}

impl CheckResult {
    /// Check if this result has any errors.
    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|i| i.severity == Severity::Error)
    }

    /// Check if this result has any warnings.
    pub fn has_warnings(&self) -> bool {
        self.issues.iter().any(|i| i.severity == Severity::Warning)
    }

    /// Check if this result is clean (no issues).
    #[allow(dead_code)]
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }
}

/// Complete diagnostic report.
#[derive(Debug, Default)]
#[allow(dead_code)]
pub struct DiagnosticReport {
    pub git_state: CheckResult,
    pub stack_integrity: CheckResult,
    pub sync_state: CheckResult,
    pub github: CheckResult,
}

#[allow(dead_code)]
impl DiagnosticReport {
    /// Get all issues from all categories.
    pub fn all_issues(&self) -> Vec<&Issue> {
        self.git_state
            .issues
            .iter()
            .chain(self.stack_integrity.issues.iter())
            .chain(self.sync_state.issues.iter())
            .chain(self.github.issues.iter())
            .collect()
    }

    /// Count total errors.
    pub fn error_count(&self) -> usize {
        self.all_issues()
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .count()
    }

    /// Count total warnings.
    pub fn warning_count(&self) -> usize {
        self.all_issues()
            .iter()
            .filter(|i| i.severity == Severity::Warning)
            .count()
    }

    /// Check if the repository is healthy (no errors or warnings).
    pub fn is_healthy(&self) -> bool {
        self.error_count() == 0 && self.warning_count() == 0
    }
}

/// Service for running diagnostic checks.
pub struct DoctorService<'a, G: rung_git::GitOps, S: rung_core::StateStore> {
    repo: &'a G,
    state: &'a S,
    stack: &'a Stack,
}

impl<'a, G: rung_git::GitOps, S: rung_core::StateStore> DoctorService<'a, G, S> {
    /// Create a new doctor service.
    pub const fn new(repo: &'a G, state: &'a S, stack: &'a Stack) -> Self {
        Self { repo, state, stack }
    }

    /// Run all diagnostic checks and return a complete report.
    #[allow(dead_code)]
    pub fn run_diagnostics(&self) -> Result<DiagnosticReport> {
        Ok(DiagnosticReport {
            git_state: self.check_git_state(),
            stack_integrity: self.check_stack_integrity(),
            sync_state: self.check_sync_state()?,
            github: self.check_github(),
        })
    }

    /// Check git repository state.
    pub fn check_git_state(&self) -> CheckResult {
        let mut result = CheckResult::default();

        // Check for dirty working directory
        if !self.repo.is_clean().unwrap_or(false) {
            result.issues.push(
                Issue::warning("Working directory has uncommitted changes")
                    .with_suggestion("Commit or stash changes before running rung commands"),
            );
        }

        // Check for detached HEAD
        if self.repo.current_branch().is_err() {
            result.issues.push(
                Issue::error("HEAD is detached (not on a branch)")
                    .with_suggestion("Checkout a branch with `git checkout <branch>`"),
            );
        }

        // Check for rebase in progress
        if self.repo.is_rebasing() {
            result.issues.push(
                Issue::error("Rebase in progress")
                    .with_suggestion("Complete or abort the rebase before running rung commands"),
            );
        }

        result
    }

    /// Check stack integrity.
    pub fn check_stack_integrity(&self) -> CheckResult {
        let mut result = CheckResult::default();

        for branch in &self.stack.branches {
            // Check if branch exists locally
            if !self.repo.branch_exists(&branch.name) {
                result.issues.push(
                    Issue::warning(format!("Branch '{}' in stack but not in git", branch.name))
                        .with_suggestion("Run `rung sync` to clean up stale branches"),
                );
                continue;
            }

            // Check if parent exists (for non-root branches)
            if let Some(parent) = &branch.parent {
                if !self.repo.branch_exists(parent) && self.stack.find_branch(parent).is_none() {
                    result.issues.push(
                        Issue::error(format!(
                            "Branch '{}' has missing parent '{}'",
                            branch.name, parent
                        ))
                        .with_suggestion("Run `rung sync` to re-parent orphaned branches"),
                    );
                }
            }
        }

        // Check for circular dependencies
        for branch in &self.stack.branches {
            if self.has_circular_dependency(&branch.name, &mut vec![]) {
                result.issues.push(Issue::error(format!(
                    "Circular dependency detected involving '{}'",
                    branch.name
                )));
            }
        }

        result
    }

    /// Check if a branch has a circular dependency.
    fn has_circular_dependency<'b>(
        &self,
        branch_name: &'b str,
        visited: &mut Vec<&'b str>,
    ) -> bool {
        if visited.contains(&branch_name) {
            return true;
        }

        visited.push(branch_name);

        if let Some(branch) = self.stack.find_branch(branch_name) {
            if let Some(parent) = &branch.parent {
                if self.stack.find_branch(parent).is_some() {
                    // Need to clone to avoid lifetime issues
                    let parent_owned = parent.clone();
                    // This is safe because we're only checking existence
                    return self.has_circular_dependency_owned(&parent_owned, visited);
                }
            }
        }

        false
    }

    /// Helper for circular dependency check with owned string.
    fn has_circular_dependency_owned(&self, branch_name: &str, visited: &mut Vec<&str>) -> bool {
        if visited.contains(&branch_name) {
            return true;
        }

        if let Some(branch) = self.stack.find_branch(branch_name) {
            if let Some(parent) = &branch.parent {
                if self.stack.find_branch(parent).is_some() {
                    let parent_owned = parent.clone();
                    return self.has_circular_dependency_owned(&parent_owned, visited);
                }
            }
        }

        false
    }

    /// Check sync state of branches.
    pub fn check_sync_state(&self) -> Result<CheckResult> {
        let mut result = CheckResult::default();

        // Check if sync is in progress
        if self.state.is_sync_in_progress() {
            result.issues.push(
                Issue::warning("Sync operation in progress")
                    .with_suggestion("Run `rung sync --continue` or `rung sync --abort`"),
            );
        }

        // Check each branch's sync state
        let default_branch = self
            .state
            .default_branch()
            .context("Failed to load default branch from config")?;

        let mut needs_sync = 0;
        for branch in &self.stack.branches {
            if !self.repo.branch_exists(&branch.name) {
                continue;
            }

            let parent_name = branch.parent.as_deref().unwrap_or(&default_branch);
            if !self.repo.branch_exists(parent_name) {
                continue;
            }

            // Check if branch needs rebasing
            if let (Ok(branch_commit), Ok(parent_commit)) = (
                self.repo.branch_commit(&branch.name),
                self.repo.branch_commit(parent_name),
            ) {
                if let Ok(merge_base) = self.repo.merge_base(branch_commit, parent_commit) {
                    if merge_base != parent_commit {
                        needs_sync += 1;
                    }
                }
            }
        }

        if needs_sync > 0 {
            result.issues.push(
                Issue::warning(format!("{needs_sync} branch(es) behind their parent"))
                    .with_suggestion("Run `rung sync` to rebase"),
            );
        }

        Ok(result)
    }

    /// Check GitHub connectivity and PR state.
    pub fn check_github(&self) -> CheckResult {
        let mut result = CheckResult::default();

        // Check auth
        let auth = Auth::auto();
        let Ok(client) = GitHubClient::new(&auth) else {
            result.issues.push(
                Issue::error("GitHub authentication failed")
                    .with_suggestion("Set GITHUB_TOKEN or authenticate with `gh auth login`"),
            );
            return result;
        };

        // Get repo info
        let Ok(origin_url) = self.repo.origin_url() else {
            result
                .issues
                .push(Issue::warning("No origin remote configured"));
            return result;
        };

        let Ok((owner, repo_name)) = Repository::parse_github_remote(&origin_url) else {
            result
                .issues
                .push(Issue::warning("Origin is not a GitHub repository"));
            return result;
        };

        // Check PRs for branches that have them
        let Ok(rt) = tokio::runtime::Runtime::new() else {
            return result;
        };

        for branch in &self.stack.branches {
            let Some(pr_number) = branch.pr else {
                continue;
            };

            // Check if PR is still open
            match rt.block_on(client.get_pr(&owner, &repo_name, pr_number)) {
                Ok(pr) => {
                    if pr.state != PullRequestState::Open {
                        let state_str = match pr.state {
                            PullRequestState::Closed => "closed",
                            PullRequestState::Merged => "merged",
                            PullRequestState::Open => "open",
                        };
                        result.issues.push(
                            Issue::warning(format!(
                                "PR #{} for '{}' is {} (not open)",
                                pr_number, branch.name, state_str
                            ))
                            .with_suggestion("Run `rung sync` to clean up or merge the branch"),
                        );
                    }
                }
                Err(_) => {
                    result.issues.push(Issue::warning(format!(
                        "Could not fetch PR #{} for '{}'",
                        pr_number, branch.name
                    )));
                }
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_issue_creation() {
        let error = Issue::error("Test error");
        assert_eq!(error.severity, Severity::Error);
        assert_eq!(error.message, "Test error");
        assert!(error.suggestion.is_none());

        let warning = Issue::warning("Test warning").with_suggestion("Fix it");
        assert_eq!(warning.severity, Severity::Warning);
        assert_eq!(warning.message, "Test warning");
        assert_eq!(warning.suggestion, Some("Fix it".to_string()));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_issue_serializes() {
        let issue = Issue::error("Missing file").with_suggestion("Create the file");
        let json = serde_json::to_string(&issue).expect("serialization should succeed");
        assert!(json.contains("error"));
        assert!(json.contains("Missing file"));
        assert!(json.contains("Create the file"));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_issue_without_suggestion_serializes() {
        let issue = Issue::warning("Minor issue");
        let json = serde_json::to_string(&issue).expect("serialization should succeed");
        assert!(json.contains("warning"));
        assert!(json.contains("Minor issue"));
        // suggestion should be omitted when None
        assert!(!json.contains("suggestion"));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn test_severity_serializes() {
        let error = Severity::Error;
        let json = serde_json::to_string(&error).expect("serialization should succeed");
        assert!(json.contains("error"));

        let warning = Severity::Warning;
        let json = serde_json::to_string(&warning).expect("serialization should succeed");
        assert!(json.contains("warning"));
    }

    #[test]
    fn test_check_result() {
        let mut result = CheckResult::default();
        assert!(result.is_clean());
        assert!(!result.has_errors());
        assert!(!result.has_warnings());

        result.issues.push(Issue::warning("warn"));
        assert!(!result.is_clean());
        assert!(!result.has_errors());
        assert!(result.has_warnings());

        result.issues.push(Issue::error("err"));
        assert!(result.has_errors());
    }

    #[test]
    fn test_check_result_only_errors() {
        let mut result = CheckResult::default();
        result.issues.push(Issue::error("critical"));
        assert!(result.has_errors());
        assert!(!result.has_warnings());
        assert!(!result.is_clean());
    }

    #[test]
    fn test_diagnostic_report() {
        let mut report = DiagnosticReport::default();
        assert!(report.is_healthy());
        assert_eq!(report.error_count(), 0);
        assert_eq!(report.warning_count(), 0);

        report.git_state.issues.push(Issue::warning("dirty"));
        assert!(!report.is_healthy());
        assert_eq!(report.warning_count(), 1);

        report.stack_integrity.issues.push(Issue::error("missing"));
        assert_eq!(report.error_count(), 1);
        assert_eq!(report.all_issues().len(), 2);
    }

    #[test]
    fn test_diagnostic_report_all_categories() {
        let mut report = DiagnosticReport::default();
        report.git_state.issues.push(Issue::warning("git issue"));
        report
            .stack_integrity
            .issues
            .push(Issue::error("stack issue"));
        report.sync_state.issues.push(Issue::warning("sync issue"));
        report.github.issues.push(Issue::error("github issue"));

        assert_eq!(report.all_issues().len(), 4);
        assert_eq!(report.error_count(), 2);
        assert_eq!(report.warning_count(), 2);
    }

    #[test]
    fn test_issue_builder_pattern() {
        let issue = Issue::error("Problem").with_suggestion("Do this");
        assert_eq!(issue.message, "Problem");
        assert_eq!(issue.suggestion, Some("Do this".to_string()));
        assert_eq!(issue.severity, Severity::Error);
    }

    #[test]
    fn test_issue_warning_with_suggestion() {
        let issue = Issue::warning("Minor").with_suggestion("Consider this");
        assert_eq!(issue.severity, Severity::Warning);
        assert_eq!(issue.suggestion, Some("Consider this".to_string()));
    }

    #[test]
    fn test_check_result_multiple_warnings() {
        let mut result = CheckResult::default();
        result.issues.push(Issue::warning("warn1"));
        result.issues.push(Issue::warning("warn2"));
        result.issues.push(Issue::warning("warn3"));

        assert!(!result.is_clean());
        assert!(result.has_warnings());
        assert!(!result.has_errors());
        assert_eq!(result.issues.len(), 3);
    }

    #[test]
    fn test_check_result_multiple_errors() {
        let mut result = CheckResult::default();
        result.issues.push(Issue::error("err1"));
        result.issues.push(Issue::error("err2"));

        assert!(result.has_errors());
        assert!(!result.has_warnings());
        assert_eq!(result.issues.len(), 2);
    }

    #[test]
    fn test_diagnostic_report_empty() {
        let report = DiagnosticReport::default();
        assert!(report.is_healthy());
        assert_eq!(report.error_count(), 0);
        assert_eq!(report.warning_count(), 0);
        assert!(report.all_issues().is_empty());
    }

    #[test]
    fn test_diagnostic_report_single_category() {
        let mut report = DiagnosticReport::default();
        report.git_state.issues.push(Issue::error("detached head"));

        assert!(!report.is_healthy());
        assert_eq!(report.error_count(), 1);
        assert_eq!(report.warning_count(), 0);
        assert_eq!(report.all_issues().len(), 1);
    }

    #[test]
    fn test_severity_equality() {
        assert_eq!(Severity::Error, Severity::Error);
        assert_eq!(Severity::Warning, Severity::Warning);
        assert_ne!(Severity::Error, Severity::Warning);
    }

    #[test]
    fn test_issue_clone() {
        let original = Issue::error("test").with_suggestion("fix");
        let cloned = original.clone();

        assert_eq!(original.message, cloned.message);
        assert_eq!(original.severity, cloned.severity);
        assert_eq!(original.suggestion, cloned.suggestion);
    }

    #[test]
    fn test_check_result_mixed_issues() {
        let mut result = CheckResult::default();
        result.issues.push(Issue::error("error 1"));
        result.issues.push(Issue::warning("warning 1"));
        result.issues.push(Issue::error("error 2"));
        result.issues.push(Issue::warning("warning 2"));

        assert!(result.has_errors());
        assert!(result.has_warnings());
        assert!(!result.is_clean());
        assert_eq!(result.issues.len(), 4);
    }

    // Mock-based tests for DoctorService methods
    #[allow(clippy::unwrap_used)]
    mod mock_tests {
        use super::*;
        use crate::services::test_mocks::{MockGitOps, MockStateStore};
        use rung_core::stack::StackBranch;
        use rung_git::Oid;

        #[test]
        fn test_check_git_state_clean() {
            let git = MockGitOps::new();
            let state = MockStateStore::new();
            let stack = Stack::default();

            let service = DoctorService::new(&git, &state, &stack);
            let result = service.check_git_state();

            assert!(result.is_clean());
        }

        #[test]
        fn test_check_git_state_dirty_working_directory() {
            let git = MockGitOps::new();
            *git.is_clean.borrow_mut() = false;

            let state = MockStateStore::new();
            let stack = Stack::default();

            let service = DoctorService::new(&git, &state, &stack);
            let result = service.check_git_state();

            assert!(result.has_warnings());
            assert!(result.issues[0].message.contains("uncommitted changes"));
        }

        #[test]
        fn test_check_git_state_rebasing() {
            let git = MockGitOps::new();
            *git.is_rebasing.borrow_mut() = true;

            let state = MockStateStore::new();
            let stack = Stack::default();

            let service = DoctorService::new(&git, &state, &stack);
            let result = service.check_git_state();

            assert!(result.has_errors());
            assert!(
                result
                    .issues
                    .iter()
                    .any(|i| i.message.contains("Rebase in progress"))
            );
        }

        #[test]
        fn test_check_stack_integrity_empty_stack() {
            let git = MockGitOps::new();
            let state = MockStateStore::new();
            let stack = Stack::default();

            let service = DoctorService::new(&git, &state, &stack);
            let result = service.check_stack_integrity();

            assert!(result.is_clean());
        }

        #[test]
        fn test_check_stack_integrity_branch_not_in_git() {
            let git = MockGitOps::new(); // No branches exist

            let state = MockStateStore::new();
            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/missing", None::<&str>).unwrap());

            let service = DoctorService::new(&git, &state, &stack);
            let result = service.check_stack_integrity();

            assert!(result.has_warnings());
            assert!(result.issues[0].message.contains("not in git"));
        }

        #[test]
        fn test_check_stack_integrity_missing_parent() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("feature/child", oid);
            // Parent "feature/parent" doesn't exist in git or stack

            let state = MockStateStore::new();
            let mut stack = Stack::default();
            stack
                .add_branch(StackBranch::try_new("feature/child", Some("feature/parent")).unwrap());

            let service = DoctorService::new(&git, &state, &stack);
            let result = service.check_stack_integrity();

            assert!(result.has_errors());
            assert!(
                result
                    .issues
                    .iter()
                    .any(|i| i.message.contains("missing parent"))
            );
        }

        #[test]
        fn test_check_stack_integrity_valid_stack() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("main", oid)
                .with_branch("feature/a", oid)
                .with_branch("feature/b", oid);

            let state = MockStateStore::new();
            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/a", Some("main")).unwrap());
            stack.add_branch(StackBranch::try_new("feature/b", Some("feature/a")).unwrap());

            let service = DoctorService::new(&git, &state, &stack);
            let result = service.check_stack_integrity();

            assert!(result.is_clean());
        }

        #[test]
        fn test_check_sync_state_clean() {
            let oid = Oid::zero();
            let git = MockGitOps::new()
                .with_branch("main", oid)
                .with_branch("feature/a", oid);

            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/a", Some("main")).unwrap());

            let state = MockStateStore::new().with_stack(stack.clone());

            let service = DoctorService::new(&git, &state, &stack);
            let result = service.check_sync_state().unwrap();

            // With same commits, no sync needed
            assert!(!result.has_errors());
        }

        #[test]
        fn test_check_sync_state_sync_in_progress() {
            let git = MockGitOps::new();
            let state = MockStateStore::new();
            *state.sync_in_progress.borrow_mut() = true;

            let stack = Stack::default();

            let service = DoctorService::new(&git, &state, &stack);
            let result = service.check_sync_state().unwrap();

            assert!(result.has_warnings());
            assert!(
                result.issues[0]
                    .message
                    .contains("Sync operation in progress")
            );
        }

        #[test]
        fn test_run_diagnostics() {
            let git = MockGitOps::new();
            let state = MockStateStore::new();
            let stack = Stack::default();

            let service = DoctorService::new(&git, &state, &stack);
            let report = service.run_diagnostics().unwrap();

            // Empty stack with clean repo should have github auth error
            // (since no real GitHub token available in tests)
            // but git_state and stack_integrity should be clean
            assert!(report.git_state.is_clean());
            assert!(report.stack_integrity.is_clean());
        }

        #[test]
        fn test_check_git_state_multiple_issues() {
            let git = MockGitOps::new();
            *git.is_clean.borrow_mut() = false;
            *git.is_rebasing.borrow_mut() = true;

            let state = MockStateStore::new();
            let stack = Stack::default();

            let service = DoctorService::new(&git, &state, &stack);
            let result = service.check_git_state();

            // Should have both dirty working dir warning and rebase error
            assert!(result.has_errors());
            assert!(result.has_warnings());
            assert!(result.issues.len() >= 2);
        }

        #[test]
        fn test_check_stack_integrity_parent_in_stack_not_git() {
            let oid = Oid::zero();
            let git = MockGitOps::new().with_branch("feature/child", oid);
            // Parent exists in stack but not in git

            let state = MockStateStore::new();
            let mut stack = Stack::default();
            stack.add_branch(StackBranch::try_new("feature/parent", None::<&str>).unwrap());
            stack
                .add_branch(StackBranch::try_new("feature/child", Some("feature/parent")).unwrap());

            let service = DoctorService::new(&git, &state, &stack);
            let result = service.check_stack_integrity();

            // Parent exists in stack, so child is OK, but parent branch is not in git
            assert!(result.has_warnings());
            assert!(
                result
                    .issues
                    .iter()
                    .any(|i| i.message.contains("feature/parent")
                        && i.message.contains("not in git"))
            );
        }
    }
}
