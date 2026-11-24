// Types matching rung CLI JSON output (from rung status --json)

export interface StatusOutput {
  branches: BranchInfo[];
  current: string | null;
}

export interface BranchInfo {
  name: string;
  parent: string | null;
  state: BranchState;
  pr?: number;
  is_current?: boolean;
}

export type BranchState =
  | { status: "synced" }
  | { status: "diverged"; commits_behind: number }
  | { status: "conflict"; files: string[] }
  | { status: "detached" };

// Types matching rung CLI JSON output (from rung doctor --json)

export interface DoctorOutput {
  healthy: boolean;
  errors: number;
  warnings: number;
  issues: DoctorIssue[];
}

export interface DoctorIssue {
  severity: "error" | "warning";
  message: string;
}

// Extension configuration

export interface RungConfig {
  cliPath: string;
  autoRefresh: boolean;
  refreshDebounce: number;
}

// Error handling

export enum ErrorType {
  NotInitialized = "NOT_INITIALIZED",
  NotGitRepo = "NOT_GIT_REPO",
  CliNotFound = "CLI_NOT_FOUND",
  SyncInProgress = "SYNC_IN_PROGRESS",
  ConflictDetected = "CONFLICT_DETECTED",
  Unknown = "UNKNOWN",
}

export class RungError extends Error {
  constructor(
    public readonly type: ErrorType,
    message: string,
    public readonly details?: string
  ) {
    super(message);
    this.name = "RungError";
  }
}

// Helper type guards

export function isBranchInfo(data: unknown): data is BranchInfo {
  return (
    typeof data === "object" &&
    data !== null &&
    "name" in data &&
    "state" in data
  );
}

export function isRungError(error: unknown): error is RungError {
  return error instanceof RungError;
}
