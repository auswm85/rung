---
title: split
description: Split a branch into multiple stacked branches for smaller, focused PRs.
since: "0.7.0"
---

Split a branch into multiple stacked branches. This is useful when you've accumulated multiple logical changes in a single branch and want to break them into separate, focused PRs for easier review.

## Usage

```bash
rung split
rung split feature/big-change
rung split --dry-run
rung split --abort
```

## Aliases

- `rung sp` — shorthand for `rung split`

## Options

| Option      | Description                                     |
| ----------- | ----------------------------------------------- |
| `--dry-run` | Show what would be done without making changes  |
| `--abort`   | Abort the current split and restore from backup |

## How It Works

When you run `rung split`:

1. **Analyze** — Lists all commits between the parent branch and the current branch
2. **Select** — Interactive UI lets you select commits to split off into new branches
3. **Name** — Prompts for branch names (suggests names based on commit messages)
4. **Create** — Creates new branches at each split point
5. **Update Stack** — Reparents the original branch onto the last created branch

### Example

Split a branch with multiple commits into separate stacked branches:

```bash
$ rung split
Commits in feat-big-change (oldest first):

  1. a1b2c3d4 Add user model
  2. e5f6g7h8 Add user API endpoints
  3. i9j0k1l2 Add user tests

Select commits to split (space to toggle, enter to confirm):
  [x] a1b2c3d4 Add user model
  [x] e5f6g7h8 Add user API endpoints
  [ ] i9j0k1l2 Add user tests

Branch name for "Add user model" [feat-add-user-model]:
Branch name for "Add user API endpoints" [feat-add-user-api]:

✓ Created branch feat-add-user-model at a1b2c3d4
✓ Created branch feat-add-user-api at e5f6g7h8
✓ Reparented feat-big-change onto feat-add-user-api

Stack after split:
  main
  └── feat-add-user-model
      └── feat-add-user-api
          └── feat-big-change
```

### Dry Run

Preview the split without making changes:

```bash
$ rung split --dry-run
Commits in feat-big-change (oldest first):

  1. a1b2c3d4 Add user model
  2. e5f6g7h8 Add user API endpoints

Would create:
  main
  └── feat-add-user-model (at a1b2c3d4)
      └── feat-add-user-api (at e5f6g7h8)
          └── feat-big-change

Dry run - no changes made
```

## Aborting a Split

If something goes wrong during the split, you can restore your branches to their pre-split state:

```bash
rung split --abort
```

This restores the original branch and removes any partially created branches.

## Split State

During a split operation, rung writes state to `.git/rung/split_state.json`:

```json
{
  "started_at": "2024-01-15T10:30:00Z",
  "backup_id": "1704067200",
  "source_branch": "feat-big-change",
  "parent_branch": "main",
  "original_branch": "feat-big-change",
  "split_points": [
    {
      "commit_sha": "a1b2c3d4...",
      "branch_name": "feat-add-user-model"
    }
  ],
  "current_index": 0,
  "completed": [],
  "stack_updated": false
}
```

This allows `--abort` to restore the original state if needed.

## Branch Naming

Rung suggests branch names based on commit messages:

- Extracts the first few words from the commit summary
- Converts to kebab-case (lowercase with hyphens)
- Falls back to `<source-branch>-part-N` if the message can't be parsed

You can accept the suggestion or type a custom name for each branch.

## Workflow Example

```bash
# You've been working on a feature and it grew too large
$ rung log
a1b2c3d    Add user model                  you
e5f6g7h    Add user API endpoints          you
i9j0k1l    Add user authentication         you
m2n3o4p    Add user tests                  you

# Split into focused branches for review
$ rung split
# Select commits, provide names

# After splitting:
$ rung status
main
└── feat-add-user-model (#101)
    └── feat-add-user-api (#102)
        └── feat-add-user-auth (#103)
            └── feat-add-user-tests

# Submit PRs for the new branches
$ rung submit
```

## Notes

- The split operation creates a linear chain of branches from the selected commits
- Each split point becomes a new branch that is a parent of the next
- The original branch is reparented onto the last created branch
- Always commit or stash your changes before splitting
- Use `--dry-run` to preview the resulting stack structure
- Backup refs are stored in `.git/rung/backups/` for undo capability

## Limitations

- Cannot split a root branch (must have a parent in the stack)
- The branch being split must have at least one commit above its parent
- Split points must be selected in order (oldest to newest)

## Related Commands

- [`create`](/commands/create/) — Create a new branch in the stack
- [`restack`](/commands/restack/) — Move a branch to a different parent
- [`log`](/commands/log/) — See commits in the current branch
- [`undo`](/commands/undo/) — Restore from last backup
