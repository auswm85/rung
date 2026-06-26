# Rung Architecture Design

---

## 1. System Overview

Rung is a two-component system:

1. **Rust CLI** (`rung`) - High-performance Git orchestration engine
2. **VS Code Extension** (`vscode-rung`) - Visual IDE integration

```
┌─────────────────────────────────────────────────────────────────┐
│                        VS Code Extension                         │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐  │
│  │ Ladder View  │  │ Status Bar   │  │ Command Palette      │  │
│  │ (TreeView)   │  │ Integration  │  │ (sync/submit/nav)    │  │
│  └──────┬───────┘  └──────┬───────┘  └──────────┬───────────┘  │
│         │                 │                      │              │
│         └─────────────────┴──────────────────────┘              │
│                           │                                      │
│                    ┌──────▼──────┐                              │
│                    │ RungService │ ←── spawns CLI                │
│                    └──────┬──────┘                              │
└───────────────────────────┼─────────────────────────────────────┘
                            │ JSON output
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│                         Rust CLI                                 │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                      rung-cli                            │   │
│  │  commands: create | sync | submit | status | nxt | prv   │   │
│  └──────────────────────────┬──────────────────────────────┘   │
│                             │                                    │
│  ┌──────────────────────────▼──────────────────────────────┐   │
│  │                      rung-core                           │   │
│  │  Stack model | State management | Sync engine | Backup   │   │
│  └───────┬─────────────────────────────────────┬───────────┘   │
│          │                                     │                │
│  ┌───────▼───────┐                   ┌────────▼────────┐       │
│  │   rung-git    │                   │   rung-forge    │       │
│  │  git2-rs ops  │                   │  ForgeApi+impls │       │
│  └───────────────┘                   └─────────────────┘       │
└─────────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Git Repository                              │
│       .git/rung/ (stack.json | config.toml | refs/)             │
└─────────────────────────────────────────────────────────────────┘
```

`rung-forge` defines the forge-neutral `ForgeApi` contract; `rung-github` (and
the in-progress `rung-gitlab`) implement it. The CLI selects a backend from the
detected git remote.

---

## 2. Rust Crate Structure

```
rung/
├── Cargo.toml                    # Workspace manifest
├── crates/
│   ├── rung-core/                # Core library
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── stack.rs          # Stack data model
│   │   │   ├── branch.rs         # Branch with metadata
│   │   │   ├── sync.rs           # Recursive rebase engine
│   │   │   ├── state.rs          # State persistence
│   │   │   ├── backup.rs         # REF_BACKUP management
│   │   │   └── error.rs          # Error types
│   │   └── Cargo.toml
│   │
│   ├── rung-git/                 # Git operations
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── repository.rs     # git2-rs wrapper
│   │   │   ├── rebase.rs         # Rebase operations
│   │   │   ├── reflog.rs         # Reflog for self-healing
│   │   │   └── notes.rs          # Git Notes integration
│   │   └── Cargo.toml
│   │
│   ├── rung-forge/              # Forge-neutral contract
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── traits.rs         # ForgeApi trait
│   │   │   ├── types.rs          # PR/comment/check domain types
│   │   │   ├── repo_id.rs        # Forge-neutral RepoId
│   │   │   ├── remote.rs         # ForgeKind remote detection
│   │   │   └── error.rs          # Neutral error types
│   │   └── Cargo.toml
│   │
│   ├── rung-github/              # GitHub backend
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── client.rs         # HTTP client + ForgeApi impl
│   │   │   └── auth.rs           # gh CLI / GITHUB_TOKEN
│   │   └── Cargo.toml
│   │
│   ├── rung-gitlab/              # GitLab backend (in progress)
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── client.rs         # HTTP client + ForgeApi (stubbed, #170)
│   │   │   └── auth.rs           # glab CLI / GITLAB_TOKEN
│   │   └── Cargo.toml
│   │
│   └── rung-cli/                 # CLI application
│       ├── src/
│       │   ├── main.rs
│       │   ├── commands/
│       │   │   ├── mod.rs
│       │   │   ├── create.rs     # rung create <name>
│       │   │   ├── sync.rs       # rung sync
│       │   │   ├── submit.rs     # rung submit
│       │   │   ├── status.rs     # rung status
│       │   │   ├── navigate.rs   # rung nxt / prv
│       │   │   └── undo.rs       # rung undo
│       │   ├── output.rs         # Terminal formatting
│       │   └── config.rs         # CLI configuration
│       └── Cargo.toml
│
├── vscode-rung/                  # VS Code extension
└── docs/
```

