---
title: update
description: Update rung to the latest version.
since: "0.1.2"
---

Update rung to the latest version from crates.io.

## Usage

```bash
rung update
rung update --check
```

## Aliases

- `rung up` — shorthand for `rung update`

## Options

| Option    | Description                               |
| --------- | ----------------------------------------- |
| `--check` | Only check for updates without installing |

## Example

### Check for Updates

```bash
$ rung update --check

Current version: 0.1.0
Latest version:  0.2.0

Run `rung update` to install the latest version.
```

### Install Update

```bash
$ rung update

Current version: 0.1.0
Latest version:  0.2.0

Installing rung 0.2.0...
✓ Updated to rung 0.2.0
```

## How It Works

1. **Version Check** — Queries crates.io for the latest published version
2. **Comparison** — Compares with your installed version
3. **Installation** — Uses `cargo-binstall` (fast, pre-built binaries) if available, otherwise falls back to `cargo install`

## Notes

- Requires an internet connection to check crates.io
- If `cargo-binstall` is installed, updates are faster (uses pre-built binaries)
- The update replaces the current `rung` binary in your PATH

## Related Commands

- [`doctor`](/commands/doctor/) — Check rung installation health
