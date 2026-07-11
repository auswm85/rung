---
title: Configuration
description: Rung configuration and authentication setup.
---

Rung needs forge authentication to create and manage pull requests (GitHub) or merge requests (GitLab). Rung detects the forge from your `origin` remote and uses the matching credentials.

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

## GitLab Authentication

:::note
GitLab support (including self-hosted instances) was added in rung 1.0.0.
:::

Rung checks for GitLab authentication in this order:

1. `GITLAB_TOKEN` environment variable
2. GitLab CLI (`glab`)

### Using GitLab CLI (Recommended)

Authenticate with the GitLab CLI:

```bash
glab auth login
```

Rung reads the stored token via `glab config get token` for the relevant host (gitlab.com or your self-hosted instance).

### Using Environment Variable

Alternatively, set the `GITLAB_TOKEN` environment variable:

```bash
export GITLAB_TOKEN=glpat_xxxxxxxxxxxx
```

Create a personal access token with the `api` scope. `GITLAB_TOKEN` takes precedence over the `glab` CLI and is used regardless of host.

## Configuration File

Rung stores repository settings in `.git/rung/config.toml`. All sections are optional — the file is created with sensible defaults on `rung init`.

```toml
[github]
# Custom API URL for GitHub Enterprise (currently reserved; github.com only for now).
# api_url = "https://github.example.com/api/v3"

[gitlab]
# Self-hosted GitLab instances. The instance host is derived from this URL, which
# lets rung recognize remotes on that host (e.g. git@gitlab.example.com:group/project.git)
# that cannot be inferred from the URL alone. Omit for gitlab.com.
# api_url = "https://gitlab.example.com/api/v4"
```

### Self-hosted GitLab

`gitlab.com` and `github.com` remotes are detected automatically. A **self-hosted GitLab** instance lives on a custom hostname that rung cannot infer from the remote URL, so you must point rung at its API:

```toml
[gitlab]
api_url = "https://gitlab.example.com/api/v4"
```

With this set, rung recognizes `origin` remotes on `gitlab.example.com` (HTTPS and SSH, including nested groups), talks to that instance's API, and reads the `glab` credential for that host.

## State Storage

Rung stores its state in `.git/rung/`:

| File              | Purpose                                   |
| ----------------- | ----------------------------------------- |
| `stack.json`      | Branch relationships and PR/MR numbers    |
| `refs/`           | Backup refs for undo capability           |
| `sync_state.json` | In-progress sync state (during conflicts) |

This directory is local to your machine and not committed to git.

## Related

- [Troubleshooting](/reference/troubleshooting/) — Common issues and fixes
- [FAQ](/reference/faq/) — Frequently asked questions