### Crate Dependencies

```
rung-cli
    ├── rung-core
    │   └── rung-git
    ├── rung-git
    ├── rung-forge              # ForgeApi trait, RepoId, ForgeKind detection
    ├── rung-github             # GitHub backend
    │   └── rung-forge
    └── clap (CLI parsing)
         tokio (async runtime)
         serde_json (output)

rung-gitlab                     # GitLab backend (implements rung-forge); CLI wiring pending (#171)
    └── rung-forge
```

---

## 3. Data Models

### 3.1 Core Types

```rust
/// A stack of dependent branches
pub struct Stack {
    /// Root branch (usually main/master)
    pub root: BranchRef,
    /// Ordered branches from root to tip
    pub branches: Vec<StackBranch>,
}

/// A branch within a stack
pub struct StackBranch {
    pub name: String,
    pub parent: Option<String>,
    pub commit: git2::Oid,
    pub upstream: Option<String>,
    pub pr: Option<PullRequest>,
    pub state: BranchState,
}

/// Branch synchronization state
pub enum BranchState {
    /// Up-to-date with parent
    Synced,
    /// Parent has moved, needs rebase
    Diverged { commits_behind: usize },
    /// Rebase resulted in conflicts
    Conflict { files: Vec<String> },
    /// Orphaned (parent deleted)
    Detached,
}

/// GitHub PR metadata (cached)
pub struct PullRequest {
    pub number: u64,
    pub url: String,
    pub title: String,
    pub state: PrState,
    pub checks: Vec<CheckStatus>,
    pub base_branch: String,
}

pub enum PrState {
    Draft,
    Open,
    Merged,
    Closed,
}

pub struct CheckStatus {
    pub name: String,
    pub status: CheckResult,
    pub url: Option<String>,
}

pub enum CheckResult {
    Pending,
    Success,
    Failure,
    Skipped,
}
```

### 3.2 State Storage

All Rung state is stored in a dedicated `.git/rung/` directory, keeping it isolated from Git's internal files while still traveling with the repository.

```
.git/
├── config              # Git's file - untouched by Rung
└── rung/               # Rung's home
    ├── stack.json      # Stack definition (branch order & relationships)
    ├── config.toml     # User configuration
    ├── sync_state      # Temporary file during sync operations
    └── refs/           # Backup commit hashes for undo
        └── <timestamp>/
            ├── feature-auth
            └── feature-auth-ui
```

**Stack definition** in `.git/rung/stack.json`:

```json
{
  "branches": [
    {
      "name": "feature/auth",
      "parent": "main",
      "pr": 123,
      "created": "2024-01-15T10:30:00Z"
    },
    {
      "name": "feature/auth-ui",
      "parent": "feature/auth",
      "pr": 124,
      "created": "2024-01-15T14:00:00Z"
    }
  ]
}
```

**Configuration** in `.git/rung/config.toml`:

```toml
[general]
default_remote = "origin"
backup_retention = 5    # Number of backup sets to keep
auto_sync = false

[github]
# Optional: override for GitHub Enterprise
# api_url = "https://github.example.com/api/v3"
```

**Sync state** in `.git/rung/sync_state` (temporary, deleted on completion):

