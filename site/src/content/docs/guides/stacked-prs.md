---
title: What are Stacked PRs?
description: Understanding stacked pull requests and why they improve your development workflow.
---

Stacked PRs (pull requests) are a development pattern where you break a large feature into a series of smaller, dependent changes that build on each other.

## The Problem with Large PRs

Traditional development often results in large pull requests:

```
main ─────────────────────────────────────────────────────●
                                                          │
                                                          └── my-feature (500 lines changed)
```

Large PRs have several problems:

- **Hard to review** — Reviewers struggle to understand 500+ lines of changes
- **Slow feedback** — Reviews take longer, blocking your work
- **Risky merges** — More code means more chances for bugs
- **Context switching** — You can't work on other things while waiting

## The Solution: Stacked PRs

Instead of one large PR, create a stack of smaller, focused PRs:

```
main ─────●
          │
          └── add-user-model (PR #1: 80 lines)
                │
                └── add-user-api (PR #2: 120 lines)
                      │
                      └── add-user-tests (PR #3: 100 lines)
```

Each PR:

- Does **one thing well**
- **Builds on** the previous PR
- Can be **reviewed independently**
- Is **merged in order** (bottom to top)

## Benefits

### 1. Faster Reviews

Small PRs get reviewed faster. A 100-line PR is easy to understand and approve. A 500-line PR sits in the queue.

### 2. Better Feedback

Reviewers can give more thoughtful feedback on focused changes. They understand the context completely.

### 3. Safer Merges

Each PR is small and tested. If something breaks, you know exactly where to look.

### 4. Parallel Work

While PR #1 is being reviewed, you can continue working on PR #2 and #3. Your stack keeps growing, and reviews flow in as they're ready.

### 5. Clear History

Each merged PR represents a logical unit of work. Your git history tells a story.

## The Challenge

Stacked PRs are powerful but have a manual overhead:

1. When `main` changes, you need to rebase all your branches
2. When PR #1 is approved, you need to update PR #2's base branch
3. Keeping track of which branches depend on which is error-prone

This is where **rung** helps.

## How Rung Helps

Rung automates the tedious parts of stacked PRs:

### Branch Tracking

Rung remembers which branches depend on which:

```bash
$ rung status

  Stack
  ──────────────────────────────────────────────────
  ●   add-user-model #41 ← main
  ●   add-user-api #42 ← add-user-model
  ● ▶ add-user-tests #43 ← add-user-api
  ──────────────────────────────────────────────────

  ● synced  ● needs sync  ● conflict
```

### Automatic Rebasing

When `main` changes, one command updates everything:

```bash
$ rung sync
✓ Synced add-user-model (rebased 3 commits)
✓ Synced add-user-api (rebased 2 commits)
✓ Synced add-user-tests (rebased 1 commit)
```

### PR Management

Submit all your PRs with correct base branches:

```bash
$ rung submit
✓ Created PR #41: add-user-model (base: main)
✓ Created PR #42: add-user-api (base: add-user-model)
✓ Created PR #43: add-user-tests (base: add-user-api)
```

### Merge Handling

When you merge, rung updates the remaining stack:

```bash
$ rung merge  # On add-user-model
✓ Merged PR #41
✓ Rebased add-user-api onto main
✓ Updated PR #42 base to main
```

## When to Use Stacked PRs

Stacked PRs work best for:

- **Large features** that can be broken down logically
- **Refactoring + feature** work (refactor first, then feature)
- **Database migrations** (migration PR, then code PR)
- **API changes** (backend PR, then frontend PR)

## When Not to Use Stacked PRs

Stacked PRs add complexity. Skip them for:

- **Simple bug fixes** — Just make a single PR
- **Truly independent changes** — Use separate branches instead
- **Tiny features** — If it's already small, don't split it

## Next Steps

- Try the [Quick Start](/getting-started/quickstart/) to create your first stack
- Set up your [daily workflow](/guides/basic-workflow/)
- Learn more about stacked PRs at [stacking.dev](https://www.stacking.dev/)
