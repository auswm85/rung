---
title: Basic Workflow
description: Day-to-day patterns for working with stacked PRs using rung.
---

This guide covers the typical daily workflow for using rung effectively.

## Starting Your Session

Start by syncing your stack with the latest changes:

```bash
# Update main
git checkout main
git pull

# Sync your stack
rung sync

# Check status
rung status
```

If there are conflicts, see [Conflict Resolution](/guides/conflict-resolution/).

## Starting a New Feature

### 1. Start from main

```bash
git checkout main
git pull
```

### 2. Create your first branch

```bash
rung create -m "feat: add user model"
```

Make your changes, then continue building the stack.

### 3. Stack dependent changes

```bash
# Make changes to your code
# ...

# Create the next branch
rung create -m "feat: add user API endpoints"

# Continue working
# ...

rung create -m "feat: add user validation"
```

### 4. Check your progress

```bash
rung status
```

```
  Stack
  ──────────────────────────────────────────────────
  ●   feat-add-user-model ← main
  ●   feat-add-user-api-endpoints ← feat-add-user-model
  ● ▶ feat-add-user-validation ← feat-add-user-api-endpoints
  ──────────────────────────────────────────────────

  ● synced  ● needs sync  ● conflict
```

## Submitting for Review

When you're ready for review:

```bash
rung submit
```

This pushes all branches and creates PRs with correct base branches.

### Draft PRs

If you want feedback before the PR is "ready":

```bash
rung submit --draft
```

### Custom PR Titles

Override the commit-derived title for any branch:

```bash
git checkout feat-add-user-model
rung submit --title "Add User model with validation"
```

## During Review

### Addressing Feedback

When reviewers request changes:

```bash
# Navigate to the branch that needs changes
rung move                    # Interactive picker
# or
git checkout feat-add-user-api-endpoints

# Make your changes
git add .
git commit -m "Address review feedback"

# Push the update
rung submit
```

### Updating After Main Changes

If main moves forward while you're in review:

```bash
git checkout main
git pull
rung sync
rung submit    # Push the rebased branches
```

## Merging

Always merge from the **bottom of the stack** (closest to main):

```bash
# Go to the bottom branch
rung prv
rung prv  # Repeat until you're at the first branch

# Merge it
rung merge
```

Rung automatically:

- Merges the PR via GitHub
- Rebases child branches
- Updates child PR base branches
- Removes the merged branch

### Continue Merging

After the first merge:

```bash
# You're now on the next branch
rung merge   # Merge this one too

# Repeat as PRs are approved
```

## Navigation Shortcuts

### Quick Navigation

```bash
rung nxt     # Go to child branch
rung prv     # Go to parent branch
rung move    # Interactive picker
```

### See What's in a Branch

```bash
rung log     # Commits in current branch
```

## Common Patterns

### Pattern 1: Feature + Tests

```bash
rung create -m "feat: add authentication"
# ... implement feature ...

rung create -m "test: add auth unit tests"
# ... write tests ...

rung create -m "test: add auth integration tests"
```

### Pattern 2: Refactor + Feature

```bash
rung create -m "refactor: extract user service"
# ... do the refactor ...

rung create -m "feat: add user preferences"
# ... implement using the refactored code ...
```

### Pattern 3: Database Migration + Code

```bash
rung create -m "chore: add users table migration"
# ... add migration ...

rung create -m "feat: implement user model"
# ... implement model using new table ...
```

## Working with Multiple Stacks

You can have multiple independent stacks:

```bash
# Stack 1
git checkout main
rung create -m "feat: user authentication"
rung create -m "feat: user sessions"

# Stack 2 (start fresh from main)
git checkout main
rung create -m "feat: payment processing"
rung create -m "feat: payment webhooks"
```

Each stack is independent and can be synced/submitted separately.

## End of Day

Before ending your day:

```bash
# Make sure everything is pushed
rung submit

# Check status
rung status
```

Your stack is safely on GitHub, ready for tomorrow.

## Quick Reference

| Task            | Command                                      |
| --------------- | -------------------------------------------- |
| Start new stack | `git checkout main && rung create -m "..."`  |
| Add to stack    | `rung create -m "..."`                       |
| Check status    | `rung status`                                |
| Sync with main  | `git checkout main && git pull && rung sync` |
| Submit PRs      | `rung submit`                                |
| Merge bottom PR | `rung prv` (repeat) then `rung merge`        |
| Navigate        | `rung nxt`, `rung prv`, `rung move`          |

## Next Steps

- Learn how to handle [conflicts](/guides/conflict-resolution/)
