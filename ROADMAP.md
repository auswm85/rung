# Roadmap

This document outlines the development direction for Rung. Our vision is to make Git history management as invisible and effortless as possible. This roadmap outlines the path from our current CLI foundation to a fully automated workflow. Items are organized by time horizon rather than specific dates, as development pace depends on contributor availability and priorities may shift based on user feedback.

## What is a Stack?

Rung manages **stacked branches** — a chain of dependent branches where each builds on the previous:

```text
Traditional Branching               Stacked Branches (Rung)

      feature-a                           feature-c  ←  PR #3
     /                                        │
main                                      feature-b  ←  PR #2
     \                                        │
      feature-b                           feature-a  ←  PR #1
                                              │
(independent, potentially conflicting)       main

                                    (sequential, each PR is small & focused)
```

With stacks, you can:

- Ship small, reviewable PRs that build on each other
- Keep working while waiting for review
- Merge from the bottom up, with Rung automatically rebasing descendants

## Now

_Current focus — actively being worked on or accepting contributions_

### Polish & Accessibility

These improvements make Rung more pleasant to use and easier to contribute to:

- [x] **Automated PR quality guardrails** — CI checks and tooling to maintain code quality across contributions
- [x] **--quiet flag** ([#19](https://github.com/auswm85/rung/issues/19)) — Suppress non-essential output for cleaner scripting
- [x] **Shell completions** ([#18](https://github.com/auswm85/rung/issues/18)) — Tab completion for bash, zsh, and fish
- [x] **rung log command** ([#22](https://github.com/auswm85/rung/issues/22)) — Show commits on the current stack branch
- [x] **Interactive navigation** (`rung move`) — TUI picker to jump to any branch in the stack
- [x] **NO_COLOR support** ([#20](https://github.com/auswm85/rung/issues/20)) — Respect the `NO_COLOR` environment variable for accessibility and scripting

### Security & Safety

- [x] **BranchName validation** ([#34](https://github.com/auswm85/rung/issues/34)) — Newtype with validation to prevent injection attacks
- [x] **Token zeroization** ([#32](https://github.com/auswm85/rung/issues/32)) — Securely clear tokens from memory after use

### Testing & Reliability

- [ ] **Sync conflict integration tests** ([#23](https://github.com/auswm85/rung/issues/23)) — Ensure conflict resolution workflows are well-tested

## Next

_Near-term priorities — planned once current work stabilizes_

### Performance

- [ ] **O(1) branch lookup** ([#33](https://github.com/auswm85/rung/issues/33)) — Optimize for large stacks with many branches

### Developer Experience

- [ ] **Improved error messages** — Actionable suggestions when operations fail
- [ ] **`rung restack`** — Interactively reorder branches within a stack
- [ ] **`rung adopt <branch>`** — Bring an existing branch into the stack

### Advanced Workflows

- [x] **`rung absorb`** ([#50](https://github.com/auswm85/rung/issues/50)) — Automatically distribute staged changes into the correct commits in history using git blame. This enables a workflow where you can make fixes and have them "absorbed" into the right place in your stack.

## Later

_Mid-term goals — meaningful features requiring more design work_

### Stack Manipulation

- [ ] **`rung split`** — Split the current branch into multiple branches, useful when a PR grows too large

- [ ] **`rung fold`** — Combine multiple adjacent branches into one, the inverse of split

### Sync Improvements

- [ ] **Parallel sync** — Sync independent branches concurrently for faster operations on wide stacks
- [ ] **Conflict prediction** — Warn before sync if conflicts are likely based on changed files

## Future

_Long-term vision — ideas we're excited about but haven't fully designed_

### Ecosystem Integration

- [ ] **GitLab support** — Extend forge support beyond GitHub
- [ ] **Bitbucket support** — Enterprise Git hosting integration
- [ ] **Editor extensions** — VS Code, maybe others for stack visualization

### Interactive Features

- [ ] **TUI mode** — Full terminal UI for managing stacks with real-time updates
- [ ] **`rung web`** — Local web UI for complex stack visualization, potentially built with Next.js or another modern JS framework

### Collaboration

- [ ] **Team stacks** — Share stack definitions across a team
- [ ] **Stack templates** — Predefined branch patterns for common workflows

---

## Contributing

Interested in helping? Issues labeled [`good first issue`](https://github.com/auswm85/rung/issues?q=is%3Aopen+is%3Aissue+label%3A%22good+first+issue%22) are great starting points. The **Now** section items are particularly welcome for contributions.

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.

## Feedback

This roadmap reflects our current thinking but isn't set in stone. If you have ideas or want to advocate for a feature, [open a discussion](https://github.com/auswm85/rung/discussions) or comment on the relevant issue.
