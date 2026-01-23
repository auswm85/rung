---
title: restack
description: Move a branch to a different parent in the stack by rebasing it onto a new base.
---

Move a branch to a different parent in the stack. This is useful when you need to reorganize your stack topology—for example, moving a feature branch from one parent to another.

## Usage

```bash
rung restack --onto main
rung restack feature/api --onto main
rung restack --onto feature/base --include-children
rung restack --dry-run
rung restack --continue
rung restack --abort
```

## Aliases

- `rung re` — shorthand for `rung restack`

## Options

| Option               | Description                                    |
| -------------------- | ---------------------------------------------- |
| `--onto <branch>`    | New parent branch to rebase onto (required)    |
| `--include-children` | Also rebase all descendant branches            |
| `--dry-run`          | Show what would be done without making changes |
| `--continue`         | Continue after resolving conflicts             |
| `--abort`            | Abort and restore from backup                  |

## How It Works

When you run `rung restack --onto <new-parent>`:

1. **Validate** — Checks that the move won't create a cycle in the stack
2. **Backup** — Creates backup refs for affected branches
3. **Rebase** — Rebases the branch onto the new parent: `git rebase --onto <new-parent> <old-parent> <branch>`
4. **Update Stack** — Updates the stack topology with the new parent relationship
5. **Report** — Shows what was restacked

### Example

Move a branch to a different parent:

```bash
$ rung restack --onto main
✓ Restacked feat-add-api onto main (was: feat-add-model)
```

### Moving with Children

To move a branch and all its descendants together:

```bash
$ rung restack feat-add-api --onto main --include-children
✓ Restacked feat-add-api onto main
✓ Restacked feat-add-api-tests onto feat-add-api
```

### Dry Run

Preview changes without modifying anything:

```bash
$ rung restack --onto main --dry-run

Would restack:
  feat-add-api: rebase onto main (currently on feat-add-model)
```

## Handling Conflicts

If a conflict occurs during restack, rung pauses and shows you what to do:

```bash
$ rung restack --onto main
✗ Conflict while rebasing feat-add-api

Conflict in: src/api/users.rs

Resolve the conflict, then run:
  rung restack --continue

Or abort and restore:
  rung restack --abort
```

### Resolving Conflicts

1. Open the conflicting files and resolve the conflicts
2. Stage the resolved files:
   ```bash
   git add src/api/users.rs
   ```
3. Continue the restack:
   ```bash
   rung restack --continue
   ```

### Aborting

If you want to discard the partial restack and restore your branches:

```bash
rung restack --abort
```

This restores all affected branches to their pre-restack state using the backup refs.

## Cycle Detection

Rung prevents moves that would create circular dependencies in your stack:

```bash
$ rung restack feat-parent --onto feat-child
Error: Cannot restack feat-parent onto feat-child: would create a cycle
```

A branch cannot be moved onto one of its own descendants.

## Restack State

During a restack operation, rung writes state to `.git/rung/restack_state.json`:

```json
{
  "started_at": "2024-01-15T10:30:00Z",
  "backup_id": "1704067200",
  "target_branch": "feat-add-api",
  "new_parent": "main",
  "old_parent": "feat-add-model",
  "original_branch": "feat-add-api",
  "current_branch": "feat-add-api",
  "completed": [],
  "remaining": ["feat-add-api-tests"],
  "stack_updated": false
}
```

This allows `--continue` to resume from where it left off.

## Notes

- Always commit or stash your changes before restacking
- The stack topology is updated after all rebases complete successfully
- Backup refs are stored in `.git/rung/backups/` for undo capability
- Use `--include-children` when you want to preserve the relative structure of descendant branches

## Related Commands

- [`sync`](/commands/sync/) — Rebase all branches when parents move
- [`undo`](/commands/undo/) — Restore from last backup
- [`status`](/commands/status/) — View stack topology
