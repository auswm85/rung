import * as vscode from "vscode";
import { debounce } from "../utils/debounce";

/**
 * Watches for file saves to trigger tree refresh.
 * This is optional and can be disabled via settings.
 */
export class FileWatcher {
  private disposable: vscode.Disposable | null = null;
  private debouncedCallback: () => void;

  constructor(
    private onFileSave: () => void,
    debounceMs: number = 1000
  ) {
    this.debouncedCallback = debounce(onFileSave, debounceMs);
  }

  /**
   * Start watching for file saves.
   */
  start(): void {
    this.disposable = vscode.workspace.onDidSaveTextDocument(() => {
      this.debouncedCallback();
    });
  }

  /**
   * Stop watching and clean up.
   */
  dispose(): void {
    this.disposable?.dispose();
    this.disposable = null;
  }
}
