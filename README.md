<p align="center">
  <img src="assets/logo.png" alt="rung logo" width="120">
</p>

# rung

A Git workflow tool for managing stacked PRs (pull request chains).

![Demo](assets/demo.gif)

## Overview

Rung helps you work with dependent branches by:

- Tracking branch relationships in a stack
- Syncing child branches when parents are updated
- Managing PR chains on GitHub with automatic stack comments
- Handling merges with automatic descendant rebasing

## Installation

### Pre-built binaries (recommended)

Download the latest release for your platform from [GitHub Releases](https://github.com/auswm85/rung/releases).

**macOS (Apple Silicon):**

```bash
curl -fsSL https://github.com/auswm85/rung/releases/latest/download/rung-$(curl -s https://api.github.com/repos/auswm85/rung/releases/latest | grep tag_name | cut -d '"' -f 4 | sed 's/v//')-aarch64-apple-darwin.tar.gz | tar xz
sudo mv rung /usr/local/bin/
```

**macOS (Intel):**

```bash
curl -fsSL https://github.com/auswm85/rung/releases/latest/download/rung-$(curl -s https://api.github.com/repos/auswm85/rung/releases/latest | grep tag_name | cut -d '"' -f 4 | sed 's/v//')-x86_64-apple-darwin.tar.gz | tar xz
sudo mv rung /usr/local/bin/
```

**Linux (x86_64):**

```bash
curl -fsSL https://github.com/auswm85/rung/releases/latest/download/rung-$(curl -s https://api.github.com/repos/auswm85/rung/releases/latest | grep tag_name | cut -d '"' -f 4 | sed 's/v//')-x86_64-unknown-linux-gnu.tar.gz | tar xz
sudo mv rung /usr/local/bin/
```

**Windows:** Download the `.zip` from [releases](https://github.com/auswm85/rung/releases) and add to your PATH.

### Homebrew (macOS/Linux)

```bash
brew tap auswm85/rung https://github.com/auswm85/rung
brew install rung
```

### From crates.io

```bash
cargo install rung-cli
```

### With cargo-binstall (faster, no compilation)

```bash
cargo binstall rung-cli
```

### From source

```bash
cargo install --path crates/rung-cli
```

## Quick Start

```bash
# Initialize rung in a repository
rung init

# Create your first stacked branch
rung create feature/auth

# Make changes, commit, then create another branch on top
rung create feature/auth-tests

# Submit all branches as PRs
rung submit

# View stack status
rung status
```

## Commands

### Global Options

- `--json` - Output as JSON (for tooling integration). Supported by: `status`, `doctor`, `sync`, `submit`, `merge`

### `rung init`

Initialize rung in the current repository. Creates a `.git/rung/` directory to store stack state.

```bash
rung init
```

### `rung create <name>`

Create a new branch with the current branch as its parent. This establishes the branch relationship in the stack.

```bash
rung create feature/new-feature
```

### `rung status`

Display the current stack as a tree view with sync state and PR status.

```bash
rung status              # Basic status
rung status --fetch      # Fetch latest PR status from GitHub
rung status --json       # Output as JSON for tooling
```

**Options:**

- `--fetch` - Fetch latest PR status from GitHub

### `rung sync`

Sync the stack by rebasing all branches when the base moves forward.

```bash
rung sync                # Sync all branches
rung sync --dry-run      # Preview what would happen
rung sync --base develop # Sync against a different base branch
```

If conflicts occur:

```bash
# Resolve conflicts, then:
git add .
rung sync --continue

# Or abort and restore:
rung sync --abort
```

**Options:**

- `--dry-run` - Show what would be done without making changes
- `--continue` - Continue after resolving conflicts
- `--abort` - Abort and restore from backup
- `-b, --base <branch>` - Base branch to sync against (default: "main")

### `rung submit`

Push all stack branches and create/update PRs on GitHub. Each PR includes a stack comment showing the branch hierarchy.

```bash
rung submit                          # Submit all branches
rung submit --draft                  # Create PRs as drafts
rung submit --force                  # Force push
rung submit --title "My PR title"    # Custom title for current branch
```

**Options:**

- `--draft` - Create PRs as drafts
- `--force` - Force push even if remote has changes
- `-t, --title <title>` - Custom PR title for current branch

### `rung merge`

Merge the current branch's PR via GitHub API. Automatically:

- Rebases all descendant branches onto the new base
- Updates PR bases on GitHub
- Removes the branch from the stack
- Deletes local and remote branches
- Pulls latest changes to keep local up to date

```bash
rung merge                  # Squash merge (default)
rung merge --method merge   # Regular merge commit
rung merge --method rebase  # Rebase merge
rung merge --no-delete      # Keep remote branch after merge
```

**Options:**

- `-m, --method <method>` - Merge method: `squash` (default), `merge`, or `rebase`
- `--no-delete` - Don't delete the remote branch after merge

### `rung undo`

Undo the last sync operation, restoring all branches to their previous state.

```bash
rung undo
```

### `rung nxt`

Navigate to the next (child) branch in the stack.

```bash
rung nxt
```

### `rung prv`

Navigate to the previous (parent) branch in the stack.

```bash
rung prv
```

### `rung move`

Interactive branch picker for quick navigation. Opens a TUI list to select and jump to any branch in the stack.

```bash
rung move    # or `rung mv`
```

Displays all branches and highlights the current branch. PR numbers are shown when available:

```
? Jump to branch:
  feat/auth #41
> feat/api #42 ◀
  feat/ui
```

### `rung doctor`

Diagnose issues with the stack and repository. Checks:

- **Stack integrity**: Branches exist, parents are valid, no circular dependencies
- **Git state**: Clean working directory, not detached HEAD, no rebase in progress
- **Sync state**: Branches that need rebasing, sync operations in progress
- **GitHub connectivity**: Authentication, PR status (open/closed/merged)

```bash
rung doctor
```

Issues are reported with severity (error/warning) and actionable suggestions.

## Typical Workflow

```bash
# Start on main
git checkout main

# Initialize rung (first time only)
rung init

# Create first feature branch
rung create feature/api-client

# Make changes and commit
git add . && git commit -m "Add API client"

# Create dependent branch
rung create feature/api-tests

# Make more changes
git add . && git commit -m "Add API tests"

# Submit both as PRs
rung submit

# After review, merge from bottom of stack
rung prv                    # Go to parent branch
rung merge                  # Merge PR, rebase children automatically

# Continue with remaining PRs
rung merge                  # Merge the next PR
```

## Stack Comments

When you submit PRs, rung adds a comment to each PR showing the stack hierarchy:

```
### Stack

- API Tests #124 👈
- API Client #123
- `main`

---
*Managed by [rung](https://github.com/auswm85/rung)*
```

## Configuration

Rung stores its state in `.git/rung/`:

- `stack.json` - Branch relationships and PR numbers
- `config.json` - Repository-specific settings
- `backups/` - Sync backup data for undo

## Requirements

- Rust 1.85+
- Git 2.x
- GitHub CLI (`gh`) authenticated, or `GITHUB_TOKEN` environment variable

## Project Structure

```
crates/
  rung-cli/      # Command-line interface
  rung-core/     # Core logic (stack, sync, state)
  rung-git/      # Git operations wrapper
  rung-github/   # GitHub API client
```

## Development

```bash
# Clone and set up git hooks
git clone https://github.com/auswm85/rung
cd rung
git config core.hooksPath .githooks

# Run tests
cargo test

# Run with clippy
cargo clippy

# Build release
cargo build --release
```

## Authors

<a href="https://github.com/auswm85/rung/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=auswm85/rung" />
</a>

## License

MIT
