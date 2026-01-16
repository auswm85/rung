//! CLI command definitions and handlers.

use clap::{Parser, Subcommand};

pub mod create;
pub mod doctor;
pub mod init;
pub mod merge;
pub mod navigate;
pub mod status;
pub mod submit;
pub mod sync;
pub mod undo;
pub mod update;

/// Rung - The developer's ladder for stacked PRs.
///
/// A lightweight orchestration layer for Git that enables "linear-parallel"
/// development by automating the management of dependent PR stacks.
#[derive(Parser)]
#[command(name = "rung")]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    /// Output as JSON (for tooling integration).
    ///
    /// Supported by: status, doctor, sync, submit, merge
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

/// Available commands.
#[derive(Subcommand)]
pub enum Commands {
    /// Initialize rung in the current repository.
    Init,

    /// Create a new branch in the stack.
    ///
    /// Creates a new branch with the current branch as its parent.
    #[command(alias = "c")]
    Create {
        /// Name of the new branch.
        name: String,
    },

    /// Display the current stack status.
    ///
    /// Shows a tree view of all branches in the stack with their
    /// sync state and PR status.
    #[command(alias = "st")]
    Status {
        /// Fetch latest PR status from GitHub.
        #[arg(long)]
        fetch: bool,
    },

    /// Sync the stack by rebasing all branches.
    ///
    /// Detects merged PRs, updates stack topology, rebases branches,
    /// updates GitHub PR base branches, and pushes all changes.
    #[command(alias = "sy")]
    Sync {
        /// Show what would be done without making changes.
        #[arg(long)]
        dry_run: bool,

        /// Continue a paused sync after resolving conflicts.
        #[arg(long, name = "continue")]
        continue_: bool,

        /// Abort the current sync and restore from backup.
        #[arg(long)]
        abort: bool,

        /// Skip pushing branches to remote after sync.
        #[arg(long)]
        no_push: bool,

        /// Base branch to sync against (defaults to "main").
        #[arg(long, short)]
        base: Option<String>,
    },

    /// Push branches and create/update PRs.
    ///
    /// Pushes all stack branches to the remote and creates or
    /// updates pull requests with stack navigation links.
    #[command(alias = "sm")]
    Submit {
        /// Create PRs as drafts (won't trigger CI).
        #[arg(long)]
        draft: bool,

        /// Force push even if lease check fails.
        #[arg(long)]
        force: bool,

        /// Custom PR title for current branch (overrides auto-generated title).
        #[arg(long, short)]
        title: Option<String>,
    },

    /// Undo the last sync operation.
    ///
    /// Restores all branches to their state before the last sync.
    #[command(alias = "un")]
    Undo,

    /// Merge the current branch's PR and clean up.
    ///
    /// Merges the PR via GitHub API, deletes the remote branch,
    /// removes it from the stack, and checks out the parent.
    #[command(alias = "m")]
    Merge {
        /// Merge method: squash (default), merge, or rebase.
        #[arg(long, short, default_value = "squash")]
        method: String,

        /// Don't delete the remote branch after merge.
        #[arg(long)]
        no_delete: bool,
    },

    /// Navigate to the next branch in the stack (child).
    #[command(alias = "n")]
    Nxt,

    /// Navigate to the previous branch in the stack (parent).
    #[command(alias = "p")]
    Prv,

    /// Diagnose issues with the stack and repository.
    ///
    /// Checks stack integrity, git state, sync status, and GitHub connectivity.
    #[command(alias = "doc")]
    Doctor,

    /// Update rung to the latest version.
    ///
    /// Checks crates.io for the latest version and installs it using
    /// cargo-binstall (fast) or cargo install (fallback).
    #[command(alias = "up")]
    Update {
        /// Only check for updates without installing.
        #[arg(long)]
        check: bool,
    },
}
