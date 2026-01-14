import * as vscode from "vscode";
import { RungCli } from "../rung/cli";
import { StackTreeProvider } from "../providers/stackTreeProvider";
import { ErrorType, isRungError } from "../types";

/**
 * Sync the stack by rebasing branches when parent has moved.
 */
export async function syncCommand(
  cli: RungCli,
  treeProvider: StackTreeProvider
): Promise<void> {
  try {
    await vscode.window.withProgress(
      {
        location: vscode.ProgressLocation.Notification,
        title: "Syncing stack...",
        cancellable: false,
      },
      async () => {
        await cli.sync();
      }
    );

    void vscode.window.showInformationMessage("Stack synced successfully");
    treeProvider.refresh();
  } catch (error: unknown) {
    if (isRungError(error) && error.type === ErrorType.ConflictDetected) {
      const action = await vscode.window.showWarningMessage(
        "Conflicts detected during sync. Resolve them and continue.",
        "Continue after resolving",
        "Abort sync"
      );

      if (action === "Continue after resolving") {
        void vscode.window.showInformationMessage(
          'Resolve conflicts in your editor, stage changes, then run "Rung: Continue Sync"'
        );
      } else if (action === "Abort sync") {
        await syncAbortCommand(cli, treeProvider);
      }
    } else if (isRungError(error) && error.type === ErrorType.SyncInProgress) {
      const action = await vscode.window.showWarningMessage(
        "A sync operation is already in progress.",
        "Continue",
        "Abort"
      );

      if (action === "Continue") {
        await syncContinueCommand(cli, treeProvider);
      } else if (action === "Abort") {
        await syncAbortCommand(cli, treeProvider);
      }
    } else {
      throw error;
    }
  }
}

/**
 * Continue a paused sync after resolving conflicts.
 */
export async function syncContinueCommand(
  cli: RungCli,
  treeProvider: StackTreeProvider
): Promise<void> {
  try {
    await vscode.window.withProgress(
      {
        location: vscode.ProgressLocation.Notification,
        title: "Continuing sync...",
        cancellable: false,
      },
      async () => {
        await cli.sync({ continueSync: true });
      }
    );

    void vscode.window.showInformationMessage("Sync continued successfully");
    treeProvider.refresh();
  } catch (error: unknown) {
    if (isRungError(error) && error.type === ErrorType.ConflictDetected) {
      void vscode.window.showWarningMessage(
        "More conflicts detected. Resolve them and try again."
      );
    } else {
      throw error;
    }
  }
}

/**
 * Abort a sync in progress and restore from backup.
 */
export async function syncAbortCommand(
  cli: RungCli,
  treeProvider: StackTreeProvider
): Promise<void> {
  await vscode.window.withProgress(
    {
      location: vscode.ProgressLocation.Notification,
      title: "Aborting sync...",
      cancellable: false,
    },
    async () => {
      await cli.sync({ abort: true });
    }
  );

  void vscode.window.showInformationMessage("Sync aborted, branches restored");
  treeProvider.refresh();
}
