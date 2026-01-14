import * as vscode from "vscode";
import { RungCli } from "../rung/cli";
import { StackTreeProvider } from "../providers/stackTreeProvider";

/**
 * Submit all stack branches as PRs.
 */
export async function submitCommand(
  cli: RungCli,
  treeProvider: StackTreeProvider
): Promise<void> {
  // Ask if user wants to submit as draft
  const submitType = await vscode.window.showQuickPick(
    [
      { label: "Submit PRs", description: "Create/update PRs normally" },
      { label: "Submit as Draft", description: "Create PRs as drafts" },
    ],
    {
      placeHolder: "How would you like to submit?",
    }
  );

  if (!submitType) {
    return; // User cancelled
  }

  const draft = submitType.label === "Submit as Draft";

  const output = await vscode.window.withProgress(
    {
      location: vscode.ProgressLocation.Notification,
      title: "Submitting PRs...",
      cancellable: false,
    },
    async () => {
      return await cli.submit({ draft });
    }
  );

  void vscode.window.showInformationMessage(
    draft ? "PRs submitted as drafts" : "PRs submitted successfully"
  );

  // Show output in output channel
  const outputChannel = vscode.window.createOutputChannel("Rung Submit");
  outputChannel.appendLine(output);
  outputChannel.show(true);

  treeProvider.refresh();
}
