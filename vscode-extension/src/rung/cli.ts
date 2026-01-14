import * as vscode from "vscode";
import { exec } from "child_process";
import { promisify } from "util";
import {
  StatusOutput,
  RungConfig,
  RungError,
  ErrorType,
  isRungError,
} from "../types";
import { getWorkspaceRoot } from "../utils/workspace";

const execAsync = promisify(exec);

/**
 * Wrapper for the rung CLI binary.
 * Executes commands and parses output.
 */
export class RungCli {
  private config: RungConfig;
  private configDisposable: vscode.Disposable;

  constructor(private outputChannel: vscode.OutputChannel) {
    this.config = this.loadConfig();

    // Listen for config changes
    this.configDisposable = vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration("rung")) {
        this.config = this.loadConfig();
      }
    });
  }

  dispose(): void {
    this.configDisposable.dispose();
  }

  private loadConfig(): RungConfig {
    const config = vscode.workspace.getConfiguration("rung");
    return {
      cliPath: config.get("cliPath", "rung"),
      autoRefresh: config.get("autoRefresh", true),
      refreshDebounce: config.get("refreshDebounce", 1000),
    };
  }

  getConfig(): RungConfig {
    return this.config;
  }

  private async execute(
    args: string[]
  ): Promise<{ stdout: string; stderr: string }> {
    const cwd = getWorkspaceRoot();
    if (!cwd) {
      throw this.createError(ErrorType.NotGitRepo, "No workspace folder open");
    }

    const command = `${this.config.cliPath} ${args.join(" ")}`;
    this.outputChannel.appendLine(`> ${command}`);

    try {
      const result = await execAsync(command, {
        cwd,
        timeout: 30000, // 30s timeout
        env: { ...process.env },
      });

      if (result.stdout) {
        this.outputChannel.appendLine(result.stdout);
      }
      if (result.stderr) {
        this.outputChannel.appendLine(`stderr: ${result.stderr}`);
      }

      return result;
    } catch (error: unknown) {
      const execError = error as {
        code?: string | number;
        stderr?: string;
        message?: string;
      };
      this.outputChannel.appendLine(
        `Error: ${execError.message ?? "Unknown error"}`
      );
      throw this.parseError(execError);
    }
  }

  private parseError(error: {
    code?: string | number;
    stderr?: string;
    message?: string;
  }): RungError {
    const message = error.stderr ?? error.message ?? "Unknown error";

    // Check for specific error patterns
    if (message.includes("rung init") || message.includes("not initialized")) {
      return this.createError(
        ErrorType.NotInitialized,
        "Rung is not initialized in this repository. Run 'rung init' first."
      );
    }
    if (message.includes("not inside a git repository")) {
      return this.createError(
        ErrorType.NotGitRepo,
        "Not inside a Git repository"
      );
    }
    if (message.includes("Sync already in progress")) {
      return this.createError(
        ErrorType.SyncInProgress,
        "A sync operation is already in progress. Use --continue or --abort."
      );
    }
    if (
      message.includes("Conflict") ||
      message.includes("conflict") ||
      message.includes("CONFLICT")
    ) {
      return this.createError(
        ErrorType.ConflictDetected,
        "Conflicts detected during sync. Resolve them and run 'rung sync --continue'.",
        message
      );
    }
    if (error.code === "ENOENT") {
      return this.createError(
        ErrorType.CliNotFound,
        `rung CLI not found at '${this.config.cliPath}'. Install rung or update the rung.cliPath setting.`
      );
    }

    return this.createError(ErrorType.Unknown, message);
  }

  private createError(
    type: ErrorType,
    message: string,
    details?: string
  ): RungError {
    return { type, message, details };
  }

  // --- Public API ---

  /**
   * Get stack status as JSON.
   */
  async status(): Promise<StatusOutput> {
    const { stdout } = await this.execute(["status", "--json"]);
    try {
      return JSON.parse(stdout) as StatusOutput;
    } catch {
      throw this.createError(
        ErrorType.Unknown,
        "Failed to parse status output",
        stdout
      );
    }
  }

  /**
   * Sync the stack by rebasing branches.
   */
  async sync(
    options: {
      dryRun?: boolean;
      continueSync?: boolean;
      abort?: boolean;
      base?: string;
    } = {}
  ): Promise<string> {
    const args = ["sync"];
    if (options.dryRun) {
      args.push("--dry-run");
    }
    if (options.continueSync) {
      args.push("--continue");
    }
    if (options.abort) {
      args.push("--abort");
    }
    if (options.base) {
      args.push("--base", options.base);
    }

    const { stdout } = await this.execute(args);
    return stdout;
  }

  /**
   * Submit PRs for all branches in the stack.
   */
  async submit(
    options: { draft?: boolean; force?: boolean; title?: string } = {}
  ): Promise<string> {
    const args = ["submit"];
    if (options.draft) {
      args.push("--draft");
    }
    if (options.force) {
      args.push("--force");
    }
    if (options.title) {
      args.push("--title", options.title);
    }

    const { stdout } = await this.execute(args);
    return stdout;
  }

  /**
   * Navigate to next (child) branch.
   */
  async next(): Promise<string> {
    const { stdout } = await this.execute(["nxt"]);
    return stdout;
  }

  /**
   * Navigate to previous (parent) branch.
   */
  async prev(): Promise<string> {
    const { stdout } = await this.execute(["prv"]);
    return stdout;
  }

  /**
   * Create a new branch in the stack.
   */
  async create(name: string): Promise<string> {
    const { stdout } = await this.execute(["create", name]);
    return stdout;
  }

  /**
   * Run doctor diagnostics.
   */
  async doctor(): Promise<string> {
    const { stdout } = await this.execute(["doctor"]);
    return stdout;
  }

  /**
   * Check if rung is initialized in the current repository.
   */
  async isInitialized(): Promise<boolean> {
    try {
      await this.status();
      return true;
    } catch (error: unknown) {
      if (isRungError(error) && error.type === ErrorType.NotInitialized) {
        return false;
      }
      // Re-throw other errors
      throw error;
    }
  }
}
