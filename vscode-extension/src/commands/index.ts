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
      (item: unknown) => {
        const typedItem = item instanceof StackTreeItem || typeof item === "string" ? item : undefined;
        compareCommand(cli, treeProvider, typedItem);
        return Promise.resolve();
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
