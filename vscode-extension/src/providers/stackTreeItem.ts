import * as vscode from "vscode";
import { BranchInfo, BranchState } from "../types";

/**
 * Types of items that can appear in the tree.
 */
export enum TreeItemType {
  Branch = "branch",
  Base = "base",
  Message = "message",
  Init = "init",
}

/**
 * Data for a message item (empty state, error, etc.)
 */
export interface MessageData {
  type: TreeItemType.Message;
  label: string;
  tooltip: string;
}

/**
 * Data for a base branch indicator.
 */
export interface BaseData {
  type: TreeItemType.Base;
  name: string;
}

/**
 * Data for an init action item.
 */
export interface InitData {
  type: TreeItemType.Init;
}

/**
 * Union type for all tree item data.
 */
export type TreeItemData = BranchInfo | MessageData | BaseData | InitData;

/**
 * Check if data is a BranchInfo (does not have 'type' property).
 */
function isBranchData(data: TreeItemData): data is BranchInfo {
  return !("type" in data);
}

/**
 * Tree item representing a branch, base, or message in the stack view.
 */
export class StackTreeItem extends vscode.TreeItem {
  constructor(public readonly data: TreeItemData) {
    const label = StackTreeItem.getLabel(data);
    super(label, vscode.TreeItemCollapsibleState.None);
    this.setupItem(data);
  }

  private static getLabel(data: TreeItemData): string {
    if ("type" in data) {
      if (data.type === TreeItemType.Message) {
        return data.label;
      }
      if (data.type === TreeItemType.Base) {
        return `\u2193 ${data.name}`;
      }
      if (data.type === TreeItemType.Init) {
        return "Initialize Rung";
      }
    }

    // BranchInfo - use type guard
    if (isBranchData(data)) {
      const prefix = data.is_current ? "\u25cf " : "  ";
      return `${prefix}${data.name}`;
    }

    return "";
  }

  private setupItem(data: TreeItemData): void {
    if ("type" in data) {
      if (data.type === TreeItemType.Message) {
        this.tooltip = data.tooltip;
        this.iconPath = new vscode.ThemeIcon("info");
        this.contextValue = "message";
        return;
      }
      if (data.type === TreeItemType.Base) {
        this.tooltip = `Base branch: ${data.name}`;
        this.iconPath = new vscode.ThemeIcon("git-branch");
        this.contextValue = "base";
        // Click to checkout base branch
        this.command = {
          command: "rung.checkout",
          title: "Checkout",
          arguments: [data.name],
        };
        return;
      }
      if (data.type === TreeItemType.Init) {
        this.tooltip = "Click to initialize rung in this repository";
        this.iconPath = new vscode.ThemeIcon("add");
        this.contextValue = "init";
        this.command = {
          command: "rung.init",
          title: "Initialize Rung",
        };
        return;
      }
    }

    // BranchInfo - use type guard
    if (isBranchData(data)) {
      this.tooltip = this.buildTooltip(data);
      this.iconPath = this.getIcon(data.state);
      this.contextValue = this.buildContextValue(data);
      this.description = this.buildDescription(data);

      // Click to checkout
      this.command = {
        command: "rung.checkout",
        title: "Checkout",
        arguments: [data.name],
      };
    }
  }

  private buildTooltip(branch: BranchInfo): vscode.MarkdownString {
    const md = new vscode.MarkdownString();
    md.appendMarkdown(`**${branch.name}**\n\n`);

    if (branch.parent) {
      md.appendMarkdown(`Parent: \`${branch.parent}\`\n\n`);
    }

    md.appendMarkdown(`Status: ${this.getStatusText(branch.state)}\n\n`);

    if (branch.pr) {
      md.appendMarkdown(`PR: #${branch.pr}`);
    }

    return md;
  }

  private getStatusText(state: BranchState): string {
    switch (state.status) {
      case "synced":
        return "\u2705 Synced";
      case "diverged":
        return `\u26a0\ufe0f ${state.commits_behind} commit(s) behind`;
      case "conflict":
        return `\u274c Conflict in ${state.files.length} file(s)`;
      case "detached":
        return "\ud83d\udd17 Detached";
    }
  }

  private getIcon(state: BranchState): vscode.ThemeIcon {
    switch (state.status) {
      case "synced":
        return new vscode.ThemeIcon(
          "check",
          new vscode.ThemeColor("testing.iconPassed")
        );
      case "diverged":
        return new vscode.ThemeIcon(
          "warning",
          new vscode.ThemeColor("list.warningForeground")
        );
      case "conflict":
        return new vscode.ThemeIcon(
          "error",
          new vscode.ThemeColor("list.errorForeground")
        );
      case "detached":
        return new vscode.ThemeIcon(
          "debug-disconnect",
          new vscode.ThemeColor("disabledForeground")
        );
    }
  }

  private buildContextValue(branch: BranchInfo): string {
    const parts = ["branch"];
    parts.push(branch.state.status);
    if (branch.pr) {
      parts.push("hasPR");
    }
    if (branch.is_current) {
      parts.push("current");
    }
    return parts.join("-");
  }

  private buildDescription(branch: BranchInfo): string {
    const parts: string[] = [];

    if (branch.state.status === "diverged") {
      parts.push(`${branch.state.commits_behind} behind`);
    } else if (branch.state.status === "conflict") {
      parts.push(`${branch.state.files.length} conflicts`);
    }

    if (branch.pr) {
      parts.push(`#${branch.pr}`);
    }

    return parts.join(" \u00b7 ");
  }
}
