---
title: undo
description: Undo the last sync operation, restoring all branches to their previous state.
since: "0.1.0"
---

Restore all branches to their state before the last sync operation.

## Usage

```bash
rung undo
```

## Aliases

- `rung un` — shorthand for `rung undo`

## What It Does

When you run `rung undo`:

1. Finds the most recent backup in `.git/rung/backups/`
2. Restores each branch to its backed-up commit
3. Deletes the used backup

## Example

```bash
# After a sync went wrong
$ rung undo

✓ Restored feat-add-user-model to abc1234
✓ Restored feat-add-user-api to def5678
✓ Restored feat-add-user-tests to ghi9012
✓ Removed backup 1704067200
```

## When to Use Undo

- A sync introduced unexpected issues
- You want to restore branches after resolving conflicts differently
- You need to go back to a known good state

## Backup Storage

Rung stores backups in `.git/rung/backups/`:

```
.git/rung/backups/
└── 1704067200/
    ├── feat-add-user-model      # Contains: abc1234...
    ├── feat-add-user-api        # Contains: def5678...
    └── feat-add-user-tests      # Contains: ghi9012...
```

Each file contains the commit SHA that branch pointed to before sync.

## Limitations

- Only the most recent sync can be undone
- Cannot undo a `rung merge` operation
- Cannot undo if you've made commits after syncing

## No Backup Available

If there's no backup to restore:

```bash
$ rung undo
No backup found. Nothing to undo.
```

This happens when:

- No sync has been performed yet
- The backup was already used for an undo
- Backups were manually deleted

## Alternative: Abort

If you're in the middle of a sync with conflicts, use `--abort` instead:

```bash
# During a sync with conflicts
rung sync --abort
```

This is different from `undo`:

- `sync --abort` — Cancels an in-progress sync
- `undo` — Reverses a completed sync

## Related Commands

- [`sync`](/commands/sync/) — The operation that creates backups
- [`sync --abort`](/commands/sync/) — Abort in-progress sync
- [`doctor`](/commands/doctor/) — Check for backup availability
