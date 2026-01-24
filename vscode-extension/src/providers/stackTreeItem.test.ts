import { describe, it, expect } from "vitest";
import { StackTreeItem, TreeItemType } from "./stackTreeItem";
import { BranchInfo } from "../types";
import { TreeItemCollapsibleState } from "vscode";

describe("StackTreeItem", () => {
  describe("branch items", () => {
    it("creates item with branch name", () => {
      const branch: BranchInfo = {
        name: "feature-1",
        parent: "main",
        state: { status: "synced" },
      };
      const item = new StackTreeItem(branch);

      expect(item.label).toBe("  feature-1");
      expect(item.collapsibleState).toBe(TreeItemCollapsibleState.None);
    });

    it("shows bullet for current branch", () => {
      const branch: BranchInfo = {
        name: "feature-1",
        parent: "main",
        state: { status: "synced" },
        is_current: true,
      };
      const item = new StackTreeItem(branch);

      expect(item.label).toBe("\u25cf feature-1");
    });

    it("sets check icon for synced branch", () => {
      const branch: BranchInfo = {
        name: "feature-1",
        parent: "main",
        state: { status: "synced" },
      };
      const item = new StackTreeItem(branch);

      expect(item.iconPath).toBeDefined();
      expect((item.iconPath as { id: string }).id).toBe("check");
    });

    it("sets warning icon for diverged branch", () => {
      const branch: BranchInfo = {
        name: "feature-1",
        parent: "main",
        state: { status: "diverged", commits_behind: 3 },
      };
      const item = new StackTreeItem(branch);

      expect(item.iconPath).toBeDefined();
      expect((item.iconPath as { id: string }).id).toBe("warning");
    });

    it("sets error icon for conflict branch", () => {
      const branch: BranchInfo = {
        name: "feature-1",
        parent: "main",
        state: { status: "conflict", files: ["a.ts"] },
      };
      const item = new StackTreeItem(branch);

      expect(item.iconPath).toBeDefined();
      expect((item.iconPath as { id: string }).id).toBe("error");
    });

    it("sets disconnect icon for detached branch", () => {
      const branch: BranchInfo = {
        name: "feature-1",
        parent: "main",
        state: { status: "detached" },
      };
      const item = new StackTreeItem(branch);

      expect(item.iconPath).toBeDefined();
      expect((item.iconPath as { id: string }).id).toBe("debug-disconnect");
    });

    it("shows PR number in description", () => {
      const branch: BranchInfo = {
        name: "feature-1",
        parent: "main",
        state: { status: "synced" },
        pr: 123,
      };
      const item = new StackTreeItem(branch);

      expect(item.description).toContain("#123");
    });

    it("shows commits behind in description for diverged", () => {
      const branch: BranchInfo = {
        name: "feature-1",
        parent: "main",
        state: { status: "diverged", commits_behind: 5 },
      };
      const item = new StackTreeItem(branch);

      expect(item.description).toContain("5 behind");
    });

    it("shows conflict count in description", () => {
      const branch: BranchInfo = {
        name: "feature-1",
        parent: "main",
        state: { status: "conflict", files: ["a.ts", "b.ts", "c.ts"] },
      };
      const item = new StackTreeItem(branch);

      expect(item.description).toContain("3 conflicts");
    });

    it("sets contextValue with branch status", () => {
      const branch: BranchInfo = {
        name: "feature-1",
        parent: "main",
        state: { status: "diverged", commits_behind: 2 },
        pr: 456,
        is_current: true,
      };
      const item = new StackTreeItem(branch);

      expect(item.contextValue).toContain("branch");
      expect(item.contextValue).toContain("diverged");
      expect(item.contextValue).toContain("hasPR");
      expect(item.contextValue).toContain("current");
    });

    it("sets checkout command", () => {
      const branch: BranchInfo = {
        name: "feature-1",
        parent: "main",
        state: { status: "synced" },
      };
      const item = new StackTreeItem(branch);

      expect(item.command).toBeDefined();
      expect(item.command?.command).toBe("rung.checkout");
      expect(item.command?.arguments).toEqual(["feature-1"]);
    });
  });

  describe("message items", () => {
    it("creates message item with label and tooltip", () => {
      const item = new StackTreeItem({
        type: TreeItemType.Message,
        label: "No branches",
        tooltip: "Run rung create",
      });

      expect(item.label).toBe("No branches");
      expect(item.tooltip).toBe("Run rung create");
      expect(item.contextValue).toBe("message");
    });

    it("sets info icon for message", () => {
      const item = new StackTreeItem({
        type: TreeItemType.Message,
        label: "Info",
        tooltip: "Details",
      });

      expect(item.iconPath).toBeDefined();
      expect((item.iconPath as { id: string }).id).toBe("info");
    });
  });

  describe("base items", () => {
    it("creates base item with name", () => {
      const item = new StackTreeItem({
        type: TreeItemType.Base,
        name: "main",
      });

      expect(item.label).toBe("\u2193 main");
      expect(item.contextValue).toBe("base");
    });

    it("sets git-branch icon for base", () => {
      const item = new StackTreeItem({
        type: TreeItemType.Base,
        name: "main",
      });

      expect(item.iconPath).toBeDefined();
      expect((item.iconPath as { id: string }).id).toBe("git-branch");
    });

    it("sets tooltip with base branch info", () => {
      const item = new StackTreeItem({
        type: TreeItemType.Base,
        name: "develop",
      });

      expect(item.tooltip).toContain("develop");
    });
  });
});
