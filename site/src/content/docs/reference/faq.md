---
title: FAQ
description: Frequently asked questions about rung.
---

## General

### What is rung?

Rung is a CLI tool for managing stacked pull requests on GitHub. It tracks branch dependencies, automates rebasing, and manages PRs across your stack.

### Why is it called "rung"?

A rung is a crosspiece on a ladder. Like rungs help you climb, stacked PRs help you build features step by step.

### Does rung work with GitHub Enterprise?

Not yet, but it's on the roadmap. Currently rung only works with github.com.

### Does rung work with GitLab/Bitbucket?

Not currently. Rung is GitHub-specific. Support for other platforms may be added in the future.

## Installation

### How do I install rung?

See the [Installation guide](/getting-started/installation/) for all methods:

- Pre-built binaries
- Homebrew
- cargo install
- From source

### What are the requirements?

- Git 2.x
- GitHub CLI (`gh`) authenticated, or `GITHUB_TOKEN` environment variable

### Do I need Rust installed?

Only if building from source. Pre-built binaries and Homebrew don't require Rust.

## Usage

### How do I start using rung?

```bash
rung init                            # Initialize
rung create -m "feat: my feature"    # Create first branch
# ... make changes ...
rung create -m "feat: next part"     # Stack more branches
rung submit                          # Create PRs
```

See the [Quick Start](/getting-started/quickstart/) for a complete tutorial.

### Can I use rung with existing branches?

Currently, branches need to be created with `rung create` to be tracked in the stack. Existing branches aren't automatically detected.

### How many branches should I have in a stack?

3-5 branches is ideal. Longer stacks:

- Are harder to review
- Have more potential for conflicts
- Take longer to merge

### Can I have multiple stacks?

Yes. Start each stack from `main`:

```bash
git checkout main
rung create -m "feat: auth"          # Stack 1

git checkout main
rung create -m "feat: payments"      # Stack 2
```

### How do I rename a branch in the stack?

```bash
# Standard git rename
git branch -m old-name new-name

# Update the stack manually
# Edit .git/rung/stack.json
```

### Can I reorder branches in the stack?

Not directly. The stack order is determined by parent relationships. You'd need to:

1. Manually update `.git/rung/stack.json`
2. Rebase branches to match the new order
3. Update PR base branches

This is an advanced operation—usually it's easier to create a new stack.

## Syncing

### When should I sync?

Sync whenever `main` (or your base branch) gets new commits:

```bash
git checkout main
git pull
rung sync
```

Daily syncing is a good habit.

### What happens during sync?

For each branch in your stack (bottom-up):

1. Rung checks if the parent has moved
2. If yes, runs `git rebase --onto <new-parent> <old-parent> <branch>`
3. If conflicts occur, pauses for you to resolve

### How do I abort a sync?

```bash
rung sync --abort
```

This restores all branches to their pre-sync state.

## Pull Requests

### How does rung create PRs?

Rung uses the GitHub API to:

1. Create PRs with correct base branches
2. Use commit messages as PR titles
3. Add stack navigation comments

### Can I customize PR descriptions?

Not directly through rung. You can:

1. Edit PR descriptions on GitHub after creation
2. Use GitHub PR templates in your repository

### What happens when I merge a PR?

`rung merge`:

1. Merges the PR via GitHub API
2. Rebases child branches onto the new base
3. Updates child PR base branches
4. Removes the branch from the stack
5. Deletes local and remote branches

## Troubleshooting

### Why is my stack out of sync?

Usually because:

- `main` got new commits
- Someone else pushed to a shared branch
- A manual rebase was done outside rung

Solution: `rung sync`

### Can I undo a sync?

Yes:

```bash
rung undo
```

This restores branches to their pre-sync state using backups.

### Can I undo a merge?

Not directly. The PR is merged on GitHub. You'd need to:

1. Revert the merge commit on GitHub
2. Manually restore your stack

### What if I get conflicts?

1. Resolve the conflicts in your editor
2. Stage the resolved files: `git add .`
3. Continue: `rung sync --continue`

See [Conflict Resolution](/guides/conflict-resolution/) for details.

### Why isn't my branch in the stack?

Branches must be created with `rung create` to be tracked. If you created a branch with `git checkout -b`, it won't be in the stack.

## Contributing

### How can I contribute?

See the [Contributing Guide](https://github.com/auswm85/rung/blob/main/CONTRIBUTING.md) for details on how to get started.

### Where do I report bugs?

Open an issue on [GitHub](https://github.com/auswm85/rung/issues).

## Related

- [Quick Start](/getting-started/quickstart/) — Get started tutorial
- [Troubleshooting](/reference/troubleshooting/) — Common issues
- [Architecture](/concepts/architecture/) — How rung works
