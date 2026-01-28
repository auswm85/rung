---
title: absorb
description: Absorb staged changes into the appropriate commits in your stack.
since: "0.4.0"
---

Absorb staged changes into the appropriate commits in your stack. This analyzes staged hunks and automatically creates fixup commits targeting the commits that last modified those lines.

## Usage

```bash
rung absorb
rung absorb --dry-run
rung absorb --base main
```

## Aliases

- `rung ab` — shorthand for `rung absorb`

## Options

| Option                | Description                                                              |
| --------------------- | ------------------------------------------------------------------------ |
| `--dry-run`           | Show what would be absorbed without making changes                       |
| `-b, --base <branch>` | Base branch to determine rebaseable range (auto-detected from GitHub by default) |

## How It Works

When you run `rung absorb`:

1. **Parse** — Parses your staged diff into hunks
2. **Blame** — Uses `git blame` to find which commit last modified each hunk's lines
3. **Validate** — Checks the target commit is in your stack (not already on the base branch)
4. **Fixup** — Creates `fixup!` commits targeting the appropriate commits

### Example

```bash
# Make some tweaks to existing code
vim src/auth.rs

# Stage the changes
git add -p

# Preview what would be absorbed
$ rung absorb --dry-run
→ 2 hunk(s) will be absorbed:
  a1b2c3d4 Add authentication middleware (2 hunk(s))
    → src/auth.rs
    → src/auth.rs
→ Dry run - no changes made

# Actually absorb the changes
$ rung absorb
→ 2 hunk(s) will be absorbed:
  a1b2c3d4 Add authentication middleware (2 hunk(s))
    → src/auth.rs
    → src/auth.rs
✓ Created 1 fixup commit(s)
→ Run `git rebase -i --autosquash` to apply the fixups
```

### Applying Fixups

After absorb creates fixup commits, apply them with an interactive rebase:

```bash
# Use the same base branch as the absorb command
git rebase -i --autosquash main
```

Git will automatically reorder the fixup commits to follow their targets.

## Workflow Example

```bash
# You're working on a feature branch with several commits
$ rung log
a1b2c3d    Add user authentication     you
e4f5g6h    Add auth middleware         you
i7j8k9l    Add auth tests              you

# You notice a small bug in the middleware
vim src/middleware.rs

# Stage just that fix
git add -p src/middleware.rs

# Absorb it into the right commit
$ rung absorb
→ 1 hunk(s) will be absorbed:
  e4f5g6h Add auth middleware (1 hunk(s))
    → src/middleware.rs
✓ Created 1 fixup commit(s)
→ Run `git rebase -i --autosquash` to apply the fixups

# Now apply the fixup
git rebase -i --autosquash main
```

## Unmapped Hunks

Some hunks cannot be absorbed. Rung reports these with reasons:

```bash
$ rung absorb
! 2 hunk(s) could not be absorbed:
  src/new_file.rs (new file)
  src/mixed.rs (multiple commits touched these lines)

→ 1 hunk(s) will be absorbed:
  a1b2c3d4 Fix validation (1 hunk(s))
    → src/auth.rs
```

### Reasons for Unmapped Hunks

| Reason                          | Description                                              |
| ------------------------------- | -------------------------------------------------------- |
| new file                        | New files have no blame history                          |
| multiple commits touched these lines | The changed lines were last modified by different commits |
| target commit not in stack      | The blamed commit is not between base and HEAD           |
| target commit already on base branch | The blamed commit is already merged                   |
| blame error                     | Git blame failed for this file/range                     |

## Limitations

- **New files** cannot be absorbed (no blame history exists)
- **Multi-commit hunks** — If a hunk touches lines from multiple commits, it cannot be automatically assigned
- **Single target only** — All staged hunks must target the same commit; stage fewer changes if they target different commits
- **Rebaseable range** — Only works with commits between the base branch and HEAD

## Base Branch Detection

By default, rung queries GitHub to detect the default branch. You can override this:

```bash
# Explicit base branch
rung absorb --base develop

# Required if GitHub auth is unavailable
rung absorb --base main
```

Use the same base branch when running `git rebase --autosquash`.

## Notes

- Stage changes with `git add -p` for fine-grained control over what gets absorbed
- Use `--dry-run` to preview before creating fixup commits
- The base branch for absorb and the subsequent rebase should match
- Works best with small, focused fixes that clearly belong to specific commits

## Related Commands

- [`log`](/commands/log/) — See commits in the current branch
- [`sync`](/commands/sync/) — Sync branches after rebasing
- [`submit`](/commands/submit/) — Push after applying fixups
