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

### Forge Connectivity

Works with both GitHub and GitLab — the check targets whichever forge your `origin` remote points at, and the progress line names it (e.g. `Checking GitLab...`).

- **Authentication** — forge auth is configured and working (GitHub `gh`/`GITHUB_TOKEN`, or GitLab `glab`/`GITLAB_TOKEN`)
- **PR/MR status** — pull requests (GitHub) or merge requests (GitLab) are open/closed/merged correctly

## Example Output

### All Good

```bash
$ rung doctor

  Checking rung initialization... ✓
  Checking git state... ✓
  Checking stack integrity... ✓
  Checking sync state... ✓
  Checking GitHub... ✓

✓ No issues found!
```

(On a GitLab repository, the last check line reads `Checking GitLab... ✓`.)

### Issues Found

```bash
$ rung doctor

  Checking rung initialization... ✓
  Checking git state... ⚠
  Checking stack integrity... ✓
  Checking sync state... ✗
  Checking GitLab... ✓

  ⚠ Uncommitted changes detected
    → Commit or stash changes before syncing
  ✗ feat-add-user-api is 3 commits behind parent
    → Run `rung sync` to update

✗ Found 2 issue(s) (1 error(s), 1 warning(s))
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

The `--json` output is a flat report: overall health, error/warning counts, and a single list of issues aggregated across all checks (git state, stack integrity, sync state, and forge connectivity). It is forge-neutral — issue messages name the forge where relevant.

```json
{
  "healthy": false,
  "errors": 1,
  "warnings": 1,
  "issues": [
    {
      "severity": "warning",
      "message": "Uncommitted changes detected",
      "suggestion": "Commit or stash changes before syncing"
    },
    {
      "severity": "error",
      "message": "feat-add-user-api is 3 commits behind parent",
      "suggestion": "Run `rung sync` to update"
    }
  ]
}
```

Each issue has a `severity` (`error` or `warning`) and `message`; `suggestion` is present only when there's a recommended fix.

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

### Forge Authentication Failed

```
  ✗ GitHub authentication failed
```

The message names the detected forge — on a GitLab remote it reads `GitLab authentication failed`.

**Solution:** Re-authenticate with the matching forge:

```bash
# GitHub
gh auth login
# or: export GITHUB_TOKEN=ghp_...

# GitLab
glab auth login
# or: export GITLAB_TOKEN=glpat_...
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
