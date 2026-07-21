---
title: Installation
description: How to install rung on macOS, Linux, and Windows.
---

Choose the installation method that works best for your system.

## Quick Install (Recommended)

The fastest way to install rung on macOS or Linux:

```bash
curl -sSf https://rungstack.com/install.sh | sh
```

This script automatically detects your platform and installs the latest version.

### Options

Custom install directory (defaults to `/usr/local/bin` or `~/.local/bin`):

```bash
INSTALL_DIR=~/bin curl -sSf https://rungstack.com/install.sh | sh
```

### Windows

Download the `.zip` from [releases](https://github.com/auswm85/rung/releases) and add to your PATH.

## Homebrew (macOS/Linux)

```bash
brew tap auswm85/rung
brew trust --formula auswm85/rung/rung
brew install rung
```

Homebrew requires third-party tap formulae to be trusted before it will load them.

Installed from the tap before it moved to
[auswm85/homebrew-rung](https://github.com/auswm85/homebrew-rung)? Untap the old one first:

```bash
brew uninstall rung
brew untap auswm85/rung
```

Then follow the steps above.

## From crates.io

If you have Rust installed:

```bash
cargo install rung-cli
```

## With cargo-binstall

Faster installation without compilation:

```bash
cargo binstall rung-cli
```

## From Source

Clone and build from the repository:

```bash
git clone https://github.com/auswm85/rung
cd rung
cargo install --path crates/rung-cli
```

## Verify Installation

After installation, verify rung is available:

```bash
rung --version
```

You should see output like:

```
rung 0.1.0
```

## Requirements

- **Git 2.x** — rung uses git2-rs for git operations
- A forge credential for your remote:
  - **GitHub** — GitHub CLI (`gh`) authenticated, or `GITHUB_TOKEN` environment variable
  - **GitLab** — GitLab CLI (`glab`) authenticated, or `GITLAB_TOKEN` environment variable

### Setting up GitHub Authentication

rung needs GitHub access to create and manage pull requests. You have two options:

#### Option 1: GitHub CLI (Recommended)

Install and authenticate the GitHub CLI:

```bash
# Install gh (if not already installed)
brew install gh        # macOS
apt install gh         # Ubuntu/Debian
winget install gh      # Windows

# Authenticate
gh auth login
```

#### Option 2: Personal Access Token

Set the `GITHUB_TOKEN` environment variable:

```bash
export GITHUB_TOKEN=ghp_your_token_here
```

Your token needs these scopes:

- `repo` — Full control of private repositories
- `read:org` — Read org membership (if using organization repos)

### Setting up GitLab Authentication

:::note
GitLab support (including self-hosted instances) was added in rung 1.0.0.
:::

For GitLab remotes (gitlab.com or self-hosted), rung needs GitLab access to create and manage merge requests. You have two options:

#### Option 1: GitLab CLI (Recommended)

Install and authenticate the GitLab CLI:

```bash
# Install glab (if not already installed)
brew install glab       # macOS
apt install glab        # Ubuntu/Debian
winget install glab     # Windows

# Authenticate
glab auth login
```

#### Option 2: Personal Access Token

Set the `GITLAB_TOKEN` environment variable:

```bash
export GITLAB_TOKEN=glpat_your_token_here
```

Your token needs the `api` scope.

For **self-hosted GitLab**, also set `gitlab.api_url` in `.git/rung/config.toml` so rung can detect and reach your instance — see [Configuration](/reference/configuration/).

## Next Steps

Once installed, head to the [Quick Start](/getting-started/quickstart/) guide to create your first stack.