```json
{
  "started_at": "2024-01-15T10:30:00Z",
  "backup_id": "1704067200",
  "current_branch": "feature/auth-ui",
  "completed": ["feature/auth"],
  "remaining": ["feature/auth-tests"]
}
```

**Backup refs** in `.git/rung/refs/<timestamp>/`:

```
.git/rung/refs/
└── 1704067200/
    ├── feature-auth          # Contains: abc123def456...
    └── feature-auth-ui       # Contains: 789xyz012abc...
```

Each file contains the commit SHA that branch pointed to before sync. Simple text files, one hash per file.

**Why `.git/rung/` over `.git/config`:**

| Concern             | `.git/config`    | `.git/rung/`        |
| ------------------- | ---------------- | ------------------- |
| Git conflicts       | ⚠️ Risk          | ✅ Isolated         |
| Future Git versions | ⚠️ May conflict  | ✅ Safe             |
| File format         | INI only         | ✅ JSON/TOML        |
| Discoverability     | Hidden in config | ✅ Clear ownership  |
| Backup refs         | Scattered        | ✅ All in one place |
| Portability         | ✅ With .git     | ✅ With .git        |

---

## 4. Command Specifications

### 4.1 `rung create <name>`

Creates a new branch with the current branch as parent.

```
Input:  Branch name
Output: Success message with stack position

Algorithm:
1. Get current branch name
2. Create new branch at HEAD
3. Add branch entry to .git/rung/stack.json with parent
4. Checkout new branch
```

### 4.2 `rung sync`

Recursively rebases the entire stack.

```
Input:  None (operates on current stack)
Output: Sync status or conflict info

Algorithm:
1. Identify current stack (traverse parents to root)
2. Create backup refs for all branches
3. For each branch (bottom-up):
   a. Check if parent moved
   b. If yes: git rebase --onto <new-parent-tip> <old-parent-tip> <branch>
   c. If conflict: pause, report, exit with code 1
4. Report success with summary

Flags:
  --dry-run    Show what would be rebased
  --continue   Resume after conflict resolution
  --abort      Abort in-progress sync, restore backups
```

### 4.3 `rung submit`

Push branches and create/update PRs.

```
Input:  None (operates on current stack)
Output: PR URLs

Algorithm:
1. For each branch in stack:
   a. Push with --force-with-lease
   b. If no PR: create PR via GitHub API
   c. If PR exists: update PR description with stack nav
2. Report PR URLs

Flags:
  --draft      Create PRs as drafts (no CI trigger)
  --force      Force push even if lease check fails
```

### 4.4 `rung status`

Display stack status.

```
Input:  None
Output: Tree view of stack with states

Algorithm:
1. Load stack from .git/rung/stack.json
2. For each branch:
   a. Compare commit to parent tip
   b. Fetch PR status if cached
3. Render tree

Flags:
  --json       Output as JSON (for VS Code extension)
  --fetch      Refresh PR status from GitHub
```

**JSON Output Format:**

```json
{
  "root": "main",
  "branches": [
    {
      "name": "feature/auth",
      "parent": "main",
      "state": "synced",
      "pr": {
        "number": 123,
        "url": "https://github.com/org/repo/pull/123",
        "state": "open",
        "checks": [{ "name": "CI", "status": "success" }]
      }
    },
    {
      "name": "feature/auth-ui",
      "parent": "feature/auth",
      "state": "diverged",
      "commits_behind": 2,
      "pr": null
    }
  ],
  "current": "feature/auth-ui"
}
```

### 4.5 `rung nxt` / `rung prv`

Navigate within the stack.

```
rung nxt: Checkout child branch (first child if multiple)
rung prv: Checkout parent branch
```

### 4.6 `rung undo`

Restore stack to pre-sync state.

```
Algorithm:
1. Find most recent backup in .git/rung/refs/<timestamp>/
2. For each file in backup dir: git reset --hard <sha-from-file>
3. Delete used backup directory
```

