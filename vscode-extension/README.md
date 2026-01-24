# Rung - Stacked PRs for VS Code

Visualize and manage stacked PRs directly from VS Code. This extension integrates with the [rung CLI](https://github.com/auswm85/rung) to provide a seamless stacked PR workflow.

## Features

### Stack Visualization
View your entire branch stack in the sidebar with status indicators:
- **Synced** - Branch is up to date with its parent
- **Diverged** - Parent has new commits, sync needed
- **Conflict** - Merge conflicts detected

### Quick Actions
- **Checkout** - Click any branch to switch to it
- **Sync** - Rebase branches to keep stack up to date
- **Submit** - Push all branches and create/update PRs
- **Compare** - View diff between branch and parent in VS Code's diff editor

### Branch Management
- **Create** - Create new branches with name or commit message
- **Navigate** - Move up/down the stack quickly
- **Merge** - Merge PRs and clean up branches

### Diagnostics
- **Doctor** - Run diagnostics to identify stack issues
- **Undo** - Revert the last sync operation

### Status Bar
See your current branch and stack position at a glance in the status bar.

## Requirements

- [rung CLI](https://github.com/auswm85/rung) installed and available in PATH
- Git repository initialized with rung (`rung init`)

## Extension Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `rung.cliPath` | `rung` | Path to rung CLI binary |
| `rung.autoRefresh` | `true` | Auto-refresh on file save and git events |
| `rung.refreshDebounce` | `1000` | Debounce time (ms) for auto-refresh |

## Commands

All commands are available via the Command Palette (`Cmd+Shift+P` / `Ctrl+Shift+P`):

| Command | Description |
|---------|-------------|
| `Rung: Refresh Stack` | Refresh the stack view |
| `Rung: Sync Branch` | Sync current branch with parent |
| `Rung: Submit PRs` | Submit all branches as PRs |
| `Rung: Create Branch` | Create a new branch in the stack |
| `Rung: Compare with Parent` | View changes from parent branch |
| `Rung: Open PR in Browser` | Open the branch's PR on GitHub |
| `Rung: Go to Child Branch` | Navigate to child branch |
| `Rung: Go to Parent Branch` | Navigate to parent branch |
| `Rung: Checkout Branch` | Switch to a branch |
| `Rung: Run Diagnostics` | Check stack health |
| `Rung: Undo Last Sync` | Revert last sync operation |
| `Rung: Merge PR & Cleanup` | Merge PR and delete branch |
| `Rung: Initialize Repository` | Initialize rung in repository |

## Getting Started

1. Install the [rung CLI](https://github.com/auswm85/rung)
2. Install this extension
3. Open a git repository in VS Code
4. Run `Rung: Initialize Repository` or `rung init` in terminal
5. Start creating stacked branches!

## License

MIT
