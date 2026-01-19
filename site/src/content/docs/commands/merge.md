---
title: merge
description: Merge the current branch's PR via GitHub API and update the stack.
---

Merge the current branch's pull request via the GitHub API. Automatically handles rebasing descendants and updating PR bases.

## Usage

```bash
rung merge
rung merge --method merge
rung merge --method rebase
rung merge --no-delete
```

## Aliases

- `rung m` — shorthand for `rung merge`

## Options

| Option                  | Description                                            |
| ----------------------- | ------------------------------------------------------ |
| `-m, --method <method>` | Merge method: `squash` (default), `merge`, or `rebase` |
| `--no-delete`           | Don't delete the remote branch after merge             |

## Merge Methods

| Method   | Description                             |
| -------- | --------------------------------------- |
| `squash` | Combines all commits into one (default) |
| `merge`  | Creates a merge commit                  |
| `rebase` | Rebases commits onto the base branch    |

## What Merge Does

When you run `rung merge`:

1. **Merge PR** — Merges the PR via GitHub API using the specified method
2. **Rebase descendants** — Rebases all child branches onto the new base
3. **Update PR bases** — Updates child PRs to point to the new base branch
4. **Remove from stack** — Removes the merged branch from the stack
5. **Delete branches** — Deletes local and remote branches (unless `--no-delete`)
6. **Pull changes** — Pulls latest changes to keep local up to date

## Example

```bash
# On feat-add-user-model with approved PR #41
$ rung merge

✓ Merged PR #41 (squash)
✓ Rebased feat-add-user-api onto main
✓ Updated PR #42 base to main
✓ Deleted branch feat-add-user-model
✓ Pulled latest main
```

### Before

```
  Stack
  ──────────────────────────────────────────────────
  ● ▶ feat-add-user-model #41 ← main
  ●   feat-add-user-api #42 ← feat-add-user-model
  ●   feat-add-user-tests #43 ← feat-add-user-api
  ──────────────────────────────────────────────────
```

### After

```
  Stack
  ──────────────────────────────────────────────────
  ● ▶ feat-add-user-api #42 ← main
  ●   feat-add-user-tests #43 ← feat-add-user-api
  ──────────────────────────────────────────────────
```

## Merge Order

Always merge from the bottom of the stack up (closest to `main` first). This ensures:

- Each PR has the correct context
- Reviewers see the full picture
- CI runs against the correct base

## Using Different Methods

### Squash Merge (Default)

```bash
rung merge
# or
rung merge --method squash
```

Combines all commits into a single commit on the base branch. Good for clean history.

### Regular Merge

```bash
rung merge --method merge
```

Creates a merge commit preserving all individual commits. Good for preserving detailed history.

### Rebase Merge

```bash
rung merge --method rebase
```

Replays commits onto the base branch without a merge commit. Creates linear history.

## Keep Remote Branch

If you want to keep the remote branch after merging:

```bash
rung merge --no-delete
```

Useful for:

- Preserving branch for reference
- Required by some CI/CD pipelines
- When you need to re-reference the branch later

## JSON Output

```bash
$ rung merge --json
```

```json
{
  "merged": {
    "branch": "feat-add-user-model",
    "pr": 41,
    "method": "squash"
  },
  "rebased": [
    {
      "branch": "feat-add-user-api",
      "new_base": "main",
      "commits": 2
    }
  ],
  "updated_prs": [42],
  "deleted": ["feat-add-user-model"]
}
```

## Requirements

- The PR must be approved and all checks passing
- You must have write access to the repository
- GitHub authentication must be configured

## Notes

- The merge uses GitHub's API, so it respects branch protection rules
- If descendant rebasing causes conflicts, you'll need to resolve them
- After merging, your checkout moves to the first child branch (or main if no children)

## Related Commands

- [`submit`](/commands/submit/) — Submit PRs for review
- [`status`](/commands/status/) — Check PR status
- [`sync`](/commands/sync/) — Sync branches manually
