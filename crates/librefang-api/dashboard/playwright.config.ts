import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  timeout: 30000,
  use: {
    baseURL: "http://127.0.0.1:4173",
    trace: "on-first-retry"
  },
  webServer: {
    // Run the long-lived server as vite itself, not through `pnpm dev`: pnpm 12 starts script children in their own process group, so Playwright's group kill at teardown misses vite and the run hangs on its open stdout.
    command: "vite --host 127.0.0.1 --port 4173",
    port: 4173,
    reuseExistingServer: !process.env.CI,
    cwd: "."
  }
});
