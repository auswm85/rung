import { execFile } from "child_process";
import { promisify } from "util";

const execFileAsync = promisify(execFile);

/**
 * Execute a git command safely without shell interpolation.
 * Prevents command injection by passing arguments as an array.
 */
export async function gitExec(
  args: string[],
  cwd: string
): Promise<{ stdout: string; stderr: string }> {
  return execFileAsync("git", args, { cwd });
}
