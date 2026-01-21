---
title: Troubleshooting
description: Common issues and how to resolve them.
---

This guide covers common issues you might encounter and how to resolve them.

## Diagnostic First Step

Always start with:

```bash
rung doctor
```

This checks stack integrity, git state, sync state, and GitHub connectivity.

## Common Issues

### "Not a rung repository"

**Symptom:**

```
Error: Not a rung repository. Run `rung init` first.
```

**Cause:** Rung hasn't been initialized in this repository.

**Solution:**

```bash
rung init
```

### "Current branch is not in stack"

**Symptom:**

```
Error: Current branch 'feature-x' is not part of a rung stack.
```

**Cause:** You're on a branch that wasn't created with `rung create`.

**Solutions:**

1. Create a new branch with rung:

   ```bash
   git checkout main
   rung create feature-x-new
   git cherry-pick <commits-from-feature-x>
   ```

2. Or work without rung on this branch.

### "Authentication failed"

**Symptom:**

```
Error: Could not authenticate with GitHub
```

**Cause:** No valid GitHub token found.

**Solutions:**

1. Use GitHub CLI:

   ```bash
   gh auth login
   ```

2. Or set environment variable:
   ```bash
   export GITHUB_TOKEN=ghp_your_token_here
   ```

### "Branch not found"

**Symptom:**

```
Error: Branch 'feature-old' not found
```

**Cause:** A branch in the stack was deleted outside of rung.

**Solution:**

1. Remove from stack manually:

   ```bash
   # Edit .git/rung/stack.json and remove the entry
   ```

2. Or recreate the branch:
   ```bash
   git checkout -b feature-old origin/feature-old
   ```

### "Sync state exists but not in rebase"

**Symptom:**

```
Error: Sync state exists but git is not in a rebase
```

**Cause:** A previous sync was interrupted abnormally.

**Solution:**

```bash
rm .git/rung/sync_state.json
rung sync
```

### "No backup found"

**Symptom:**

```
Error: No backup found. Nothing to undo.
```

**Cause:** No sync has been performed, or backups were deleted.

**Solution:**

Use git reflog to find previous commit:

```bash
git reflog show feature-branch
git reset --hard feature-branch@{1}
```

### "PR base branch mismatch"

**Symptom:**
PR has wrong base branch on GitHub.

**Cause:** Stack changed but PRs weren't updated.

**Solution:**

```bash
rung submit
```

This updates all PR base branches.

### "Force push rejected"

**Symptom:**

```
Error: Remote has changes. Use --force to overwrite.
```

**Cause:** Someone else pushed to your branch, or the remote diverged.

**Solutions:**

1. If your local is correct:

   ```bash
   rung submit --force
   ```

2. If you need to merge remote changes:
   ```bash
   git pull origin feature-branch
   # Resolve any conflicts
   rung submit
   ```

### "Circular dependency detected"

**Symptom:**

```
Error: Circular dependency: a -> b -> c -> a
```

**Cause:** Stack definition has a cycle.

**Solution:**

Edit `.git/rung/stack.json` to fix the parent relationships:

```bash
code .git/rung/stack.json
```

### "Working directory not clean"

**Symptom:**

```
Error: Working directory has uncommitted changes
```

**Cause:** You have uncommitted changes that would conflict with sync.

**Solution:**

```bash
# Option 1: Commit changes
git add . && git commit -m "WIP"

# Option 2: Stash changes
git stash
rung sync
git stash pop
```

## Git State Issues

### Detached HEAD

**Symptom:**

```
Error: Not on a branch (detached HEAD)
```

**Solution:**

```bash
git checkout feature-branch
```

### Rebase in Progress

**Symptom:**

```
Error: A rebase is already in progress
```

**Solution:**

```bash
# Continue the rebase
git rebase --continue

# Or abort it
git rebase --abort
```

### Merge in Progress

**Symptom:**

```
Error: A merge is in progress
```

**Solution:**

```bash
# Complete the merge
git add . && git commit

# Or abort it
git merge --abort
```

## Performance Issues

### Slow Status

**Symptom:** `rung status` takes several seconds.

**Possible causes:**

- Large repository
- Many branches in stack

**Solutions:**

- Keep stacks small (3-5 branches)
- Ensure your repository is not excessively large

### Slow Sync

**Symptom:** `rung sync` is slow.

**Possible causes:**

- Many commits to rebase
- Large files in commits
- Complex merge conflicts

**Solutions:**

- Keep commits small
- Sync frequently to reduce rebased commits
- Consider squashing old commits before syncing

## Recovery Procedures

### Complete Reset

If everything is broken, start fresh:

```bash
# Backup your stack definition
cp .git/rung/stack.json ~/stack-backup.json

# Remove rung state
rm -rf .git/rung

# Re-initialize
rung init

# Restore stack (if needed)
cp ~/stack-backup.json .git/rung/stack.json
```

### Recover Branch from Reflog

```bash
# Find the commit
git reflog show feature-branch

# Reset to it
git checkout feature-branch
git reset --hard feature-branch@{2}
```

### Recover from Bad Merge

If `rung merge` caused problems:

```bash
# Find pre-merge state
git reflog show feature-branch

# Reset branches
git checkout feature-branch
git reset --hard <commit-before-merge>
```

## Inspecting State

You can examine rung's state files directly:

```bash
# Stack definition
cat .git/rung/stack.json | jq .

# Backup refs
ls -la .git/rung/refs/

# Sync state (only exists during conflicts)
cat .git/rung/sync_state.json | jq .
```

## Getting Help

If you can't resolve an issue:

1. Check the [FAQ](/reference/faq/)
2. Search [GitHub Issues](https://github.com/auswm85/rung/issues)
3. Open a new issue with:
   - `rung doctor --json` output
   - Steps to reproduce
   - Expected vs actual behavior

## Related

- [Doctor command](/commands/doctor/) — Diagnostic tool
- [State management](/concepts/state-management/) — Understanding rung's data
- [FAQ](/reference/faq/) — Frequently asked questions
