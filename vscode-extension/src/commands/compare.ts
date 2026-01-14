import * as vscode from "vscode";
import { RungCli } from "../rung/cli";
import { StackTreeProvider } from "../providers/stackTreeProvider";
import { StackTreeItem } from "../providers/stackTreeItem";
import { isBranchInfo } from "../types";

// Type definitions for VS Code Git extension API
interface GitRepository {
  checkout(ref: string): Promise<void>;
  state: {
    remotes: Array<{ name: string; fetchUrl?: string }>;
  };
}

interface GitAPI {
  repositories: GitRepository[];
}

/**
 * Compare current branch with its parent (incremental diff).
 * Shows what changed in this stack level only.
 */
export function compareCommand(
  _cli: RungCli,
  treeProvider: StackTreeProvider,
  item?: StackTreeItem | string
): void {
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

  try {
    // Use VS Code's built-in git extension for diff
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

    // Show diff in terminal (VS Code git extension doesn't have branch-to-branch diff)
    const terminal = vscode.window.createTerminal("Rung Diff");
    terminal.sendText(`git diff ${branch.parent}..${branchName}`);
    terminal.show();

    void vscode.window.showInformationMessage(
      `Comparing ${branchName} with parent ${branch.parent}`
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
    } else if (remoteUrl.includes("github.com")) {
      // HTTPS format
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
