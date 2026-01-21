---
title: init
description: Initialize rung in a Git repository.
---

Initialize rung in the current Git repository. This creates the `.git/rung/` directory to store stack state.

## Usage

```bash
rung init
```

## What It Does

Running `rung init` creates:

```
.git/rung/
├── stack.json      # Branch relationships and PR numbers
├── config.json     # Repository-specific settings
└── backups/        # Sync backup data for undo
```

## Example

```bash
$ cd my-project
$ rung init
✓ Initialized rung in /path/to/my-project
```

If rung is already initialized:

```bash
$ rung init
ℹ rung is already initialized
```

## Notes

- Run this once per repository, before using any other rung commands
- The `.git/rung/` directory is local and not committed to git
- All stack state travels with your `.git` directory

## Related Commands

- [`create`](/commands/create/) — Create your first branch after initialization
- [`status`](/commands/status/) — View the current stack
