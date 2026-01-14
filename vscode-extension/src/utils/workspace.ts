import * as vscode from "vscode";

/**
 * Get the root path of the current workspace folder.
 * Returns undefined if no workspace is open.
 */
export function getWorkspaceRoot(): string | undefined {
  const folders = vscode.workspace.workspaceFolders;
  if (!folders || folders.length === 0) {
    return undefined;
  }
  return folders[0].uri.fsPath;
}

/**
 * Check if a workspace folder is open.
 */
export function hasWorkspace(): boolean {
  return getWorkspaceRoot() !== undefined;
}
