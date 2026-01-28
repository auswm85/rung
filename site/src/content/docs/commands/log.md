---
title: log
description: Show commits on the current branch between the parent branch and HEAD.
since: "0.2.0"
---

Show commits on the current branch—specifically, commits between the parent branch and HEAD. This helps you see exactly what changes are in the current stack branch, excluding commits from parent branches.

## Usage

```bash
rung log
rung log --json
```

## Options

| Option   | Description                                                      |
| -------- | ---------------------------------------------------------------- |
| `--json` | Output as JSON (includes branch name, parent, and commit details) |

## Example

```bash
$ rung log

a1b2c3d    Add user authentication     alice
e4f5g6h    Fix login redirect          alice
```

## Output Format

```
<short-sha>    <commit-message>    <author>
```

## JSON Output

```bash
$ rung log --json
```

```json
{
  "commits": [
    { "hash": "a1b2c3d", "message": "Add user authentication", "author": "alice" },
    { "hash": "e4f5g6h", "message": "Fix login redirect", "author": "alice" }
  ],
  "branch": "feat-auth",
  "parent": "main"
}
```

## When There Are No Commits

```bash
$ rung log
! Current branch has no commits
```

This happens when the current branch points to the same commit as its parent.

## Related Commands

- [`status`](/commands/status/) — View the full stack tree
- [`absorb`](/commands/absorb/) — Absorb staged changes into commits
- [`nxt`](/commands/navigation/) / [`prv`](/commands/navigation/) — Navigate the stack
