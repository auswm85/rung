import * as vscode from "vscode";
import { RungCli } from "./rung/cli";
import { StackTreeProvider } from "./providers/stackTreeProvider";
import { StatusBarProvider } from "./providers/statusBarProvider";
import { GitWatcher } from "./watchers/gitWatcher";
import { FileWatcher } from "./watchers/fileWatcher";
import { registerCommands } from "./commands";
import { getWorkspaceRoot } from "./utils/workspace";

let outputChannel: vscode.OutputChannel;

/**
 * Extension activation.
 * Called when the extension is first activated (workspace contains .git/rung).
 */
export async function activate(
  context: vscode.ExtensionContext
): Promise<void> {
  outputChannel = vscode.window.createOutputChannel("Rung");
  context.subscriptions.push(outputChannel);

  outputChannel.appendLine("Rung extension activating...");

  // Check if we have a workspace
  const workspaceRoot = getWorkspaceRoot();
  if (workspaceRoot) {
    outputChannel.appendLine(`Workspace: ${workspaceRoot}`);
  } else {
    outputChannel.appendLine("No workspace folder open");
  }

  // Initialize CLI wrapper
  const cli = new RungCli(outputChannel);
  context.subscriptions.push({ dispose: () => cli.dispose() });

  // Initialize tree provider
  const treeProvider = new StackTreeProvider(cli);

  // Create tree view
  const treeView = vscode.window.createTreeView("rungStack", {
    treeDataProvider: treeProvider,
    showCollapseAll: false,
  });
  context.subscriptions.push(treeView);

  // Initialize status bar
  const statusBar = new StatusBarProvider(cli);
  context.subscriptions.push(statusBar);

  // Initial status bar update
  void statusBar.update();

  // Register commands
  registerCommands(context, cli, treeProvider, statusBar);

  // Set up auto-refresh watchers
  const config = vscode.workspace.getConfiguration("rung");
  const autoRefresh = config.get("autoRefresh", true);
  const debounceMs = config.get("refreshDebounce", 1000);

  // Combined refresh function for tree and status bar
  const refreshAll = () => {
    treeProvider.refresh();
    void statusBar.update();
  };

  if (autoRefresh) {
    outputChannel.appendLine(
      `Auto-refresh enabled (debounce: ${debounceMs}ms)`
    );

    // Git event watcher
    const gitWatcher = new GitWatcher(refreshAll, debounceMs);
    gitWatcher.start();
    context.subscriptions.push({ dispose: () => gitWatcher.dispose() });

    // File save watcher
    const fileWatcher = new FileWatcher(refreshAll, debounceMs);
    fileWatcher.start();
    context.subscriptions.push({ dispose: () => fileWatcher.dispose() });
  }

  // Check if rung is initialized
  try {
    const isInitialized = await cli.isInitialized();
    if (isInitialized) {
      outputChannel.appendLine("Rung is initialized in this repository");
    } else {
      outputChannel.appendLine(
        'Rung not initialized. Run "rung init" to set up.'
      );
    }
  } catch (error) {
    outputChannel.appendLine(
      `Error checking rung status: ${error instanceof Error ? error.message : "Unknown"}`
    );
  }

  outputChannel.appendLine("Rung extension activated");
}

/**
 * Extension deactivation.
 */
export function deactivate(): void {
  outputChannel?.appendLine("Rung extension deactivated");
}
