# PRD: Rung – The Developer’s Ladder for Stacked PRs

**Version:** 1.1  
**Status:** Approved  
**Name:** Rung  
**Core Stack:** Rust (CLI) + TypeScript (VS Code Extension)

---

## 1. Product Vision

**Rung** is a lightweight, high-performance orchestration layer for Git. It enables "linear-parallel" development by automating the management of dependent PR stacks. Unlike heavy enterprise tools, Rung is designed to feel like a native extension of the developer's local environment, providing a visual "ladder" in the IDE and a powerful CLI for history manipulation.

---

## 2. Target Workflow: The "QA/Prod" Fast-Track

In your 2-stage pipeline, speed and safety are paramount. Rung supports this by:

- **Incremental QA:** Allowing the base of a stack (e.g., a DB schema) to hit the QA branch for testing while the developer continues coding the UI in a dependent branch.
- **Atomic Prod Merges:** Ensuring that when a stack is approved, it merges into Prod as a clean, verified sequence without "rebase hell" at the finish line.

---

## 3. Core Components

### 3.1 The "Rung" CLI (Rust)

The engine that handles Git state and history rewriting.

- **`rung create <name>`**: High-speed branch creation that automatically sets the current branch as the `parent`.
- **`rung sync`**: The recursive rebase engine.
  - **Logic:** Uses `git rebase --onto` to shift the entire stack vertically when `main` or a parent branch moves.
  - **Safety:** Automatically creates a `REF_BACKUP` before any rebase; `rung undo` reverts the entire stack instantly.
- **`rung submit`**:
  - Pushes branches with `--force-with-lease`.
  - Creates/Updates GitHub PRs via API.
  - **Automation:** Injects "Stack Navigation" (links to parent/child PRs) into the GitHub PR descriptions.

### 3.2 The VS Code Extension (TypeScript/React)

A visual "HUD" for the CLI.

- **The Ladder View:** A vertical tree in the sidebar showing the stack hierarchy.
  - **Synced State:** Branch is green (up-to-date).
  - **Diverged State:** Branch is yellow (needs `sync`).
  - **Conflict State:** Branch is red (needs manual resolution).
- **Incremental Diffs:** One-click comparison between a branch and its stack-parent rather than just the `main` branch.
- **Action Bar:** Quick-access buttons for `Sync`, `Submit`, and `Checkout`.

---

## 4. Technical Requirements & Integrations

### 4.1 Performance & Reliability

- **Sub-50ms Response:** CLI must return stack status nearly instantaneously using `git2-rs`.
- **Local-First State:** State is stored in `.git/config` and local Git Notes. No external database required.
- **Atomic Operations:** If a recursive sync fails mid-way, the tool must provide a clear path to resume or abort safely.

### 4.2 Third-Party SaaS Integrations

- **GitHub/GitLab:** API integration for PR metadata and base-branch management.
- **CI/CD (Actions/CircleCI):** Pull status check results into the VS Code "Rung" view.
- **Auth:** Utilize `gh` CLI credentials or standard OAuth tokens.

---

## 5. User Experience (The "Developer First" Rules)

1.  **Self-Healing:** If a user renames a branch via standard Git, Rung should detect the change via the Reflog and update its internal metadata.
2.  **Smart Conflict Handling:** When a conflict occurs during `sync`, the VS Code extension should automatically trigger the Merge Editor.
3.  **No Noise:** Provide a `--draft` flag for `submit` to prevent triggering CI/CD pipelines until the developer is ready for a formal QA review.

---

## 6. Preliminary Command Map

| Command        | Alias   | Description                                     |
| :------------- | :------ | :---------------------------------------------- |
| `rung status`  | `rg st` | Display the current stack tree and PR links.    |
| `rung sync`    | `rg sy` | Update the whole stack against the base branch. |
| `rung restack` | `rg re` | Move a branch to a different parent in stack.   |
| `rung nxt`     | `rg n`  | Quickly navigate up the current stack.          |
| `rung prv`     | `rg p`  | Quickly navigate down the current stack.        |
| `rung submit`  | `rg sm` | Push all changes and update/create GitHub PRs.  |
| `rung undo`    | `rg un` | Revert the last sync or rebase operation.       |

---

## 7. Testing Strategy

- **Sandbox Integration Tests:** Rust-based tests that spawn temporary repositories to validate the rebase engine.
- **Cross-Platform CI:** Validate binary stability on Linux, macOS (Intel/M-series), and Windows.
- **Mocked API Tests:** Use `wiremock` to test GitHub API interactions without creating real PRs.
