---
title: Command Overview
description: All rung commands at a glance.
---

Quick reference for all rung commands. Click any command for detailed documentation.

## Global Options

These options work with most commands:

| Option        | Description                              |
| ------------- | ---------------------------------------- |
| `--json`      | Output as JSON (for tooling integration) |
| `-q, --quiet` | Suppress informational output            |
| `--help`      | Show help for any command                |
| `--version`   | Show rung version                        |

## Commands

| Command                                 | Alias  | Description                           |
| --------------------------------------- | ------ | ------------------------------------- |
| [`init`](/commands/init/)               |        | Initialize rung in a repository       |
| [`create`](/commands/create/)           | `c`    | Create a new branch in the stack      |
| [`status`](/commands/status/)           | `st`   | Display stack tree and PR status      |
| [`sync`](/commands/sync/)               | `sy`   | Rebase all branches when parents move |
| [`submit`](/commands/submit/)           | `sm`   | Push branches and create/update PRs   |
| [`merge`](/commands/merge/)             | `m`    | Merge PR and update the stack         |
| [`restack`](/commands/restack/)         | `re`   | Move branch to different parent       |
| [`nxt`](/commands/navigation/)          | `n`    | Navigate to child branch              |
| [`prv`](/commands/navigation/)          | `p`    | Navigate to parent branch             |
| [`move`](/commands/navigation/)         | `mv`   | Interactive branch picker             |
| [`log`](/commands/log/)                 |        | Show commits on current branch        |
| [`absorb`](/commands/absorb/)           | `ab`   | Absorb staged changes into commits    |
| [`undo`](/commands/undo/)               | `un`   | Restore stack to pre-sync state       |
| [`doctor`](/commands/doctor/)           | `doc`  | Diagnose stack and repo issues        |
| [`update`](/commands/update/)           | `up`   | Update rung to the latest version     |
| [`completions`](/commands/completions/) | `comp` | Generate shell completions            |

## Quick Reference

### Starting a Stack

```bash
rung init                            # Initialize rung
rung create feature/auth             # Create named branch
rung create -m "feat: add auth"      # Create from commit message
```

### Working with Stacks

```bash
rung status                          # View stack tree (or: rung st)
rung sync                            # Rebase all branches (or: rung sy)
rung submit                          # Push and create PRs (or: rung sm)
```

### Navigation

```bash
rung nxt                             # Go to child branch
rung prv                             # Go to parent branch
rung move                            # Interactive picker
rung log                             # Show branch commits
```

### Merging

```bash
rung merge                           # Squash merge (default)
rung merge --method merge            # Regular merge
rung merge --method rebase           # Rebase merge
```

### Restacking

```bash
rung restack --onto main             # Move current branch onto main
rung restack feat/api --onto main    # Move specific branch
rung restack --onto main --include-children  # Also move descendants
```

### Absorbing Changes

```bash
git add -p                           # Stage changes selectively
rung absorb --dry-run                # Preview what would be absorbed
rung absorb                          # Create fixup commits
git rebase -i --autosquash main      # Apply the fixups
```

### Recovery

```bash
rung undo                            # Restore from last sync
rung sync --abort                    # Abort in-progress sync
rung doctor                          # Diagnose issues
```