---

## 5. VS Code Extension Architecture

```
vscode-rung/
├── src/
│   ├── extension.ts              # Activation
│   │
│   ├── providers/
│   │   ├── ladderTreeProvider.ts # Sidebar tree view
│   │   ├── statusBarProvider.ts  # Status bar item
│   │   └── decorationProvider.ts # Editor decorations
│   │
│   ├── services/
│   │   ├── rungService.ts        # CLI wrapper
│   │   ├── stateWatcher.ts       # .git/rung/ watcher
│   │   └── cacheService.ts       # Local state cache
│   │
│   ├── commands/
│   │   ├── sync.ts
│   │   ├── submit.ts
│   │   ├── navigate.ts
│   │   ├── diff.ts               # Diff against parent
│   │   └── checkout.ts
│   │
│   ├── views/
│   │   └── webview/              # React components (optional)
│   │       ├── StackPanel.tsx
│   │       └── ConflictResolver.tsx
│   │
│   └── types/
│       └── rung.d.ts
│
├── package.json
├── tsconfig.json
└── webpack.config.js
```

### 5.1 Ladder View

TreeDataProvider implementation:

```typescript
class LadderTreeProvider implements vscode.TreeDataProvider<BranchNode> {
  private _onDidChange = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this._onDidChange.event;

  constructor(private rungService: RungService) {}

  async getChildren(element?: BranchNode): Promise<BranchNode[]> {
    if (!element) {
      // Root: return stack branches
      const status = await this.rungService.getStatus();
      return status.branches.map((b) => new BranchNode(b));
    }
    return []; // Flat list, no children
  }

  getTreeItem(element: BranchNode): vscode.TreeItem {
    const item = new vscode.TreeItem(element.name);
    item.iconPath = this.getStateIcon(element.state);
    item.description = element.pr?.number ? `#${element.pr.number}` : "";
    item.contextValue = element.state;
    return item;
  }

  private getStateIcon(state: BranchState): vscode.ThemeIcon {
    switch (state) {
      case "synced":
        return new vscode.ThemeIcon(
          "check",
          new vscode.ThemeColor("charts.green"),
        );
      case "diverged":
        return new vscode.ThemeIcon(
          "warning",
          new vscode.ThemeColor("charts.yellow"),
        );
      case "conflict":
        return new vscode.ThemeIcon(
          "error",
          new vscode.ThemeColor("charts.red"),
        );
      default:
        return new vscode.ThemeIcon("git-branch");
    }
  }
}
```

### 5.2 CLI Integration

```typescript
class RungService {
  private cache: StackStatus | null = null;
  private cacheTime: number = 0;
  private readonly CACHE_TTL = 5000; // 5 seconds

  async getStatus(forceRefresh = false): Promise<StackStatus> {
    if (
      !forceRefresh &&
      this.cache &&
      Date.now() - this.cacheTime < this.CACHE_TTL
    ) {
      return this.cache;
    }

    const result = await this.exec(["status", "--json"]);
    this.cache = JSON.parse(result);
    this.cacheTime = Date.now();
    return this.cache;
  }

  async sync(): Promise<SyncResult> {
    const result = await this.exec(["sync"]);
    this.invalidateCache();
    return this.parseSyncResult(result);
  }

