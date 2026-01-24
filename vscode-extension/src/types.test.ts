import { describe, it, expect } from "vitest";
import {
  isBranchInfo,
  isRungError,
  ErrorType,
  BranchInfo,
  RungError,
} from "./types";

describe("isBranchInfo", () => {
  it("returns true for valid BranchInfo", () => {
    const branch: BranchInfo = {
      name: "feature-1",
      parent: "main",
      state: { status: "synced" },
    };
    expect(isBranchInfo(branch)).toBe(true);
  });

  it("returns true for BranchInfo with all optional fields", () => {
    const branch: BranchInfo = {
      name: "feature-1",
      parent: "main",
      state: { status: "diverged", commits_behind: 3 },
      pr: 123,
      is_current: true,
    };
    expect(isBranchInfo(branch)).toBe(true);
  });

  it("returns false for null", () => {
    expect(isBranchInfo(null)).toBe(false);
  });

  it("returns false for undefined", () => {
    expect(isBranchInfo(undefined)).toBe(false);
  });

  it("returns false for object without name", () => {
    expect(isBranchInfo({ state: { status: "synced" } })).toBe(false);
  });

  it("returns false for object without state", () => {
    expect(isBranchInfo({ name: "test" })).toBe(false);
  });

  it("returns false for primitives", () => {
    expect(isBranchInfo("string")).toBe(false);
    expect(isBranchInfo(123)).toBe(false);
    expect(isBranchInfo(true)).toBe(false);
  });
});

describe("isRungError", () => {
  it("returns true for valid RungError", () => {
    const error: RungError = {
      type: ErrorType.NotInitialized,
      message: "Rung is not initialized",
    };
    expect(isRungError(error)).toBe(true);
  });

  it("returns true for RungError with details", () => {
    const error: RungError = {
      type: ErrorType.ConflictDetected,
      message: "Conflicts detected",
      details: "file1.ts, file2.ts",
    };
    expect(isRungError(error)).toBe(true);
  });

  it("returns false for null", () => {
    expect(isRungError(null)).toBe(false);
  });

  it("returns false for undefined", () => {
    expect(isRungError(undefined)).toBe(false);
  });

  it("returns false for object without type", () => {
    expect(isRungError({ message: "error" })).toBe(false);
  });

  it("returns false for object without message", () => {
    expect(isRungError({ type: ErrorType.Unknown })).toBe(false);
  });

  it("returns false for standard Error", () => {
    expect(isRungError(new Error("test"))).toBe(false);
  });
});

describe("BranchState", () => {
  it("synced state has correct structure", () => {
    const branch: BranchInfo = {
      name: "test",
      parent: null,
      state: { status: "synced" },
    };
    expect(branch.state.status).toBe("synced");
  });

  it("diverged state includes commits_behind", () => {
    const branch: BranchInfo = {
      name: "test",
      parent: null,
      state: { status: "diverged", commits_behind: 5 },
    };
    expect(branch.state.status).toBe("diverged");
    if (branch.state.status === "diverged") {
      expect(branch.state.commits_behind).toBe(5);
    }
  });

  it("conflict state includes files array", () => {
    const branch: BranchInfo = {
      name: "test",
      parent: null,
      state: { status: "conflict", files: ["a.ts", "b.ts"] },
    };
    expect(branch.state.status).toBe("conflict");
    if (branch.state.status === "conflict") {
      expect(branch.state.files).toEqual(["a.ts", "b.ts"]);
    }
  });

  it("detached state has correct structure", () => {
    const branch: BranchInfo = {
      name: "test",
      parent: null,
      state: { status: "detached" },
    };
    expect(branch.state.status).toBe("detached");
  });
});
