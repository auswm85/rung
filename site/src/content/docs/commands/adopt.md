---
title: adopt
description: Bring an existing Git branch into the rung stack.
since: "0.6.0"
---

Adopt an existing Git branch into the rung stack by establishing its parent relationship. Use this when you have branches created outside of rung that you want to manage as part of your stack.

## Usage

```bash
rung adopt [branch]
rung adopt [branch] --parent <parent>
rung adopt --parent <parent>
rung adopt --dry-run
```

## Aliases

- `rung ad` — shorthand for `rung adopt`

## Options

| Option              | Description                                                              |
| ------------------- | ------------------------------------------------------------------------ |
| `[branch]`          | Branch to adopt. Defaults to the current branch.                         |
| `-p, --parent`      | Parent branch for the adopted branch. Shows interactive picker if omitted. |
| `--dry-run`         | Preview what would happen without making changes.                        |

## Examples

### Adopt Current Branch

```bash
git checkout my-feature
rung adopt --parent main
```

Adopts the current branch (`my-feature`) with `main` as its parent.

### Adopt Specific Branch

```bash
rung adopt feature/api --parent main
```

Adopts `feature/api` into the stack with `main` as its parent.

### Interactive Parent Selection

```bash
rung adopt
```

When no `--parent` is specified, rung shows an interactive picker with available parents (base branch and any branches already in the stack).

### Preview with Dry Run

```bash
rung adopt feature/api --parent main --dry-run
```

Shows what would happen without modifying the stack.

## Workflow

### Bringing Legacy Branches into the Stack

```bash
# You have existing branches created with plain git
git branch
# * main
#   feature/auth
#   feature/api

# Adopt them into the stack in order
git checkout feature/auth
rung adopt --parent main

git checkout feature/api
rung adopt --parent feature/auth

# Check the stack
rung status
```

```
  Stack
  ──────────────────────────────────────────────────
  ●   feature/auth ← main
  ● ▶ feature/api ← feature/auth
  ──────────────────────────────────────────────────

  ● synced  ● needs sync  ● conflict
```

### Adopting a Branch Chain

When adopting multiple related branches, adopt them bottom-up (closest to main first):

```bash
# Wrong order - will fail
rung adopt feature/child --parent feature/parent
# Error: Parent 'feature/parent' is not in the stack

# Correct order
rung adopt feature/parent --parent main
rung adopt feature/child --parent feature/parent
```

## Validation

Rung validates that:

1. **Branch exists** — The branch must exist in Git
2. **Not already in stack** — Can't adopt a branch that's already managed
3. **Valid parent** — Parent must be either the base branch or already in the stack

## Notes

- The branch must already exist in Git (use `rung create` for new branches)
- Adopting doesn't modify the branch's commits or history
- After adopting, use `rung sync` to rebase if the parent has moved
- The base branch (usually `main`) is always a valid parent option

## Related Commands

- [`create`](/commands/create/) — Create a new branch in the stack
- [`status`](/commands/status/) — View the stack tree
- [`sync`](/commands/sync/) — Sync branches after adoption