  private exec(args: string[]): Promise<string> {
    return new Promise((resolve, reject) => {
      const proc = spawn("rung", args, { cwd: this.workspaceRoot });
      let stdout = "";
      let stderr = "";
      proc.stdout.on("data", (d) => (stdout += d));
      proc.stderr.on("data", (d) => (stderr += d));
      proc.on("close", (code) => {
        if (code === 0) resolve(stdout);
        else reject(new Error(stderr));
      });
    });
  }
}
```

---

## 6. Sync Algorithm Detail

### 6.1 Core Sync Logic

```rust
pub fn sync_stack(repo: &Repository, stack: &Stack) -> Result<SyncResult> {
    // Phase 1: Create backups
    let backup = Backup::create(repo, &stack)?;

    // Phase 2: Determine rebase targets
    let plan = create_sync_plan(repo, &stack)?;

    if plan.is_empty() {
        return Ok(SyncResult::AlreadySynced);
    }

    // Phase 3: Execute rebases
    for action in plan {
        match execute_rebase(repo, &action)? {
            RebaseOutcome::Success => continue,
            RebaseOutcome::Conflict(info) => {
                return Ok(SyncResult::Paused {
                    at_branch: action.branch.clone(),
                    conflict: info,
                    backup_id: backup.id,
                    remaining: plan.remaining(),
                });
            }
        }
    }

    Ok(SyncResult::Complete {
        branches_rebased: plan.len(),
        backup_id: backup.id,
    })
}

struct SyncAction {
    branch: String,
    old_base: Oid,
    new_base: Oid,
}

fn execute_rebase(repo: &Repository, action: &SyncAction) -> Result<RebaseOutcome> {
    // git rebase --onto <new_base> <old_base> <branch>
    let mut rebase = repo.rebase(
        Some(&repo.find_commit(action.old_base)?),
        Some(&repo.find_commit(action.new_base)?),
        None,
        Some(&mut RebaseOptions::new()),
    )?;

    while let Some(op) = rebase.next() {
        match op {
            Ok(_) => {
                if repo.index()?.has_conflicts() {
                    return Ok(RebaseOutcome::Conflict(get_conflict_info(repo)?));
                }
                rebase.commit(None, &signature, None)?;
            }
            Err(e) => return Err(e.into()),
        }
    }

    rebase.finish(None)?;
    Ok(RebaseOutcome::Success)
}
```

### 6.2 Conflict Handling

When a conflict occurs:

1. CLI exits with code 1 and conflict info
2. VS Code extension detects conflict state
3. Extension opens VS Code's merge editor
4. User resolves conflicts
5. User runs `rung sync --continue`
6. Sync resumes from where it paused

---

## 7. GitHub Integration

### 7.1 Authentication

```rust
pub enum AuthMethod {
    /// Read token from gh CLI
    GhCli,
    /// Environment variable
    EnvVar(String),
    /// OAuth flow (future)
    OAuth,
}

impl AuthMethod {
    pub fn get_token(&self) -> Result<String> {
        match self {
            AuthMethod::GhCli => {
                let output = Command::new("gh")
                    .args(["auth", "token"])
                    .output()?;
                Ok(String::from_utf8(output.stdout)?.trim().to_string())
            }
            AuthMethod::EnvVar(var) => {
                std::env::var(var).map_err(|_| Error::NoToken)
            }
            AuthMethod::OAuth => unimplemented!(),
        }
    }
}
```

### 7.2 PR Stack Navigation

Injected into PR descriptions:

```markdown
<!-- rung:stack-nav -->

### Stack

| Branch              | PR   | Status  |
| ------------------- | ---- | ------- |
| main                | -    | base    |
| feature/auth        | #123 | ✅      |
| **feature/auth-ui** | #124 | 🔄      |
| feature/auth-tests  | -    | pending |

