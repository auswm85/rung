# Contributing to Rung

## Getting Started

```bash
# Clone and set up git hooks
git clone https://github.com/auswm85/rung
cd rung
git config core.hooksPath .githooks

# Run tests
cargo test

# Run lints
cargo fmt --check
cargo clippy
```

## Branch Naming

Use prefixes that match the type of change:

| Prefix      | Purpose                   | Example                        |
| ----------- | ------------------------- | ------------------------------ |
| `feat/`     | New feature               | `feat/stack-reorder`           |
| `fix/`      | Bug fix                   | `fix/sync-conflict-handling`   |
| `chore/`    | Maintenance, dependencies | `chore/update-deps`            |
| `docs/`     | Documentation only        | `docs/add-examples`            |
| `refactor/` | Code restructuring        | `refactor/extract-sync-module` |
| `test/`     | Adding or updating tests  | `test/merge-edge-cases`        |
| `perf/`     | Performance improvement   | `perf/stack-lookup`            |

## Commit Messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>

[optional body]

[optional footer(s)]
```

### Types

- `feat`: New feature (correlates with MINOR in semver)
- `fix`: Bug fix (correlates with PATCH in semver)
- `docs`: Documentation only
- `style`: Formatting, no code change
- `refactor`: Code change that neither fixes a bug nor adds a feature
- `perf`: Performance improvement
- `test`: Adding or correcting tests
- `chore`: Maintenance tasks

### Scope (optional)

The crate or area affected: `core`, `git`, `github`, `cli`

### Examples

```
feat(cli): add --json flag to status command

fix(core): handle empty stack in sync operation

docs: update installation instructions

chore(deps): bump clap to 4.5
```

### Breaking Changes

Add `!` after type or include `BREAKING CHANGE:` in footer:

```
feat(core)!: change stack file format

BREAKING CHANGE: stack.json schema updated, run `rung migrate`
```

## Code Style

- Run `cargo fmt` before committing
- Run `cargo clippy` and address warnings
- Follow existing patterns in the codebase
- Add doc comments for public APIs
- Prefer `thiserror` for library errors, `anyhow` for CLI

## Testing

- Add tests for new functionality
- Run `cargo test` before submitting PR
- For git operations, use `tempfile` for test repositories

## Pull Requests

1. Create a branch from `main` using naming conventions above
2. Make focused, atomic commits following conventional commits
3. Ensure CI passes (fmt, clippy, tests)
4. Fill out the PR template
5. Request review

**Current focus areas:**

- ⚙️ **Git Internals:** Help optimize `git2-rs` operations.
- 🧪 **Testing:** We need more edge-case tests for complex rebase scenarios as well as integration tests

## Questions?

Open an issue or start a discussion.
