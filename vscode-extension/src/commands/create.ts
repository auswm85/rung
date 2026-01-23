import * as vscode from "vscode";
import { RungCli } from "../rung/cli";
import { StackTreeProvider } from "../providers/stackTreeProvider";

/**
 * Create a new branch in the stack.
 * Prompts user to choose between branch name or commit message.
 */
export async function createCommand(
  cli: RungCli,
  treeProvider: StackTreeProvider
): Promise<void> {
  // Ask user how they want to create the branch
  const choice = await vscode.window.showQuickPick(
    [
      {
        label: "$(git-branch) Branch Name",
        description: "Enter a branch name directly",
        value: "name",
      },
      {
        label: "$(comment) Commit Message",
        description: "Generate branch name from commit message",
        value: "message",
      },
    ],
    {
      placeHolder: "How would you like to create the branch?",
    }
  );

  if (!choice) {
    return; // User cancelled
  }

  let createOptions: { name?: string; message?: string };
  let progressTitle: string;

  if (choice.value === "name") {
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

    createOptions = { name };
    progressTitle = `Creating branch '${name}'...`;
  } else {
    const message = await vscode.window.showInputBox({
      prompt: "Enter commit message",
      placeHolder: "Add user authentication feature",
      validateInput: (value) => {
        if (!value || value.trim().length === 0) {
          return "Commit message is required";
        }
        return undefined;
      },
    });

    if (!message) {
      return; // User cancelled
    }

    createOptions = { message };
    progressTitle = "Creating branch from commit message...";
  }

  await vscode.window.withProgress(
    {
      location: vscode.ProgressLocation.Notification,
      title: progressTitle,
      cancellable: false,
    },
    async () => {
      await cli.create(createOptions);
    }
  );

  void vscode.window.showInformationMessage("Branch created");
  treeProvider.refresh();
}
