---
title: Conflict Resolution
description: How to handle rebase conflicts when syncing your stack.
---

Conflicts happen when changes in your stack overlap with changes in the parent branch. This guide walks you through resolving them.

## Understanding Conflicts

When you run `rung sync` and a conflict occurs:

```bash
$ rung sync
✓ Synced feat-add-user-model
✗ Conflict in feat-add-user-api

Conflict in: src/api/users.rs

Resolve the conflict, then run:
  rung sync --continue

Or abort and restore:
  rung sync --abort
```

Rung pauses at the conflicting branch and waits for you to resolve.

## The Resolution Process

### Step 1: Identify the Conflict

Look at the conflicting files:

```bash
git status
```

```
Unmerged paths:
  both modified:   src/api/users.rs
```

### Step 2: Open the File

Open the conflicting file. You'll see conflict markers:

```rust
fn get_user(id: u64) -> User {
<<<<<<< HEAD
    // Code from the new parent (main or parent branch)
    database.find_user(id)
=======
    // Your code from this branch
    let user = database.get_user_by_id(id);
    cache.store(user.clone());
    user
>>>>>>> feat-add-user-api
}
```

### Step 3: Resolve the Conflict

Edit the file to combine both changes correctly:

```rust
fn get_user(id: u64) -> User {
    // Combined: use new API but keep caching
    let user = database.find_user(id);
    cache.store(user.clone());
    user
}
```

Remove all conflict markers (`<<<<<<<`, `=======`, `>>>>>>>`).

### Step 4: Stage the Resolution

```bash
git add src/api/users.rs
```

### Step 5: Continue the Sync

```bash
rung sync --continue
```

Rung continues rebasing the remaining branches.

## If More Conflicts Occur

The sync might pause again at another branch:

```bash
$ rung sync --continue
✓ Synced feat-add-user-api
✗ Conflict in feat-add-user-tests

Conflict in: tests/user_test.rs
```

Repeat the resolution process for each conflict.

## Aborting

If you want to give up and restore everything:

```bash
rung sync --abort
```

This restores all branches to their pre-sync state using the backup refs.

## Common Conflict Patterns

### Import Conflicts

When both sides add imports:

```rust
<<<<<<< HEAD
use crate::services::UserService;
use crate::models::User;
=======
use crate::models::User;
use crate::cache::UserCache;
>>>>>>> feat-add-user-api
```

**Resolution:** Keep all unique imports:

```rust
use crate::cache::UserCache;
use crate::models::User;
use crate::services::UserService;
```

### Function Signature Changes

When both sides modify the same function:

```rust
<<<<<<< HEAD
fn create_user(name: &str, email: &str) -> User {
=======
fn create_user(data: UserData) -> Result<User, Error> {
>>>>>>> feat-add-user-api
```

**Resolution:** Decide which signature is correct and update the body accordingly.

### File Moved/Deleted

If the parent deleted or moved a file you modified:

```
CONFLICT (modify/delete): src/old_file.rs deleted in HEAD
```

**Resolution:** Either recreate your changes in the new location or `git rm` the file if the changes are no longer needed.

## Prevention Strategies

### Keep Stacks Small

Smaller stacks = fewer conflicts. Aim for 3-5 branches per stack.

### Sync Frequently

Don't let your stack drift too far from main:

```bash
# Do this daily
git checkout main && git pull && rung sync
```

### Avoid Overlapping Work

If two people are modifying the same files, coordinate:

- Split work differently
- Use separate stacks
- Merge one stack before starting another

## Conflict During Merge

Sometimes conflicts happen during `rung merge` when rebasing child branches:

```bash
$ rung merge
✓ Merged PR #41
✗ Conflict rebasing feat-add-user-api onto main

Resolve the conflict, then run:
  rung sync --continue
```

The resolution process is the same.

## Using VS Code

If you use VS Code, it has built-in merge conflict resolution:

1. Open the conflicting file
2. Click "Accept Current Change", "Accept Incoming Change", or "Accept Both Changes"
3. Or manually edit the combined result
4. Save the file

Then:

```bash
git add .
rung sync --continue
```

## Using Git Mergetool

You can use git's built-in mergetool:

```bash
git mergetool
```

This opens your configured merge tool. After resolving:

```bash
rung sync --continue
```

## Checking for Problems

After resolving:

```bash
# Make sure nothing is broken
cargo build    # or your build command
cargo test     # run tests

# Then continue
rung sync --continue
```

## Troubleshooting

### "Not in a rebase"

If you see this error:

```bash
$ rung sync --continue
Error: Not currently in a rebase
```

The rebase may have been completed or aborted outside of rung. Run:

```bash
rung status
```

To see the current state.

### Lost My Changes

If you accidentally lost changes during conflict resolution:

```bash
# Find your old commit
git reflog show feat-add-user-api

# Cherry-pick or reset to recover
git cherry-pick abc123
```

### Sync State Stuck

If the sync state file exists but you're not in a rebase:

```bash
rm .git/rung/sync_state.json
```

Then run `rung status` to check the current state.

## Related

- [Sync command](/commands/sync/) — Full sync documentation
- [Undo command](/commands/undo/) — Restore from backup
- [Basic workflow](/guides/basic-workflow/) — Daily patterns
