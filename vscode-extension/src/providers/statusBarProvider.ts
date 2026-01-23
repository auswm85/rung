import * as vscode from "vscode";
import { RungCli } from "../rung/cli";
import { StatusOutput, BranchInfo, isRungError } from "../types";

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
    this.statusBarItem.command = "rungStack.focus";
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
      if (isRungError(error)) {
        // Hide status bar if rung isn't initialized
        this.statusBarItem.hide();
      } else {
        // Show error state
        this.statusBarItem.text = "$(error) Rung";
        this.statusBarItem.tooltip = "Error loading stack status";
        this.statusBarItem.show();
      }
    }
  }

  /**
   * Render the status bar content based on stack status.
   */
  private render(status: StatusOutput): void {
    const current = status.branches.find((b) => b.is_current);
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
   */
  private getStackPosition(
    status: StatusOutput,
    current: BranchInfo,
  ): { index: number; total: number } {
    const branches = status.branches;
    const stackNames = new Set(branches.map((b) => b.name));

    // Find the chain from current to root
    let depth = 1;
    let branch: BranchInfo | undefined = current;
    while (branch?.parent && stackNames.has(branch.parent)) {
      depth++;
      branch = branches.find((b) => b.name === branch!.parent);
    }

    return { index: depth, total: branches.length };
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
    }
  }

  /**
   * Build rich tooltip with stack details.
   */
  private buildTooltip(
    status: StatusOutput,
    current: BranchInfo,
    position: { index: number; total: number },
  ): vscode.MarkdownString {
    const md = new vscode.MarkdownString();
    md.isTrusted = true;

    md.appendMarkdown(`**Rung Stack**\n\n`);
    md.appendMarkdown(`Branch: \`${current.name}\`\n\n`);
    md.appendMarkdown(`Position: ${position.index} of ${position.total}\n\n`);

    if (current.parent) {
      md.appendMarkdown(`Parent: \`${current.parent}\`\n\n`);
    }

    // Status with color
    const statusText = this.getStatusText(current);
    md.appendMarkdown(`Status: ${statusText}\n\n`);

    if (current.pr) {
      md.appendMarkdown(`PR: [#${current.pr}](command:rung.openPR)\n\n`);
    }

    // Count branches needing sync
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
   * Get human-readable status text.
   */
  private getStatusText(branch: BranchInfo): string {
    switch (branch.state.status) {
      case "synced":
        return "$(check) Synced";
      case "diverged":
        return `$(warning) ${branch.state.commits_behind} commit(s) behind`;
      case "conflict":
        return `$(error) ${branch.state.files.length} conflict(s)`;
      case "detached":
        return "$(debug-disconnect) Detached";
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
