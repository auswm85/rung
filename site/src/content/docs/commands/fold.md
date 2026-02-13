---
title: fold
description: Combine adjacent branches in a stack into a single branch.
since: "0.7.0"
---

Combine adjacent branches in a stack into a single branch. This is the inverse of `rung split` — useful when you've split a feature too granularly or want to consolidate related branches.

## Usage

```bash
rung fold
rung fold --into-parent
rung fold --include-children
rung fold branch1 branch2 branch3
rung fold --dry-run
rung fold --abort
```

## Aliases

- `rung fo` — shorthand for `rung fold`

## Options

| Option               | Description                                    |
| -------------------- | ---------------------------------------------- |
| `--into-parent`      | Fold the current branch into its parent        |
| `--include-children` | Fold all children into the current branch      |
| `--dry-run`          | Show what would be done without making changes |
| `--abort`            | Abort the current fold and restore from backup |

## How It Works

When you run `rung fold`:

1. **Analyze** — Examines the current branch's parent and children in the stack
2. **Select** — Interactive UI lets you choose the fold operation
3. **Confirm** — Shows what will be folded and asks for confirmation
4. **Execute** — Combines branches and updates the stack topology
5. **Cleanup** — Removes folded branches and reports any PRs to close

### Fold Directions

**Upward Fold (`--into-parent`)**: Merges the current branch into its parent. The current branch is deleted and its commits become part of the parent.

**Downward Fold (`--include-children`)**: Merges all children into the current branch. Child branches are deleted and their commits become part of the current branch.

**Explicit Selection**: Specify branch names directly for fine-grained control over which adjacent branches to fold.

### Example: Fold Into Parent

```bash
$ rung status
main
└── feat-auth-model
    └── feat-auth-api      ← you are here
        └── feat-auth-tests

$ rung fold --into-parent
Will fold [feat-auth-api] into 'feat-auth-model'
? Proceed with fold? Yes

✓ Folded 1 branch(es) into 'feat-auth-model' (3 commits)
  • removed feat-auth-api

$ rung status
main
└── feat-auth-model        ← now includes auth-api commits
    └── feat-auth-tests
```

### Example: Fold Children

```bash
$ rung status
main
└── feat-auth              ← you are here
    ├── feat-auth-model
    └── feat-auth-api

$ rung fold --include-children
Will fold [feat-auth-model, feat-auth-api] into 'feat-auth'
? Proceed with fold? Yes

✓ Folded 2 branch(es) into 'feat-auth' (5 commits)
  • removed feat-auth-model
  • removed feat-auth-api

$ rung status
main
└── feat-auth              ← now includes all child commits
```

### Example: Interactive Selection

```bash
$ rung fold
Select fold operation:
> Fold into parent (merge feat-auth-api into parent)
  Fold children (feat-auth-model) into feat-auth-api
  Cancel
```

### Dry Run

Preview the fold without making changes:

```bash
$ rung fold --into-parent --dry-run
Would fold 1 branch(es) into 'feat-auth-model'
Branches to fold:
  feat-auth-api

Dry run - no changes made
```

## Aborting a Fold

If something goes wrong during the fold, you can restore your branches:

```bash
rung fold --abort
```

This restores the original branches and stack topology from the backup.

## Fold State

During a fold operation, rung writes state to `.git/rung/fold_state.json`:

```json
{
  "started_at": "2024-01-15T10:30:00Z",
  "backup_id": "1704067200",
  "target_branch": "feat-auth-model",
  "branches_to_fold": ["feat-auth-api"],
  "new_parent": "main",
  "completed": [],
  "stack_updated": false
}
```

This allows `--abort` to restore the original state if needed.

## PR Handling

When you fold branches that have open PRs:

- The command reports which PRs should be closed
- Run `rung submit` after folding to update PR state
- Child PRs targeting a folded branch are automatically retargeted

```bash
$ rung fold --into-parent
✓ Folded 1 branch(es) into 'feat-auth-model' (3 commits)
  • removed feat-auth-api
PRs to close: #42
Run `rung submit` to update PRs
```

## Workflow Example

```bash
# You split a branch too granularly and want to consolidate
$ rung status
main
└── feat-user-model (#40)
    └── feat-user-validation (#41)
        └── feat-user-api (#42)
            └── feat-user-tests

# Fold validation and api into model
$ git checkout feat-user-model
$ rung fold --include-children
# Select feat-user-validation and feat-user-api

$ rung status
main
└── feat-user-model (#40)    ← now includes validation and api
    └── feat-user-tests

# Update the remaining PRs
$ rung submit
```

## Notes

- Folded branches must be adjacent in the stack (form a parent-child chain)
- The oldest branch in the chain becomes the target (receives all commits)
- Commits are preserved in order from oldest to newest branch
- Backup refs are stored in `.git/rung/backups/` for undo capability
- Always commit or stash your changes before folding

## Limitations

- Cannot fold the root branch (main/master)
- Branches must form a linear chain (no branching within selection)
- Interactive mode requires at least a parent or child to fold

## Related Commands

- [`split`](/commands/split/) — Split a branch into multiple branches (inverse of fold)
- [`restack`](/commands/restack/) — Move a branch to a different parent
- [`undo`](/commands/undo/) — Restore from last backup
- [`submit`](/commands/submit/) — Update PRs after folding
