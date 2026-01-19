//! `rung log` command - show commits between the base branch and HEAD.

use super::utils::open_repo_and_state;
use crate::output;
use anyhow::{Result, bail};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct LogOutput {
    commits: Vec<CommitInfo>,
    branch: String,
    parent: String,
}

#[derive(Debug, Serialize)]
struct CommitInfo {
    hash: String,
    message: String,
    author: String
}

impl CommitInfo {
    fn display(&self) {
        let msg = format!("{:<10} {:<25}     {}", self.hash, self.message, self.author);
        output::info(&msg); 
    }
}

// Run the log command.
pub fn run(json: bool) -> Result<()> {
    let (repo, state) = open_repo_and_state()?;
    let current = repo.current_branch()?;
    let stack = state.load_stack()?;

    if stack.is_empty() {
        bail!("No branches in stack. Use `rung create <name>` to add one.");
    }

    // Get branches
    let Some(head) = stack.find_branch(&current) else {
        bail!("Current branch '{current}' is not in stack")
    };

    let Some(base) = &head.parent else {
        bail!("Current branch '{current}' has no parent branch")
    };

    // Get commits
    let head_oid = repo.branch_commit(head.name.as_str())?;
    let base_oid = repo.branch_commit(base.as_str())?;
    let commits = repo.commits_between(base_oid, head_oid)?;

    if commits.is_empty() {
        bail!("Current branch has no commits");
    }

    // Collect commits 
    let commits_info: Result<Vec<CommitInfo>> = commits.iter().map(|&oid| { 
        let commit = repo.find_commit(oid)?;
        let hash = commit.id().to_string()[..7].to_owned();
        let message = commit.message().unwrap_or("").trim().to_owned();
        let sig =  commit.author();
        let author = sig.name().unwrap_or("unknown").to_owned();
        
        Ok (CommitInfo { hash, message, author })

    }).collect();

    // Print commits
    if !json {
        commits_info?.iter().for_each(|commit| commit.display()); 
        return Ok(())
    }
    
    //  Display json output
    let log_output = LogOutput {
        commits: commits_info?,
        branch: current,
        parent: base.to_string(),
    };
    
    let json_log_output = serde_json::to_string_pretty(&log_output)?;
    println!("{json_log_output}");


    Ok(())
}
