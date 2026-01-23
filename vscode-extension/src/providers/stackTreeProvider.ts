import * as vscode from "vscode";
import { RungCli } from "../rung/cli";
import { StatusOutput, BranchInfo, ErrorType, isRungError } from "../types";
import { StackTreeItem, TreeItemType } from "./stackTreeItem";

/**
 * Tree data provider for the rung stack view.
 * Displays branches as a "ladder" with tip at top and base at bottom.
 */
export class StackTreeProvider
  implements vscode.TreeDataProvider<StackTreeItem>
{
  private _onDidChangeTreeData = new vscode.EventEmitter<
    StackTreeItem | undefined
  >();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  private statusCache: StatusOutput | null = null;

  constructor(private cli: RungCli) {}

  /**
   * Refresh the tree view by clearing cache and firing change event.
   */
  refresh(): void {
    this.statusCache = null;
    this._onDidChangeTreeData.fire(undefined);
  }

  getTreeItem(element: StackTreeItem): vscode.TreeItem {
    return element;
  }

  async getChildren(element?: StackTreeItem): Promise<StackTreeItem[]> {
    // No nested children - flat ladder view
    if (element) {
      return [];
    }

    try {
      const status = await this.cli.status();
      this.statusCache = status;

      if (status.branches.length === 0) {
        return [
          this.createMessageItem(
            "No branches in stack",
            'Use "rung create <name>" to add a branch'
          ),
        ];
      }

      return this.buildLadder(status);
    } catch (error: unknown) {
      return this.handleError(error);
    }
  }

  /**
   * Build the ladder view: tip at top, base at bottom.
   * Uses DFS to order branches (children before parents).
   */
  private buildLadder(status: StatusOutput): StackTreeItem[] {
    const items: StackTreeItem[] = [];
    const branches = status.branches;

    // Build parent-to-children map
    const childrenMap = new Map<string | null, BranchInfo[]>();
    for (const branch of branches) {
      const parent = branch.parent;
      if (!childrenMap.has(parent)) {
        childrenMap.set(parent, []);
      }
      childrenMap.get(parent)!.push(branch);
    }

    // Find root branches (parent not in stack)
    const stackNames = new Set(branches.map((b) => b.name));
    const roots = branches.filter(
      (b) => !b.parent || !stackNames.has(b.parent)
    );

    // DFS: collect leaves first (tip of stack at top)
    const ordered: BranchInfo[] = [];
    const visited = new Set<string>();

    const visit = (branch: BranchInfo) => {
      if (visited.has(branch.name)) {
        return;
      }
      visited.add(branch.name);

      const children = childrenMap.get(branch.name) ?? [];
      // Visit children first (they appear at top)
      for (const child of children) {
        visit(child);
      }
      ordered.push(branch);
    };

    for (const root of roots) {
      visit(root);
    }

    // Create tree items (leaves/tip first)
    for (const branch of ordered) {
      items.push(new StackTreeItem(branch));
    }

    // Add base branch indicator at bottom
    const baseParent = roots[0]?.parent;
    if (baseParent) {
      items.push(this.createBaseItem(baseParent));
    }

    return items;
  }

  private createMessageItem(label: string, tooltip: string): StackTreeItem {
    return new StackTreeItem({
      type: TreeItemType.Message,
      label,
      tooltip,
    });
  }

  private createBaseItem(name: string): StackTreeItem {
    return new StackTreeItem({
      type: TreeItemType.Base,
      name,
    });
  }

  private createInitItem(): StackTreeItem {
    return new StackTreeItem({
      type: TreeItemType.Init,
    });
  }

  private handleError(error: unknown): StackTreeItem[] {
    if (isRungError(error)) {
      switch (error.type) {
        case ErrorType.NotInitialized:
          return [
            this.createInitItem(),
          ];
        case ErrorType.NotGitRepo:
          return [
            this.createMessageItem(
              "Not a Git repository",
              "Open a folder containing a Git repository"
            ),
          ];
        case ErrorType.CliNotFound:
          return [
            this.createMessageItem(
              "rung CLI not found",
              "Install rung or configure rung.cliPath in settings"
            ),
          ];
        default:
          return [this.createMessageItem("Error loading stack", error.message)];
      }
    }

    // Unknown error
    const message =
      error instanceof Error ? error.message : "Unknown error occurred";
    return [this.createMessageItem("Error", message)];
  }

  // --- Public getters for commands ---

  /**
   * Get the currently checked out branch from cached status.
   */
  getCurrentBranch(): BranchInfo | undefined {
    return this.statusCache?.branches.find((b) => b.is_current);
  }

  /**
   * Get a branch by name from cached status.
   */
  getBranch(name: string): BranchInfo | undefined {
    return this.statusCache?.branches.find((b) => b.name === name);
  }

  /**
   * Get the full cached status output.
   */
  getStatus(): StatusOutput | null {
    return this.statusCache;
  }
}
