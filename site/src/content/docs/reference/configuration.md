---
title: Configuration
description: Rung configuration and authentication setup.
---

Rung requires GitHub authentication to create and manage pull requests.

## GitHub Authentication

Rung checks for GitHub authentication in this order:

1. `GITHUB_TOKEN` environment variable
2. GitHub CLI (`gh auth token`)

### Using GitHub CLI (Recommended)

The easiest way to authenticate is with the GitHub CLI:

```bash
gh auth login
```

Rung automatically uses the token from `gh auth token`.

### Using Environment Variable

Alternatively, set the `GITHUB_TOKEN` environment variable:

```bash
export GITHUB_TOKEN=ghp_xxxxxxxxxxxx
```

Required scopes:

- `repo` — Full control of private repositories
- `read:org` — Read org membership (for org repos)

## State Storage

Rung stores its state in `.git/rung/`:

| File              | Purpose                                   |
| ----------------- | ----------------------------------------- |
| `stack.json`      | Branch relationships and PR numbers       |
| `refs/`           | Backup refs for undo capability           |
| `sync_state.json` | In-progress sync state (during conflicts) |

This directory is local to your machine and not committed to git.

## Related

- [Troubleshooting](/reference/troubleshooting/) — Common issues and fixes
- [FAQ](/reference/faq/) — Frequently asked questions
