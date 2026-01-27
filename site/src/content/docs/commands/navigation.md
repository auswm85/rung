---
title: Navigation Commands
description: Move between branches in your stack with nxt, prv, and move.
since: "0.1.0"
---

Rung provides several commands for navigating within your stack.

## nxt

Navigate to the next (child) branch in the stack.

### Usage

```bash
rung nxt
```

### Alias

- `rung n` — shorthand for `rung nxt`

### Example

```bash
# Starting on feat-add-user-model
$ rung nxt
Switched to feat-add-user-api
```

If the current branch has multiple children, `nxt` switches to the first child.

### When There's No Child

```bash
$ rung nxt
No child branch found
```

## prv

Navigate to the previous (parent) branch in the stack.

### Usage

```bash
rung prv
```

### Alias

- `rung p` — shorthand for `rung prv`

### Example

```bash
# Starting on feat-add-user-tests
$ rung prv
Switched to feat-add-user-api

$ rung prv
Switched to feat-add-user-model

$ rung prv
Switched to main
```

### When at Root

```bash
# On main
$ rung prv
Already at root of stack
```

## move

Interactive branch picker for quick navigation. Opens a TUI list to select and jump to any branch in the stack.

### Usage

```bash
rung move
```

### Alias

- `rung mv` — shorthand for `rung move`

### Example

```bash
$ rung move
? Jump to branch:
  feat/auth #41
> feat/api #42 ◀
  feat/ui
```

Use arrow keys to navigate, Enter to select.

### Features

- Shows all branches in the stack
- Highlights current branch with `◀`
- Displays PR numbers when available
- Fuzzy search as you type

## Navigation Workflow

```bash
# Start at the tip of your stack
$ rung status

  Stack
  ──────────────────────────────────────────────────
  ●   feat-add-user-model #41 ← main
  ●   feat-add-user-api #42 ← feat-add-user-model
  ● ▶ feat-add-user-tests #43 ← feat-add-user-api
  ──────────────────────────────────────────────────

  ● synced  ● needs sync  ● conflict

# Go back to the beginning
$ rung prv
Switched to feat-add-user-api

$ rung prv
Switched to feat-add-user-model

# Check what's in this branch
$ rung log
abc123    Add user model    you

# Jump to any branch
$ rung move
? Jump to branch:
> feat-add-user-tests #43

# Forward navigation
$ rung nxt
Switched to feat-add-user-api
```

## Related Commands

- [`status`](/commands/status/) — View the full stack tree
- [`create`](/commands/create/) — Create new branches
