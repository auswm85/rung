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
        // Git branch name restrictions (see git-check-ref-format)
        if (value.includes(" ")) {
          return "Branch name cannot contain spaces";
        }
        if (value.startsWith("-")) {
          return "Branch name cannot start with a dash";
        }
        if (value.startsWith(".") || value.includes("/./") || value.includes("/..")) {
          return "Branch name cannot start with '.' or contain '/.' segments";
        }
        if (value.endsWith("/") || value.endsWith(".")) {
          return "Branch name cannot end with '/' or '.'";
        }
        if (value.endsWith(".lock")) {
          return "Branch name cannot end with '.lock'";
        }
        if (value.includes("..")) {
          return "Branch name cannot contain '..'";
        }
        if (value.includes("@{")) {
          return "Branch name cannot contain '@{'";
        }
        // Check for invalid characters: ~, ^, :, ?, *, [, \
        if (/[~^:?*[\\]/.test(value)) {
          return "Branch name contains invalid characters (~, ^, :, ?, *, [, \\)";
        }
        // Check for control characters (ASCII 0-31 and 127)
        // eslint-disable-next-line no-control-regex
        if (/[\x00-\x1f\x7f]/.test(value)) {
          return "Branch name cannot contain control characters";
        }
        if (value.includes("//")) {
          return "Branch name cannot contain consecutive slashes";
        }
        return undefined;
      },
    });

    if (!name) {
      return; // User cancelled
    }

    const trimmedName = name.trim();
    if (!trimmedName) {
      return; // Empty after trimming
    }

    createOptions = { name: trimmedName };
    progressTitle = `Creating branch '${trimmedName}'...`;
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

    const trimmedMessage = message.trim();
    if (!trimmedMessage) {
      return; // Empty after trimming
    }

    createOptions = { message: trimmedMessage };
    progressTitle = "Creating branch from commit message...";
  }

  try {
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
  } catch (error: unknown) {
    const message =
      error instanceof Error ? error.message : "Unknown error occurred";
    void vscode.window.showErrorMessage(`Failed to create branch: ${message}`);
  }
}
