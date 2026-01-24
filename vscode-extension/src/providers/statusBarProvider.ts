import * as vscode from "vscode";
import { RungCli } from "../rung/cli";
import { StatusOutput, BranchInfo, isRungError, ErrorType } from "../types";

/**
 * Status bar item showing current stack state.
 * Displays: branch name, stack position, and sync status.
 */
export class StatusBarProvider implements vscode.Disposable {
  private statusBarItem: vscode.StatusBarItem;
  private disposables: vscode.Disposable[] = [];

  constructor(private cli: RungCli) {
    this.statusBarItem = vscode.window.createStatusBarItem(
      vscode.StatusBarAlignment.Left,
      100,
    );
    this.statusBarItem.command = "workbench.view.extension.rung";
    this.statusBarItem.name = "Rung Stack";
    this.disposables.push(this.statusBarItem);
  }

  /**
   * Update the status bar with current stack state.
   */
  async update(): Promise<void> {
    try {
      const status = await this.cli.status();
      this.render(status);
      this.statusBarItem.show();
    } catch (error: unknown) {
      if (isRungError(error) && error.type === ErrorType.NotInitialized) {
        // Hide status bar only if rung isn't initialized
        this.statusBarItem.hide();
      } else {
        // Show error state for all other errors
        const errorMessage = isRungError(error)
          ? error.message
          : error instanceof Error
            ? error.message
            : "Unknown error";
        this.statusBarItem.text = "$(error) Rung";
        this.statusBarItem.tooltip = `Error: ${errorMessage}`;
        this.statusBarItem.show();
      }
    }
  }

  /**
   * Render the status bar content based on stack status.
   */
  private render(status: StatusOutput): void {
    // Try to find current branch by is_current flag, fall back to status.current name
    let current = status.branches.find((b) => b.is_current);
    if (!current && status.current) {
      current = status.branches.find((b) => b.name === status.current);
    }
    if (!current) {
      this.statusBarItem.text = "$(git-branch) Rung";
      this.statusBarItem.tooltip = "No branch in stack";
      return;
    }

    const position = this.getStackPosition(status, current);
    const icon = this.getStatusIcon(current);
    const prText = current.pr ? ` #${current.pr}` : "";

    this.statusBarItem.text = `${icon} ${current.name}${prText}`;
    this.statusBarItem.tooltip = this.buildTooltip(status, current, position);
  }

  /**
   * Calculate stack position (1-indexed from base).
   * Returns position in the current chain, not total branches in stack.
   */
  private getStackPosition(
    status: StatusOutput,
    current: BranchInfo,
  ): { index: number; total: number } {
    const branches = status.branches;
    const stackNames = new Set(branches.map((b) => b.name));

    // Build children map for traversing down
    const childrenMap = new Map<string, BranchInfo[]>();
    for (const b of branches) {
      if (b.parent) {
        const existing = childrenMap.get(b.parent);
        if (existing) {
          existing.push(b);
        } else {
          childrenMap.set(b.parent, [b]);
        }
      }
    }

    // Count ancestors (depth from root to current)
    // Track visited nodes to prevent infinite loops on cyclic graphs
    let depth = 1;
    let branch: BranchInfo | undefined = current;
    const visitedAncestors = new Set<string>([current.name]);
    while (branch?.parent && stackNames.has(branch.parent)) {
      // Cycle guard: stop if we've already visited this parent
      if (visitedAncestors.has(branch.parent)) {
        break;
      }
      visitedAncestors.add(branch.parent);
      depth++;
      branch = branches.find((b) => b.name === branch!.parent);
    }

    // Count descendants (from current to deepest leaf in this chain)
    // Track visited nodes to prevent infinite loops on cyclic graphs
    let descendants = 0;
    let node: BranchInfo | undefined = current;
    const visitedDescendants = new Set<string>([current.name]);
    while (node) {
      const children = childrenMap.get(node.name);
      if (!children || children.length === 0) {
        break;
      }
      // Follow first child (main chain) for consistent counting
      const nextNode = children[0];
      // Cycle guard: stop if we've already visited this child
      if (!nextNode.name || visitedDescendants.has(nextNode.name)) {
        break;
      }
      visitedDescendants.add(nextNode.name);
      node = nextNode;
      descendants++;
    }

    // Total chain length = ancestors + current + descendants
    const chainLength = depth + descendants;

    return { index: depth, total: chainLength };
  }

  /**
   * Get icon based on branch sync status.
   */
  private getStatusIcon(branch: BranchInfo): string {
    switch (branch.state.status) {
      case "synced":
        return "$(check)";
      case "diverged":
        return "$(warning)";
      case "conflict":
        return "$(error)";
      case "detached":
        return "$(debug-disconnect)";
      default:
        return "$(question)";
    }
  }

  /**
   * Build rich tooltip with stack details.
   * Uses appendText() for user-controlled data to prevent Markdown injection.
   */
  private buildTooltip(
    status: StatusOutput,
    current: BranchInfo,
    position: { index: number; total: number },
  ): vscode.MarkdownString {
    const md = new vscode.MarkdownString();
    // Restrict trusted commands to only allow rung.openPR
    md.isTrusted = { enabledCommands: ["rung.openPR"] };

    md.appendMarkdown(`**Rung Stack**\n\n`);
    md.appendMarkdown(`Branch: \``);
    md.appendText(current.name); // User data - use appendText to prevent injection
    md.appendMarkdown(`\`\n\n`);
    md.appendMarkdown(`Position: ${position.index} of ${position.total}\n\n`);

    if (current.parent) {
      md.appendMarkdown(`Parent: \``);
      md.appendText(current.parent); // User data - use appendText to prevent injection
      md.appendMarkdown(`\`\n\n`);
    }

    // Status - getStatusText returns safe text (numbers from internal state, not user input)
    const statusText = this.getStatusText(current);
    md.appendMarkdown(`Status: ${statusText}\n\n`);

    if (current.pr) {
      // PR number is from our internal state, safe to interpolate
      // Use encoded command URI for safety
      const prNumber = Number(current.pr);
      if (Number.isInteger(prNumber) && prNumber > 0) {
        md.appendMarkdown(`PR: [#${prNumber}](command:rung.openPR)\n\n`);
      }
    }

    // Count branches needing sync (internal state, safe)
    const needsSync = status.branches.filter(
      (b) => b.state.status === "diverged" || b.state.status === "conflict",
    ).length;

    if (needsSync > 0) {
      md.appendMarkdown(
        `---\n\n$(warning) ${needsSync} branch(es) need sync\n\n`,
      );
    }

    md.appendMarkdown(`\n\n*Click to open stack view*`);

    return md;
  }

  /**
   * Get human-readable status text (returns safe static text, no user data).
   */
  private getStatusText(branch: BranchInfo): string {
    switch (branch.state.status) {
      case "synced":
        return "Synced";
      case "diverged":
        return `${branch.state.commits_behind} commit(s) behind`;
      case "conflict":
        return `${branch.state.files.length} conflict(s)`;
      case "detached":
        return "Detached";
      default:
        return "Unknown";
    }
  }

  /**
   * Hide the status bar item.
   */
  hide(): void {
    this.statusBarItem.hide();
  }

  /**
   * Clean up resources.
   */
  dispose(): void {
    for (const disposable of this.disposables) {
      disposable.dispose();
    }
    this.disposables = [];
  }
}
