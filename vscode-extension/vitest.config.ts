import { defineConfig } from "vitest/config";
import { fileURLToPath } from "node:url";

export default defineConfig({
  test: {
    globals: true,
    environment: "node",
    include: ["src/**/*.test.ts"],
    alias: {
      vscode: fileURLToPath(
        new URL("./src/test/__mocks__/vscode.ts", import.meta.url)
      ),
    },
  },
});
