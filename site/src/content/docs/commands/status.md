---
title: status
description: Display the current stack as a tree view with sync state and PR status.
---

Display the current stack as a tree view showing branch relationships, sync state, and PR status.

## Usage

```bash
rung status
rung status --json
```

## Aliases

- `rung st` — shorthand for `rung status`

## Example Output

```bash
$ rung status

  Stack
  ──────────────────────────────────────────────────
  ●   feat-add-user-model #41 ← main
  ● ▶ feat-add-user-api #42 ← feat-add-user-model
  ●   feat-add-user-tests #43 ← feat-add-user-api
  ──────────────────────────────────────────────────

  ● synced  ● needs sync  ● conflict
```

### Legend

| Symbol | Meaning                                          |
| ------ | ------------------------------------------------ |
| `▶`    | Current branch (appears before branch name)      |
| `●`    | Green: synced, Yellow: needs sync, Red: conflict |
| `#N`   | PR number                                        |
| `←`    | Shows parent branch                              |

## JSON Output

For integration with other tools:

```bash
$ rung status --json
```

```json
{
  "branches": [
    {
      "name": "feat-add-user-model",
      "parent": "main",
      "state": "synced",
      "pr": 41,
      "is_current": false
    },
    {
      "name": "feat-add-user-api",
      "parent": "feat-add-user-model",
      "state": { "diverged": { "commits_behind": 2 } },
      "pr": 42,
      "is_current": true
    }
  ],
  "current": "feat-add-user-api"
}
```

## Branch States

| State      | Description                    |
| ---------- | ------------------------------ |
| `synced`   | Up-to-date with parent branch  |
| `diverged` | Parent has moved, needs rebase |
| `conflict` | Rebase resulted in conflicts   |
| `detached` | Orphaned (parent deleted)      |

## Notes

- PR numbers are stored locally in `.git/rung/stack.json`
- Use `--json` for CI/CD integration and scripting
- The `is_current` field is only included when `true`

## Related Commands

- [`sync`](/commands/sync/) — Rebase diverged branches
- [`submit`](/commands/submit/) — Push and update PRs
- [`doctor`](/commands/doctor/) — Diagnose issues
