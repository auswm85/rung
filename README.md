# rung

A Git workflow tool for managing stacked PRs (pull request chains).

## Overview

Rung helps you work with dependent branches by:

- Tracking branch relationships in a stack
- Syncing child branches when parents are updated
- Managing PR chains on GitHub

## Installation

```bash
cargo install --path crates/rung-cli
```

## Usage

```bash
# Initialize rung in a repository
rung init

# Create a new branch in the stack
rung create feature-auth

# View stack status
rung status

# Navigate the stack
rung up      # Move to child branch
rung down    # Move to parent branch

# Sync all branches after rebasing
rung sync

# Submit PRs for the stack
rung submit
```

## Project Structure

```
crates/
  rung-cli/      # Command-line interface
  rung-core/     # Core logic (stack, sync, state)
  rung-git/      # Git operations wrapper
  rung-github/   # GitHub API client
```

## Requirements

- Rust 1.85+
- Git 2.x

## Development

```bash
# Clone and set up git hooks
git clone https://github.com/auswm85/rung
cd rung
git config core.hooksPath .githooks

# Run tests
cargo test

# Build
cargo build --release
```

## License

MIT
