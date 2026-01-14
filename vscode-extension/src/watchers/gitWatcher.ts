import * as vscode from "vscode";
import { debounce } from "../utils/debounce";

/**
 * Watches for Git events that should trigger a tree refresh.
 * Monitors: HEAD changes, branch refs, rung state files.
 */
export class GitWatcher {
  private disposables: vscode.Disposable[] = [];
  private debouncedCallback: () => void;

  constructor(
    private onGitEvent: () => void,
    debounceMs: number = 1000
  ) {
    this.debouncedCallback = debounce(onGitEvent, debounceMs);
  }

  /**
   * Start watching for git events.
   */
  start(): void {
    const workspaceRoot = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    if (!workspaceRoot) {
      return;
    }

    // Watch key git files for changes
    // .git/HEAD - branch checkout
    // .git/refs/heads/** - branch creation/deletion
    // .git/REBASE_HEAD - rebase in progress
    // .git/MERGE_HEAD - merge in progress
    const gitPattern = new vscode.RelativePattern(
      workspaceRoot,
      ".git/{HEAD,REBASE_HEAD,MERGE_HEAD,refs/heads/**}"
    );

    const gitWatcher = vscode.workspace.createFileSystemWatcher(gitPattern);

    gitWatcher.onDidChange(() => this.debouncedCallback());
    gitWatcher.onDidCreate(() => this.debouncedCallback());
    gitWatcher.onDidDelete(() => this.debouncedCallback());

    this.disposables.push(gitWatcher);

    // Watch rung state files
    const rungPattern = new vscode.RelativePattern(
      workspaceRoot,
      ".git/rung/**"
    );

    const rungWatcher = vscode.workspace.createFileSystemWatcher(rungPattern);

    rungWatcher.onDidChange(() => this.debouncedCallback());
    rungWatcher.onDidCreate(() => this.debouncedCallback());
    rungWatcher.onDidDelete(() => this.debouncedCallback());

    this.disposables.push(rungWatcher);
  }

  /**
   * Stop watching and clean up.
   */
  dispose(): void {
    for (const disposable of this.disposables) {
      disposable.dispose();
    }
    this.disposables = [];
  }
}
