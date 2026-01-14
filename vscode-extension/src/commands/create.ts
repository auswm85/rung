import * as vscode from "vscode";
import { RungCli } from "../rung/cli";
import { StackTreeProvider } from "../providers/stackTreeProvider";

/**
 * Create a new branch in the stack.
 * Prompts user for branch name.
 */
export async function createCommand(
  cli: RungCli,
  treeProvider: StackTreeProvider
): Promise<void> {
  const name = await vscode.window.showInputBox({
    prompt: "Enter branch name",
    placeHolder: "feature/my-feature",
    validateInput: (value) => {
      if (!value || value.trim().length === 0) {
        return "Branch name is required";
      }
      if (value.includes(" ")) {
        return "Branch name cannot contain spaces";
      }
      if (value.startsWith("-")) {
        return "Branch name cannot start with a dash";
      }
      return undefined;
    },
  });

  if (!name) {
    return; // User cancelled
  }

  await vscode.window.withProgress(
    {
      location: vscode.ProgressLocation.Notification,
      title: `Creating branch '${name}'...`,
      cancellable: false,
    },
    async () => {
      await cli.create(name);
    }
  );

  void vscode.window.showInformationMessage(`Branch '${name}' created`);
  treeProvider.refresh();
}
