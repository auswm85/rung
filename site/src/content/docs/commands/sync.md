---
title: sync
description: Sync the stack by rebasing all branches when the base moves forward.
---

Sync the stack by rebasing all branches when their parent branches have moved forward. This is the core command for keeping your stack up-to-date.

## Usage

```bash
rung sync
rung sync --dry-run
rung sync --base develop
rung sync --force
rung sync --continue
rung sync --abort
rung sync --no-push
```

## Aliases

- `rung sy` — shorthand for `rung sync`

## Options

| Option                | Description                                                              |
| --------------------- | ------------------------------------------------------------------------ |
| `--dry-run`           | Show what would be done without making changes                           |
| `-b, --base <branch>` | Base branch to sync against (default: repository's default branch)       |
| `--force`             | Proceed even if branches have diverged from remote                       |
| `--continue`          | Continue after resolving conflicts                                       |
| `--abort`             | Abort and restore from backup                                            |
| `--no-push`           | Skip pushing branches to remote after sync                               |

## How It Works

When you run `rung sync`:

1. **Backup** — Creates backup refs for all branches
2. **Plan** — Determines which branches need rebasing
3. **Rebase** — For each branch (bottom-up): `git rebase --onto <new-parent> <old-parent> <branch>`
4. **Report** — Shows what was rebased

### Example

```bash
$ rung sync
✓ Synced feat-add-user-model (rebased 3 commits onto main)
✓ Synced feat-add-user-api (rebased 2 commits onto feat-add-user-model)
✓ Synced feat-add-user-tests (rebased 1 commit onto feat-add-user-api)
```

### Dry Run

Preview changes without modifying anything:

```bash
$ rung sync --dry-run

Would sync:
  feat-add-user-model: rebase 3 commits onto main (abc123..def456)
  feat-add-user-api: rebase 2 commits onto feat-add-user-model
  feat-add-user-tests: rebase 1 commit onto feat-add-user-api
```

## Handling Conflicts

If a conflict occurs during sync, rung pauses and shows you what to do:

```bash
$ rung sync
✓ Synced feat-add-user-model
✗ Conflict in feat-add-user-api

Conflict in: src/api/users.rs

Resolve the conflict, then run:
  rung sync --continue

Or abort and restore:
  rung sync --abort
```

### Resolving Conflicts

1. Open the conflicting files and resolve the conflicts
2. Stage the resolved files:
   ```bash
   git add src/api/users.rs
   ```
3. Continue the sync:
   ```bash
   rung sync --continue
   ```

### Aborting

If you want to discard the partial sync and restore your branches:

```bash
rung sync --abort
```

This restores all branches to their pre-sync state using the backup refs.

## Using a Different Base

By default, rung auto-detects your repository's default branch. To use a different base:

```bash
rung sync --base develop
```

## Sync State

During a sync operation, rung writes state to `.git/rung/sync_state.json`:

```json
{
  "started_at": "2024-01-15T10:30:00Z",
  "backup_id": "1704067200",
  "current_branch": "feat-add-user-api",
  "completed": ["feat-add-user-model"],
  "remaining": ["feat-add-user-tests"]
}
```

This allows `--continue` to resume from where it left off.

## JSON Output

```bash
$ rung sync --json
```

```json
{
  "status": "complete",
  "branches_synced": [
    { "name": "feat-add-user-model", "commits": 3 },
    { "name": "feat-add-user-api", "commits": 2 }
  ],
  "backup_id": "1704067200"
}
```

Or if there's a conflict:

```json
{
  "status": "conflict",
  "branch": "feat-add-user-api",
  "files": ["src/api/users.rs"],
  "completed": ["feat-add-user-model"],
  "remaining": ["feat-add-user-tests"],
  "backup_id": "1704067200"
}
```

## Divergence Detection

If any branches have diverged from their remote tracking branches (both local and remote have unique commits), sync will warn and abort:

```bash
$ rung sync
⚠ Branch feat-add-api has diverged from remote (2 ahead, 1 behind)
Error: Cannot sync with diverged branches. Use --force to proceed anyway.
```

Use `--force` to proceed with diverged branches. This is safe because rung creates backups before any rebase operation.

## Notes

- Always commit or stash your changes before syncing
- The sync algorithm processes branches bottom-up (from root to tips)
- Backup refs are stored in `.git/rung/backups/` for undo capability
- If no branches need syncing, rung reports "Already synced"
- Use `--force` when you intentionally want to sync branches that have diverged from remote

## Related Commands

- [`undo`](/commands/undo/) — Restore from last sync backup
- [`status`](/commands/status/) — Check which branches need syncing
- [`submit`](/commands/submit/) — Push after syncing
