---
title: doctor
description: Diagnose issues with the stack and repository.
since: "0.1.0"
---

Diagnose issues with your stack and repository. Checks for common problems and provides actionable suggestions.

## Usage

```bash
rung doctor
rung doctor --json
```

## Aliases

- `rung doc` — shorthand for `rung doctor`

## What It Checks

### Stack Integrity

- **Branches exist** — All branches in the stack still exist in git
- **Parents are valid** — Each branch's parent exists and is correct
- **No circular dependencies** — The stack doesn't have any cycles

### Git State

- **Clean working directory** — No uncommitted changes
- **Not detached HEAD** — You're on a branch, not a commit
- **No rebase in progress** — No interrupted operations

### Sync State

- **Branches need rebasing** — Which branches are out of sync
- **Sync operations in progress** — Interrupted syncs that need attention

### GitHub Connectivity

- **Authentication** — GitHub auth is configured and working
- **PR status** — PRs are open/closed/merged correctly

## Example Output

### All Good

```bash
$ rung doctor

✓ Stack integrity: OK
✓ Git state: clean
✓ Sync state: all branches synced
✓ GitHub: connected, 3 PRs open

No issues found.
```

### Issues Found

```bash
$ rung doctor

✓ Stack integrity: OK
⚠ Git state: uncommitted changes in 2 files
✗ Sync state: 2 branches need rebasing
✓ GitHub: connected

Issues:
  warning: Uncommitted changes detected
    → Commit or stash changes before syncing

  error: feat-add-user-api is 3 commits behind parent
    → Run `rung sync` to update

  error: feat-add-user-tests is 1 commit behind parent
    → Run `rung sync` to update
```

## Issue Severities

| Severity | Meaning                      |
| -------- | ---------------------------- |
| ✓        | No issues                    |
| ⚠        | Warning — may cause problems |
| ✗        | Error — needs attention      |

## JSON Output

```bash
$ rung doctor --json
```

```json
{
  "stack_integrity": {
    "status": "ok",
    "issues": []
  },
  "git_state": {
    "status": "warning",
    "issues": [
      {
        "severity": "warning",
        "message": "Uncommitted changes detected",
        "suggestion": "Commit or stash changes before syncing",
        "files": ["src/main.rs", "Cargo.toml"]
      }
    ]
  },
  "sync_state": {
    "status": "error",
    "issues": [
      {
        "severity": "error",
        "branch": "feat-add-user-api",
        "message": "3 commits behind parent",
        "suggestion": "Run `rung sync` to update"
      }
    ]
  },
  "github": {
    "status": "ok",
    "authenticated": true,
    "prs": {
      "open": 3,
      "merged": 1,
      "closed": 0
    }
  }
}
```

## Common Issues and Solutions

### Uncommitted Changes

```
⚠ Git state: uncommitted changes in 2 files
```

**Solution:** Commit or stash your changes:

```bash
git add . && git commit -m "WIP"
# or
git stash
```

### Branches Need Rebasing

```
✗ Sync state: 2 branches need rebasing
```

**Solution:** Run sync:

```bash
rung sync
```

### Missing Branch

```
✗ Stack integrity: branch 'feat-old' not found
```

**Solution:** Remove the orphaned branch from the stack, or recreate it:

```bash
# Remove from stack (edit .git/rung/stack.json)
# Or recreate the branch
git checkout -b feat-old origin/feat-old
```

### GitHub Authentication Failed

```
✗ GitHub: authentication failed
```

**Solution:** Re-authenticate:

```bash
gh auth login
# or set GITHUB_TOKEN
export GITHUB_TOKEN=ghp_...
```

### Sync In Progress

```
⚠ Sync state: sync operation in progress
```

**Solution:** Continue or abort the sync:

```bash
rung sync --continue
# or
rung sync --abort
```

## When to Run Doctor

- Before starting a new day's work
- When commands fail unexpectedly
- After resolving merge conflicts
- When the stack seems out of sync

## Related Commands

- [`status`](/commands/status/) — Quick view of stack state
- [`sync`](/commands/sync/) — Fix out-of-sync branches
- [`undo`](/commands/undo/) — Restore from backup
