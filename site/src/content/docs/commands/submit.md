---
title: submit
description: Push all stack branches and create/update PRs on GitHub.
since: "0.1.0"
---

Push all stack branches and create or update pull requests on GitHub. Each PR includes a stack comment showing the branch hierarchy.

## Usage

```bash
rung submit
rung submit --draft
rung submit --force
rung submit --title "Custom title"
rung submit --dry-run
```

## Aliases

- `rung sm` — shorthand for `rung submit`

## Options

| Option                | Description                                                   |
| --------------------- | ------------------------------------------------------------- |
| `--draft`             | Create PRs as drafts                                          |
| `--force`             | Force push even if lease check fails                          |
| `-t, --title <title>` | Custom PR title for current branch (overrides commit message) |
| `--dry-run`           | Preview what would happen without pushing or creating PRs     |

## Example

```bash
$ rung submit

✓ Pushed feat-add-user-model
✓ Created PR #41: feat: add user model
  https://github.com/org/repo/pull/41

✓ Pushed feat-add-user-api
✓ Created PR #42: feat: add user API (base: feat-add-user-model)
  https://github.com/org/repo/pull/42

✓ Pushed feat-add-user-tests
✓ Created PR #43: feat: add user tests (base: feat-add-user-api)
  https://github.com/org/repo/pull/43
```

## What Submit Does

For each branch in the stack:

1. **Push** — Pushes the branch with `--force-with-lease` (safe force push)
2. **Create PR** — If no PR exists, creates one via GitHub API
3. **Update PR** — If PR exists, updates the description with stack navigation
4. **Stack Comment** — Adds/updates a comment showing the PR hierarchy

## Stack Comments

When you submit, rung adds a comment to each PR showing where it fits in the stack:

```markdown
- **#43** 👈
- **#42**
- **#41**
- `main`

---

_Managed by [rung](https://github.com/auswm85/rung)_
```

The `👈` indicates the current PR in the stack.

## PR Titles

By default, rung uses the first commit message as the PR title. You can override this:

```bash
# Set custom title for current branch
rung submit --title "Add user authentication system"
```

If you created branches with `rung create -m "message"`, that message becomes the PR title.

## Draft PRs

Create PRs as drafts to avoid triggering CI or notifying reviewers:

```bash
rung submit --draft
```

## Force Push

If the remote branch has diverged (e.g., someone else pushed), use `--force`:

```bash
rung submit --force
```

:::caution
Force pushing overwrites the remote branch. Only use this when you know your local branch should replace the remote.
:::

## Dry Run

Preview what would happen:

```bash
$ rung submit --dry-run

Would push:
  feat-add-user-model → origin/feat-add-user-model
  feat-add-user-api → origin/feat-add-user-api

Would create PRs:
  feat-add-user-model: "feat: add user model" (base: main)
  feat-add-user-api: "feat: add user API" (base: feat-add-user-model)
```

## JSON Output

```bash
$ rung submit --json
```

```json
{
  "submitted": [
    {
      "branch": "feat-add-user-model",
      "pushed": true,
      "pr": {
        "number": 41,
        "url": "https://github.com/org/repo/pull/41",
        "created": true
      }
    },
    {
      "branch": "feat-add-user-api",
      "pushed": true,
      "pr": {
        "number": 42,
        "url": "https://github.com/org/repo/pull/42",
        "created": false
      }
    }
  ]
}
```

## Notes

- Branches are pushed with `--force-with-lease` by default (safe force push)
- PRs have the correct base branch (parent in the stack)
- Stack comments are automatically updated when the stack changes
- You need GitHub authentication (via `gh` CLI or `GITHUB_TOKEN`)

## Related Commands

- [`status`](/commands/status/) — Check PR status
- [`sync`](/commands/sync/) — Sync before submitting
- [`merge`](/commands/merge/) — Merge approved PRs
