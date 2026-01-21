---
title: Quick Start
description: Create your first stacked PR workflow in 5 minutes.
---

This guide walks you through creating your first stack of dependent branches and submitting them as pull requests.

## Prerequisites

- [rung installed](/getting-started/installation/)
- A git repository with a GitHub remote
- GitHub CLI authenticated (`gh auth login`) or `GITHUB_TOKEN` set

## Initialize rung

Start in any git repository:

```bash
cd your-repo
git checkout main
rung init
```

This creates a `.git/rung/` directory to store stack state.

## Create Your First Stack

Let's build a simple feature with three dependent branches.

### Step 1: Create the Base Branch

```bash
# Create a branch with a commit message
rung create -m "feat: add user model"
```

This command:

1. Creates a new branch named `feat-add-user-model` (derived from the message)
2. Stages all changes
3. Creates a commit with the message "feat: add user model"

:::tip
The `-m` flag is powerful: it derives the branch name, stages changes, and commits—all in one step.
:::

### Step 2: Make Some Changes

Edit your files to add the user model, then check the status:

```bash
rung status
```

```
  Stack
  ──────────────────────────────────────────────────
  ● ▶ feat-add-user-model ← main
  ──────────────────────────────────────────────────

  ● synced  ● needs sync  ● conflict
```

The `▶` indicator shows your current branch.

### Step 3: Stack Another Branch

Now create a dependent branch for the API:

```bash
# Make your changes first, then:
rung create -m "feat: add user API"
```

```bash
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

### Step 4: Add One More

```bash
# Make test changes, then:
rung create -m "feat: add user tests"
```

Your stack now looks like:

```
  Stack
  ──────────────────────────────────────────────────
  ●   feat-add-user-model ← main
  ●   feat-add-user-api ← feat-add-user-model
  ● ▶ feat-add-user-tests ← feat-add-user-api
  ──────────────────────────────────────────────────

  ● synced  ● needs sync  ● conflict
```

## Submit Your Stack

Push all branches and create PRs:

```bash
rung submit
```

Output:

```
✓ Pushed feat-add-user-model
✓ Created PR #41: feat: add user model
  https://github.com/you/repo/pull/41

✓ Pushed feat-add-user-api
✓ Created PR #42: feat: add user API (base: feat-add-user-model)
  https://github.com/you/repo/pull/42

✓ Pushed feat-add-user-tests
✓ Created PR #43: feat: add user tests (base: feat-add-user-api)
  https://github.com/you/repo/pull/43
```

Each PR automatically:

- Has the correct base branch (parent in the stack)
- Includes a stack comment showing the hierarchy

Check the status with PR numbers:

```bash
rung status
```

```
  Stack
  ──────────────────────────────────────────────────
  ●   feat-add-user-model #41 ← main
  ●   feat-add-user-api #42 ← feat-add-user-model
  ● ▶ feat-add-user-tests #43 ← feat-add-user-api
  ──────────────────────────────────────────────────

  ● synced  ● needs sync  ● conflict
```

## Navigate Your Stack

Move between branches easily:

```bash
rung prv           # Go to parent (feat-add-user-api)
rung prv           # Go to parent (feat-add-user-model)
rung nxt           # Go to child (feat-add-user-api)
rung move          # Interactive picker
```

## Sync When Main Changes

When someone merges to main (or you pull new changes):

```bash
git checkout main
git pull
rung sync
```

```
✓ Synced feat-add-user-model (rebased 3 commits)
✓ Synced feat-add-user-api (rebased 2 commits)
✓ Synced feat-add-user-tests (rebased 1 commit)
```

All your branches are automatically rebased to include the new main changes.

## Merge Your PRs

Once PR #41 is approved, merge it from the bottom up:

```bash
git checkout feat-add-user-model
rung merge
```

This:

1. Merges the PR via GitHub API
2. Rebases `feat-add-user-api` onto `main`
3. Updates the PR base branch on GitHub
4. Removes the merged branch from the stack

Repeat for each PR as they're approved.

## Summary

| Command                 | What it does                          |
| ----------------------- | ------------------------------------- |
| `rung init`             | Initialize rung in a repository       |
| `rung create -m "msg"`  | Create branch, stage, and commit      |
| `rung status`           | Show stack tree with PR status        |
| `rung submit`           | Push branches and create/update PRs   |
| `rung sync`             | Rebase all branches when parent moves |
| `rung merge`            | Merge PR and update stack             |
| `rung prv` / `rung nxt` | Navigate the stack                    |

## Next Steps

- Learn about [all commands](/commands/) in detail
- Understand [how stacked PRs work](/concepts/stacked-prs/)
- Set up your [daily workflow](/guides/basic-workflow/)
