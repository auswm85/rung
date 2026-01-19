---
title: Installation
description: How to install rung on macOS, Linux, and Windows.
---

Choose the installation method that works best for your system.

## Pre-built Binaries (Recommended)

Download the latest release for your platform from [GitHub Releases](https://github.com/auswm85/rung/releases).

### macOS (Apple Silicon)

```bash
curl -fsSL https://github.com/auswm85/rung/releases/latest/download/rung-$(curl -s https://api.github.com/repos/auswm85/rung/releases/latest | grep tag_name | cut -d '"' -f 4 | sed 's/v//')-aarch64-apple-darwin.tar.gz | tar xz
sudo mv rung /usr/locxal/bin/
```

### macOS (Intel)

```bash
curl -fsSL https://github.com/auswm85/rung/releases/latest/download/rung-$(curl -s https://api.github.com/repos/auswm85/rung/releases/latest | grep tag_name | cut -d '"' -f 4 | sed 's/v//')-x86_64-apple-darwin.tar.gz | tar xz
sudo mv rung /usr/local/bin/
```

### Linux (x86_64)

```bash
curl -fsSL https://github.com/auswm85/rung/releases/latest/download/rung-$(curl -s https://api.github.com/repos/auswm85/rung/releases/latest | grep tag_name | cut -d '"' -f 4 | sed 's/v//')-x86_64-unknown-linux-gnu.tar.gz | tar xz
sudo mv rung /usr/local/bin/
```

### Windows

Download the `.zip` from [releases](https://github.com/auswm85/rung/releases) and add to your PATH.

## Homebrew (macOS/Linux)

```bash
brew tap auswm85/rung https://github.com/auswm85/rung
brew install rung
```

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
- **GitHub CLI (`gh`)** authenticated, or `GITHUB_TOKEN` environment variable

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

## Next Steps

Once installed, head to the [Quick Start](/getting-started/quickstart/) guide to create your first stack.
