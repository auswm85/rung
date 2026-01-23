import * as vscode from "vscode";
import { exec } from "child_process";
import { promisify } from "util";
import { RungCli } from "../rung/cli";
import { StackTreeProvider } from "../providers/stackTreeProvider";
import { getWorkspaceRoot } from "../utils/workspace";

const execAsync = promisify(exec);

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
  cli: RungCli,
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
    // Check if already on this branch
    const { stdout: currentBranch } = await execAsync(
      "git rev-parse --abbrev-ref HEAD",
      { cwd }
    );
    if (currentBranch.trim() === branchName) {
      return; // Already on this branch
    }

    // Use git directly for checkout
    await execAsync(`git checkout ${branchName}`, { cwd });
    treeProvider.refresh();
  } catch (error: unknown) {
    const message =
      error instanceof Error ? error.message : "Failed to checkout branch";
    void vscode.window.showErrorMessage(`Checkout failed: ${message}`);
  }
}