_Managed by [Rung](https://github.com/org/rung)_

<!-- /rung:stack-nav -->
```

---

## 8. Performance Requirements

| Operation                  | Target            | Strategy                           |
| -------------------------- | ----------------- | ---------------------------------- |
| `rung status`              | <50ms             | In-memory git2-rs, cached state    |
| `rung sync` (no conflicts) | <200ms per branch | Native rebase, parallel where safe |
| `rung submit`              | <1s per PR        | Async HTTP, batch where possible   |
| VS Code tree refresh       | <100ms            | Cached JSON, incremental updates   |

### Optimization Techniques

1. **git2-rs over shell**: Native bindings, no process spawn overhead
2. **Lazy evaluation**: Don't compute diff until needed
3. **Parallel status**: Use rayon for multi-branch state checks
4. **Connection pooling**: Reuse HTTP connections for GitHub API
5. **Incremental updates**: Only refresh changed branches in UI

---

## 9. Testing Strategy

### 9.1 Test Categories

```
tests/
├── unit/
│   ├── stack_tests.rs        # Stack model logic
│   ├── state_tests.rs        # stack.json / config.toml parsing
│   └── sync_plan_tests.rs    # Plan generation
│
├── integration/
│   ├── fixtures/
│   │   └── repos/            # Pre-built test repos
│   ├── sandbox.rs            # TempDir repo creation
│   ├── sync_tests.rs         # Full sync scenarios
│   ├── undo_tests.rs         # Backup/restore
│   ├── conflict_tests.rs     # Conflict handling
│   └── self_heal_tests.rs    # Reflog detection
│
├── mocked/
│   ├── github_tests.rs       # wiremock API tests
│   └── fixtures/
│       └── api_responses/    # JSON fixtures
│
└── e2e/
    ├── cli_tests.rs          # Full CLI invocation
    └── vscode_tests.ts       # Extension tests
```

### 9.2 Key Test Scenarios

1. **Linear Stack Sync**
   - Create 3-branch stack
   - Add commit to base
   - Sync: verify all branches rebased

2. **Mid-Stack Conflict**
   - Create conflicting changes
   - Sync: verify pause at conflict
   - Resolve, continue: verify completion

3. **Undo After Partial Sync**
   - Sync with conflict at branch 2/3
   - Undo: verify all branches restored

4. **Branch Rename Detection**
   - Rename branch via `git branch -m`
   - Verify rung detects via reflog
   - Update metadata automatically

5. **PR Creation with Navigation**
   - Submit new stack
   - Verify PR bodies contain stack table
   - Verify PR base branches correct

---

## 10. Error Handling

### 10.1 Error Categories

```rust
pub enum RungError {
    // Git errors
    GitError(git2::Error),
    NotARepository,
    BranchNotFound(String),

    // Stack errors
    NotInStack,
    CyclicDependency,
    OrphanedBranch(String),

    // Sync errors
    ConflictDetected(ConflictInfo),
    RebaseFailed(String),
    NoBackupFound,

    // GitHub errors
    AuthenticationFailed,
    ApiError(u16, String),
    RateLimited,

    // General
    IoError(std::io::Error),
    StateParseError(String),  // .git/rung/ file parsing errors
}
```

### 10.2 User-Facing Messages

| Error                  | Message                                      | Recovery Hint                                       |
| ---------------------- | -------------------------------------------- | --------------------------------------------------- |
| `NotInStack`           | "Current branch is not part of a rung stack" | "Use `rung create` to start a new stack"            |
| `ConflictDetected`     | "Conflict in {file} while syncing {branch}"  | "Resolve conflicts then run `rung sync --continue`" |
| `AuthenticationFailed` | "Could not authenticate with GitHub"         | "Run `gh auth login` or set GITHUB_TOKEN"           |

---

## 11. Future Considerations

### Phase 2 Features (Not in MVP)

1. **GitLab Support**: The `rung-forge` contract and `rung-gitlab` backend (auth + client scaffold) exist; remaining work is the `ForgeApi` implementation (#170) and CLI wiring (#171)
2. **Multi-Root Stacks**: Allow branch to depend on multiple parents
3. **Auto-Sync**: Watch for upstream changes, notify user
4. **Team Collaboration**: Shared stacks across team members
5. **Merge Queue Integration**: GitHub merge queue awareness

### Extension Points

- **Plugin System**: Allow custom PR body templates
- **Hook Support**: Pre/post sync hooks for CI integration
- **Custom Remotes**: Support for non-GitHub forges
