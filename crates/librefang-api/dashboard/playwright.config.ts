import { createConnection } from "node:net";
import { createHash } from "node:crypto";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "@playwright/test";

/**
 * The port this checkout's dev server listens on.
 *
 * Fixed at 4173 before, and that was a trap. `reuseExistingServer` below
 * reuses *whatever* is listening, so a dev server started in another worktree
 * kept serving that worktree's code while the spec under test came from this
 * one. The mixed state it produces (one tree's fix with another tree's
 * locators) exists in no commit, so two people measured incompatible numbers
 * from the same run and neither was wrong.
 *
 * Derived from this config's own directory, so the port follows the checkout
 * rather than the machine. `LIBREFANG_E2E_PORT` overrides it for a caller that
 * needs a known port.
 *
 * The spread is the point: 9000 values make a collision between unrelated
 * worktrees unlikely rather than merely possible — a 400-wide space is a
 * birthday problem, and at twenty-odd worktrees on this machine that is a
 * coin-toss. The range sits below Linux's ephemeral range (32768-60999) so a
 * client socket cannot take a port out from under a server, and clear of the
 * other fixed ports in this repo: 4173 and 4174 (the `web/` app's e2e), 4545
 * (the daemon the dev server proxies to, which this suite must never contend
 * with) and 5173 (vite's own default).
 */
const CHECKOUT = dirname(fileURLToPath(import.meta.url));
const PORT =
  Number(process.env.LIBREFANG_E2E_PORT) ||
  10000 + (parseInt(createHash("sha256").update(CHECKOUT).digest("hex").slice(0, 8), 16) % 9000);
const ORIGIN = `http://127.0.0.1:${PORT}`;

/** Whether something is already listening on `port`. */
const isPortBusy = (port: number): Promise<boolean> =>
  new Promise((resolve) => {
    const socket = createConnection({ host: "127.0.0.1", port });
    const settle = (busy: boolean): void => {
      socket.destroy();
      resolve(busy);
    };
    socket.setTimeout(500);
    socket.on("connect", () => settle(true));
    socket.on("timeout", () => settle(false));
    socket.on("error", () => settle(false));
  });

// A wide port space makes a collision unlikely; this makes it harmless. With
// `reuseExistingServer` set, a busy port is not an error to Playwright — it
// simply reuses what is there, which is how one worktree ends up being
// measured through another's code. Failing instead costs the operator one
// command.
//
// The marker is what keeps the check from firing on this run's own server:
// Playwright evaluates the config once per process — the main one that starts
// the webServer, then one per worker — and the later evaluations happen after
// the port is legitimately in use. The first evaluation sets the variable and
// its children inherit it. Without it the check fails three of four spec files
// against a server it started itself.
if (!process.env.CI && !process.env.LIBREFANG_E2E_PORT_CHECKED && (await isPortBusy(PORT))) {
  throw new Error(
    `Port ${PORT} already has a listener, and this run would reuse it rather ` +
      `than start its own — which is how a suite ends up testing another ` +
      `checkout's code. Stop whatever is on it (a dev server left behind by an ` +
      `interrupted run is the usual cause), or give this run a port of its ` +
      `own with LIBREFANG_E2E_PORT=<port>.`,
  );
}
process.env.LIBREFANG_E2E_PORT_CHECKED = "1";

export default defineConfig({
  testDir: "./e2e",
  timeout: 30000,
  use: {
    baseURL: ORIGIN,
    trace: "on-first-retry"
  },
  webServer: {
    // Run the long-lived server as vite itself, not through `pnpm dev`: pnpm 12 starts script children in their own process group, so Playwright's group kill at teardown misses vite and the run hangs on its open stdout.
    command: `vite --host 127.0.0.1 --port ${PORT}`,
    port: PORT,
    reuseExistingServer: !process.env.CI,
    cwd: "."
  }
});
