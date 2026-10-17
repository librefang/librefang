import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  test: {
    environment: "jsdom",
    globals: true,
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["src/setupTests.ts"],
    // Vitest's 5000ms default is wall-clock, and these tests are slow for reasons that have nothing to do with the behaviour they assert: several of them render the whole agent manifest form through jsdom, and `userEvent` awaits a task between every event, so a handful of clicks costs seconds. Measured on the tier picker: 1.5-2.7s per test on an idle machine, 5.3s when the suite's parallel workers are competing, which turns the default into a coin flip on a loaded runner. A budget is only meaningful when the failure it reports is the test's fault.
    testTimeout: 20_000
  }
});
