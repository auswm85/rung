import * as vscode from "vscode";
import { RungCli } from "../rung/cli";
import { StackTreeProvider } from "../providers/stackTreeProvider";
import { StackTreeItem } from "../providers/stackTreeItem";
import { isBranchInfo } from "../types";
import { getWorkspaceRoot } from "../utils/workspace";
import { gitExec } from "../utils/git";

// Strict regex for valid git branch names (prevents injection via special chars)
const SAFE_BRANCH_REGEX = /^[a-zA-Z0-9._\-/]+$/;

// Type definitions for VS Code Git extension API
interface GitRepository {
  checkout(ref: string): Promise<void>;
  rootUri: vscode.Uri;
  state: {
    remotes: Array<{ name: string; fetchUrl?: string }>;
  };
}

interface GitAPI {
  repositories: GitRepository[];
  toGitUri(uri: vscode.Uri, ref: string): vscode.Uri;
}

/**
 * Compare current branch with its parent (incremental diff).
 * Shows what changed in this stack level only.
 */
export async function compareCommand(
  _cli: RungCli,
  treeProvider: StackTreeProvider,
  item?: StackTreeItem | string
): Promise<void> {
  let branchName: string;

  if (typeof item === "string") {
    branchName = item;
  } else if (item && isBranchInfo(item.data)) {
    branchName = item.data.name;
  } else {
    // Use current branch
    const current = treeProvider.getCurrentBranch();
    if (!current) {
      void vscode.window.showWarningMessage("No current branch in stack");
      return;
    }
    branchName = current.name;
  }

  const branch = treeProvider.getBranch(branchName);
  if (!branch?.parent) {
    void vscode.window.showWarningMessage(
      `Branch '${branchName}' has no parent to compare with`
    );
    return;
  }

  const cwd = getWorkspaceRoot();
  if (!cwd) {
    void vscode.window.showErrorMessage("No workspace folder open");
    return;
  }

  try {
    // Get the Git extension API
    const gitExtension =
      vscode.extensions.getExtension<{ getAPI: (version: number) => GitAPI }>(
        "vscode.git"
      );
    if (!gitExtension) {
      void vscode.window.showErrorMessage("Git extension not available");
      return;
    }

    const git = gitExtension.exports.getAPI(1);
    const repo = git.repositories[0];

    if (!repo) {
      void vscode.window.showErrorMessage("No Git repository found");
      return;
    }

    // Validate branch names to prevent command injection
    if (!SAFE_BRANCH_REGEX.test(branchName) || !SAFE_BRANCH_REGEX.test(branch.parent)) {
      void vscode.window.showErrorMessage("Invalid branch name detected");
      return;
    }

    // Get list of changed files between branches (using safe git exec with -- separator)
    const { stdout } = await gitExec(
      ["diff", "--name-only", branch.parent, branchName, "--"],
      cwd
    );

    const files = stdout.trim().split("\n").filter((f) => f.length > 0);

    if (files.length === 0) {
      void vscode.window.showInformationMessage(
        `No changes between ${branchName} and ${branch.parent}`
      );
      return;
    }

    // Show quick pick with changed files
    const items = files.map((file) => ({
      label: `$(file) ${file}`,
      file,
      description: "",
    }));

    const selected = await vscode.window.showQuickPick(items, {
      placeHolder: `${files.length} file(s) changed - select to view diff`,
      matchOnDescription: true,
    });

    if (!selected) {
      return;
    }

    // Create URIs for the diff using VS Code Git extension's toGitUri
    const fileUri = vscode.Uri.joinPath(repo.rootUri, selected.file);
    const leftUri = git.toGitUri(fileUri, branch.parent);
    const rightUri = git.toGitUri(fileUri, branchName);

    // Open the diff editor
    await vscode.commands.executeCommand(
      "vscode.diff",
      leftUri,
      rightUri,
      `${selected.file} (${branch.parent} ↔ ${branchName})`
    );
  } catch (error: unknown) {
    const message =
      error instanceof Error ? error.message : "Failed to open diff";
    void vscode.window.showErrorMessage(`Compare failed: ${message}`);
  }
}

/**
 * Open PR in browser for a branch.
 */
export async function openPRCommand(
  _cli: RungCli,
  treeProvider: StackTreeProvider,
  prNumber?: number | StackTreeItem
): Promise<void> {
  let pr: number | undefined;

  if (typeof prNumber === "number") {
    pr = prNumber;
  } else if (prNumber && isBranchInfo(prNumber.data)) {
    pr = prNumber.data.pr;
  } else {
    // Use current branch
    const current = treeProvider.getCurrentBranch();
    pr = current?.pr;
  }

  if (!pr) {
    void vscode.window.showWarningMessage("No PR associated with this branch");
    return;
  }

  // Get repository URL from git remote
  try {
    const gitExtension =
      vscode.extensions.getExtension<{ getAPI: (version: number) => GitAPI }>(
        "vscode.git"
      );
    if (!gitExtension) {
      void vscode.window.showErrorMessage("Git extension not available");
      return;
    }

    const git = gitExtension.exports.getAPI(1);
    const repo = git.repositories[0];

    if (!repo) {
      void vscode.window.showErrorMessage("No Git repository found");
      return;
    }

    // Try to get the remote URL
    const remotes = repo.state.remotes;
    const origin = remotes.find((r) => r.name === "origin");

    if (!origin?.fetchUrl) {
      void vscode.window.showErrorMessage("Could not find origin remote");
      return;
    }

    // Parse GitHub URL from remote
    const remoteUrl = origin.fetchUrl;
    let baseUrl: string;

    if (remoteUrl.startsWith("git@github.com:")) {
      // SSH format: git@github.com:user/repo.git
      const path = remoteUrl.replace("git@github.com:", "").replace(".git", "");
      baseUrl = `https://github.com/${path}`;
    } else if (
      remoteUrl.startsWith("https://github.com/") ||
      remoteUrl.startsWith("http://github.com/")
    ) {
      // HTTPS format - validate hostname explicitly to prevent URL injection
      baseUrl = remoteUrl.replace(".git", "");
    } else {
      void vscode.window.showErrorMessage(
        "Could not parse GitHub URL from remote"
      );
      return;
    }

    const prUrl = `${baseUrl}/pull/${pr}`;
    await vscode.env.openExternal(vscode.Uri.parse(prUrl));
  } catch (error: unknown) {
    const message =
      error instanceof Error ? error.message : "Failed to open PR";
    void vscode.window.showErrorMessage(`Open PR failed: ${message}`);
  }
}
