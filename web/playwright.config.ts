import { defineConfig } from '@playwright/test'

export default defineConfig({
  testDir: './e2e',
  timeout: 30000,
  retries: 0,
  fullyParallel: true,
  use: {
    baseURL: 'http://127.0.0.1:4174',
    trace: 'retain-on-failure',
    viewport: { width: 1280, height: 720 },
  },
  webServer: {
    // Run the long-lived server as vite itself, not through `pnpm preview`: pnpm 12 starts script children in their own process group, so Playwright's group kill at teardown misses vite and the run hangs on its open stdout.
    command: 'pnpm build && vite preview --host 127.0.0.1 --port 4174',
    port: 4174,
    reuseExistingServer: !process.env.CI,
    timeout: 180_000,
  },
})
