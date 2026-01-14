import * as vscode from "vscode";
import { RungCli } from "../rung/cli";
import { StackTreeProvider } from "../providers/stackTreeProvider";

// Type definitions for VS Code Git extension API
interface GitRepository {
  checkout(ref: string): Promise<void>;
}

interface GitAPI {
  repositories: GitRepository[];
}

/**
 * Navigate to the next (child) branch in the stack.
 */
export async function nextCommand(
  cli: RungCli,
  treeProvider: StackTreeProvider
): Promise<void> {
  await cli.next();
  treeProvider.refresh();
}

/**
 * Navigate to the previous (parent) branch in the stack.
 */
export async function prevCommand(
  cli: RungCli,
  treeProvider: StackTreeProvider
): Promise<void> {
  await cli.prev();
  treeProvider.refresh();
}

/**
 * Checkout a specific branch by name.
 */
export async function checkoutCommand(
  _cli: RungCli,
  treeProvider: StackTreeProvider,
  branchName: string
): Promise<void> {
  if (!branchName) {
    void vscode.window.showWarningMessage("No branch name provided");
    return;
  }

  try {
    // Use VS Code's built-in git extension for checkout
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

    await repo.checkout(branchName);
    treeProvider.refresh();
  } catch (error: unknown) {
    const message =
      error instanceof Error ? error.message : "Failed to checkout branch";
    void vscode.window.showErrorMessage(`Checkout failed: ${message}`);
  }
}
