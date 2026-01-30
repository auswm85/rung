//! Doctor service for diagnosing stack and repository issues.
//!
//! This module contains the diagnostic logic separated from CLI concerns,
//! enabling testing and reuse.

use anyhow::{Context, Result};
use rung_core::{Stack, State};
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
pub struct DoctorService<'a> {
    repo: &'a Repository,
    state: &'a State,
    stack: &'a Stack,
}

impl<'a> DoctorService<'a> {
    /// Create a new doctor service.
    pub const fn new(repo: &'a Repository, state: &'a State, stack: &'a Stack) -> Self {
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
}
