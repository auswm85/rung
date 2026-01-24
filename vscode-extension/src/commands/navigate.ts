import * as vscode from "vscode";
import { RungCli } from "../rung/cli";
import { StackTreeProvider } from "../providers/stackTreeProvider";
import { getWorkspaceRoot } from "../utils/workspace";
import { gitExec } from "../utils/git";

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

  const cwd = getWorkspaceRoot();
  if (!cwd) {
    void vscode.window.showErrorMessage("No workspace folder open");
    return;
  }

  try {
    // Check if already on this branch (using safe git exec)
    const { stdout: currentBranch } = await gitExec(
      ["rev-parse", "--abbrev-ref", "HEAD"],
      cwd
    );
    if (currentBranch.trim() === branchName) {
      return; // Already on this branch
    }

    // Use git directly for checkout (safe - no shell interpolation)
    await gitExec(["checkout", branchName], cwd);
    treeProvider.refresh();
  } catch (error: unknown) {
    const message =
      error instanceof Error ? error.message : "Failed to checkout branch";
    void vscode.window.showErrorMessage(`Checkout failed: ${message}`);
  }
}
