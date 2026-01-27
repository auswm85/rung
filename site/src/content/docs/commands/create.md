---
title: create
description: Create a new branch with the current branch as its parent.
---

Create a new branch in the stack with the current branch as its parent. This establishes the branch relationship that rung uses for syncing and PR management.

## Usage

```bash
rung create [name]
rung create -m <message>
rung create [name] -m <message>
rung create [name] --dry-run
```

## Aliases

- `rung c` — shorthand for `rung create`

## Options

| Option                    | Description                                                                                                                |
| ------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `-m, --message <message>` | Commit message. Stages all changes and creates a commit. If no branch name is provided, derives the name from the message. |
| `--dry-run`               | Preview what would happen without making changes.                                                                          |

## Examples

### Explicit Branch Name

```bash
rung create feature/authentication
```

Creates a new branch called `feature/authentication` with the current branch as its parent.

### Derive Name from Message

```bash
rung create -m "feat: add user authentication"
```

This powerful shorthand:

1. Derives the branch name from the message → `feat-add-user-authentication`
2. Creates and checks out the new branch
3. Stages all changes (`git add -A`)
4. Commits with the provided message

### Explicit Name with Commit

```bash
rung create my-feature -m "feat: implement my feature"
```

Uses the explicit name `my-feature` instead of deriving it from the message.

## Branch Name Derivation

When using `-m` without an explicit name, rung converts the message to a branch name by:

1. Converting to lowercase
2. Replacing spaces and special characters with hyphens
3. Removing duplicate hyphens

| Message                | Derived Branch Name  |
| ---------------------- | -------------------- |
| `feat: add auth`       | `feat-add-auth`      |
| `Fix login redirect`   | `fix-login-redirect` |
| `Add user model (WIP)` | `add-user-model-wip` |

## Workflow

```bash
# Starting from main
git checkout main

# Create first branch
rung create -m "feat: add user model"
# → Now on feat-add-user-model

# Make more changes, create dependent branch
rung create -m "feat: add user API"
# → Now on feat-add-user-api, with feat-add-user-model as parent

# Check the stack
rung status
```

```
  Stack
  ──────────────────────────────────────────────────
  ●   feat-add-user-model ← main
  ● ▶ feat-add-user-api ← feat-add-user-model
  ──────────────────────────────────────────────────

  ● synced  ● needs sync  ● conflict
```

## Notes

- The current branch becomes the parent of the new branch
- If using `-m`, all staged and unstaged changes are committed
- The commit message is used as the PR title when running `rung submit`
- You must be on a branch (not detached HEAD) to create a new branch

## Related Commands

- [`status`](/commands/status/) — View the stack tree
- [`submit`](/commands/submit/) — Push and create PRs
- [`nxt`](/commands/navigation/) / [`prv`](/commands/navigation/) — Navigate the stack
