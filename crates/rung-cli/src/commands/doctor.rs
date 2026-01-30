//! `rung doctor` command - Diagnose issues with the stack and repository.

use anyhow::Result;
use colored::Colorize;
use rung_core::State;
use rung_git::Repository;
use serde::Serialize;

use crate::output;
use crate::services::{CheckResult, DoctorService, Issue, Severity};

/// JSON output for doctor command.
#[derive(Debug, Serialize)]
struct DoctorOutput {
    healthy: bool,
    errors: usize,
    warnings: usize,
    issues: Vec<Issue>,
}

/// Run the doctor command.
pub fn run(json: bool) -> Result<()> {
    // Check if we're in a git repo
    let Ok(repo) = Repository::open_current() else {
        if json {
            return output_json(&[Issue::error("Not inside a git repository")]);
        }
        output::error("Not inside a git repository");
        return Ok(());
    };

    let Some(workdir) = repo.workdir() else {
        if json {
            return output_json(&[Issue::error("Cannot run in bare repository")]);
        }
        output::error("Cannot run in bare repository");
        return Ok(());
    };

    let state = State::new(workdir)?;

    // Check initialization
    if !json {
        println!();
        print_check("Checking rung initialization...");
    }
    if !state.is_initialized() {
        let issue = Issue::error("Rung not initialized in this repository")
            .with_suggestion("Run `rung init` to initialize");
        if json {
            return output_json(&[issue]);
        }
        print_issues(&[&issue]);
        return Ok(());
    }
    if !json {
        print_ok();
    }

    // Load stack and create service
    let stack = state.load_stack()?;
    let service = DoctorService::new(&repo, &state, &stack);

    // Run diagnostics with progress output
    if !json {
        print_check("Checking git state...");
    }
    let git_result = service.check_git_state();
    if !json {
        print_status(&git_result);
    }

    if !json {
        print_check("Checking stack integrity...");
    }
    let stack_result = service.check_stack_integrity();
    if !json {
        print_status(&stack_result);
    }

    if !json {
        print_check("Checking sync state...");
    }
    let sync_result = service.check_sync_state()?;
    if !json {
        print_status(&sync_result);
    }

    if !json {
        print_check("Checking GitHub...");
    }
    let github_result = service.check_github();
    if !json {
        print_status(&github_result);
    }

    // Collect all issues
    let all_issues: Vec<&Issue> = git_result
        .issues
        .iter()
        .chain(stack_result.issues.iter())
        .chain(sync_result.issues.iter())
        .chain(github_result.issues.iter())
        .collect();

    // Output
    if json {
        let owned_issues: Vec<Issue> = all_issues.into_iter().cloned().collect();
        return output_json(&owned_issues);
    }

    println!();
    print_issues(&all_issues);
    print_summary(&all_issues);

    Ok(())
}

/// Output issues as JSON.
fn output_json(issues: &[Issue]) -> Result<()> {
    let errors = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .count();
    let warnings = issues
        .iter()
        .filter(|i| i.severity == Severity::Warning)
        .count();

    let output = DoctorOutput {
        healthy: errors == 0 && warnings == 0,
        errors,
        warnings,
        issues: issues.to_vec(),
    };

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn print_check(message: &str) {
    print!("  {message}");
}

fn print_ok() {
    println!(" {}", "✓".green());
}

fn print_status(result: &CheckResult) {
    if result.has_errors() {
        println!(" {}", "✗".red());
    } else if result.has_warnings() {
        println!(" {}", "⚠".yellow());
    } else {
        println!(" {}", "✓".green());
    }
}

fn print_issues(issues: &[&Issue]) {
    if issues.is_empty() {
        return;
    }

    for issue in issues {
        let icon = match issue.severity {
            Severity::Error => "✗".red(),
            Severity::Warning => "⚠".yellow(),
        };

        println!("  {icon} {}", issue.message);

        if let Some(suggestion) = &issue.suggestion {
            println!("    {} {suggestion}", "→".dimmed());
        }
    }
    println!();
}

fn print_summary(issues: &[&Issue]) {
    let errors = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .count();
    let warnings = issues
        .iter()
        .filter(|i| i.severity == Severity::Warning)
        .count();

    if errors == 0 && warnings == 0 {
        output::success("No issues found!");
    } else {
        let summary = format!(
            "Found {} issue(s) ({} error(s), {} warning(s))",
            errors + warnings,
            errors,
            warnings
        );
        if errors > 0 {
            output::error(&summary);
        } else {
            output::warn(&summary);
        }
    }
    println!();
}
