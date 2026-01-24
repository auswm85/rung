import * as vscode from "vscode";
import { RungCli } from "../rung/cli";
import { StackTreeProvider } from "../providers/stackTreeProvider";
import { StatusBarProvider } from "../providers/statusBarProvider";
import { syncCommand, syncContinueCommand, syncAbortCommand } from "./sync";
import { submitCommand } from "./submit";
import { nextCommand, prevCommand, checkoutCommand } from "./navigate";
import { createCommand } from "./create";
import { compareCommand, openPRCommand } from "./compare";
import { isRungError } from "../types";
import { StackTreeItem } from "../providers/stackTreeItem";

type CommandHandler = (...args: unknown[]) => Promise<void>;

/**
 * Register all rung commands with VS Code.
 */
export function registerCommands(
  context: vscode.ExtensionContext,
  cli: RungCli,
  treeProvider: StackTreeProvider,
  statusBar?: StatusBarProvider
): void {
  // Create output channel once and register for disposal
  const diagnosticsChannel = vscode.window.createOutputChannel("Rung Diagnostics");
  context.subscriptions.push(diagnosticsChannel);

  const commands: Array<[string, CommandHandler]> = [
    [
      "rung.refresh",
      async () => {
        treeProvider.refresh();
        if (statusBar) {
          await statusBar.update();
        }
      },
    ],
    ["rung.sync", async () => syncCommand(cli, treeProvider)],
    ["rung.sync.continue", async () => syncContinueCommand(cli, treeProvider)],
    ["rung.sync.abort", async () => syncAbortCommand(cli, treeProvider)],
    ["rung.submit", async () => submitCommand(cli, treeProvider)],
    ["rung.navigate.next", async () => nextCommand(cli, treeProvider)],
    ["rung.navigate.prev", async () => prevCommand(cli, treeProvider)],
    [
      "rung.checkout",
      async (name: unknown) => {
        if (typeof name === "string") {
          await checkoutCommand(cli, treeProvider, name);
        }
      },
    ],
    ["rung.create", async () => createCommand(cli, treeProvider)],
    [
      "rung.compare",
      async (item: unknown) => {
        const typedItem = item instanceof StackTreeItem || typeof item === "string" ? item : undefined;
        await compareCommand(cli, treeProvider, typedItem);
      },
    ],
    [
      "rung.openPR",
      async (pr: unknown) => {
        const typedPr = pr instanceof StackTreeItem || typeof pr === "number" ? pr : undefined;
        await openPRCommand(cli, treeProvider, typedPr);
      },
    ],
    [
      "rung.init",
      async () => {
        await cli.init();
        void vscode.window.showInformationMessage("Rung initialized successfully!");
        treeProvider.refresh();
        if (statusBar) {
          await statusBar.update();
        }
      },
    ],
    [
      "rung.doctor",
      async () => {
        const result = await cli.doctor();

        if (result.healthy) {
          void vscode.window.showInformationMessage("Rung: All diagnostics passed!");
          return;
        }

        // Show issues in output channel (reuse shared channel)
        diagnosticsChannel.clear();
        diagnosticsChannel.appendLine("=== Rung Diagnostics ===\n");

        for (const issue of result.issues) {
          const icon = issue.severity === "error" ? "❌" : "⚠️";
          diagnosticsChannel.appendLine(`${icon} [${issue.severity.toUpperCase()}] ${issue.message}`);
        }

        diagnosticsChannel.appendLine(`\n--- Summary: ${result.errors} error(s), ${result.warnings} warning(s) ---`);
        diagnosticsChannel.show();

        // Show notification
        if (result.errors > 0) {
          void vscode.window.showErrorMessage(`Rung: Found ${result.errors} error(s). See output for details.`);
        } else {
          void vscode.window.showWarningMessage(`Rung: Found ${result.warnings} warning(s). See output for details.`);
        }
      },
    ],
    [
      "rung.undo",
      async () => {
        const confirm = await vscode.window.showWarningMessage(
          "Undo the last sync operation?",
          { modal: true },
          "Undo"
        );
        if (confirm !== "Undo") {
          return;
        }

        await vscode.window.withProgress(
          {
            location: vscode.ProgressLocation.Notification,
            title: "Undoing last sync...",
            cancellable: false,
          },
          async () => {
            await cli.undo();
          }
        );

        void vscode.window.showInformationMessage("Rung: Sync undone successfully");
        treeProvider.refresh();
        if (statusBar) {
          await statusBar.update();
        }
      },
    ],
    [
      "rung.merge",
      async () => {
        const confirm = await vscode.window.showWarningMessage(
          "Merge the current branch's PR and clean up?",
          { modal: true },
          "Merge"
        );
        if (confirm !== "Merge") {
          return;
        }

        await vscode.window.withProgress(
          {
            location: vscode.ProgressLocation.Notification,
            title: "Merging PR...",
            cancellable: false,
          },
          async () => {
            await cli.merge();
          }
        );

        void vscode.window.showInformationMessage("Rung: PR merged and branch cleaned up");
        treeProvider.refresh();
        if (statusBar) {
          await statusBar.update();
        }
      },
    ],
  ];

  for (const [id, handler] of commands) {
    const disposable = vscode.commands.registerCommand(
      id,
      async (...args: unknown[]) => {
        try {
          await handler(...args);
        } catch (error: unknown) {
          // Show user-friendly error message
          let message: string;
          if (isRungError(error)) {
            message = error.message;
          } else if (error instanceof Error) {
            message = error.message;
          } else {
            message = "An unknown error occurred";
          }
          void vscode.window.showErrorMessage(`Rung: ${message}`);
        }
      }
    );
    context.subscriptions.push(disposable);
  }
}
