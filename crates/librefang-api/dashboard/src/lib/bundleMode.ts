// Bundle-mode bootstrap for the mobile Tauri shell.
//
// On mobile release builds, the Tauri shell loads the dashboard from the
// embedded `tauri://localhost/index.html` (set via `frontendDist` in
// `tauri.{ios,android}.conf.json`) instead of the remote daemon's HTTP
// origin. The dashboard JS still issues relative API requests like
// `fetch("/api/agents")` and `new WebSocket("/api/agents/.../events")`,
// which would resolve against `tauri://localhost` and fail.
//
// The shell encodes the daemon URL into the navigation target's hash
// (`tauri://localhost/index.html#api=<encoded>`); this module pulls it out,
// persists it to localStorage so reloads keep working, and patches
// `window.fetch` + `window.WebSocket` to rewrite same-origin paths onto
// the configured daemon base. CORS on the daemon must allow
// `tauri://localhost` for the rewritten requests to succeed.
//
// In thin-client mode (debug builds, desktop, or any non-Tauri page) this
// is a no-op — the bootstrap detects `window.location.protocol !== "tauri:"`
// or an unset base and bails out before touching globals.
//
// IMPORTANT: `setupBundleMode()` runs in `main.tsx` after the imports
// but before `createRoot().render()`. ESM evaluates module bodies in
// textual import order, so any module imported by `main.tsx` (or
// transitively) that issues a fetch / opens a WebSocket *during its own
// module evaluation* would beat the patch. Today none do
// (`react-i18next`, `@tanstack/react-query`, `@tanstack/react-router`,
// `i18next` w/ bundled JSON resources). Adding a backend-loaded i18n
// (`i18next-http-backend`) or any other eager-fetching module here will
// silently regress bundled mobile — keep the rule.

const BASE_KEY = "librefang-api-base";

const SAME_ORIGIN_PREFIXES = [
  "/api/",
  "/dashboard/",
  "/locales/",
  "/.well-known/",
  "/a2a/",
  "/connect",
  "/hooks/",
  "/mcp",
];

function isApiPath(path: string): boolean {
  if (path === "/") return true;
  return SAME_ORIGIN_PREFIXES.some((p) => path.startsWith(p));
}

function rewritePath(httpBase: string, path: string): string {
  return httpBase + path;
}

function maybeStripTauriOrigin(url: string): string | null {
  if (url.startsWith("tauri://localhost")) {
    return url.slice("tauri://localhost".length) || "/";
  }
  return null;
}

export function setupBundleMode(): void {
  if (typeof window === "undefined") return;
  if (window.location.protocol !== "tauri:") return;

  // Pull the daemon URL out of the hash injected by the mobile shell, store
  // it locally, and strip the hash so React Router doesn't try to interpret
  // it as a route.
  const hash = window.location.hash;
  if (hash.startsWith("#api=")) {
    const encoded = hash.slice("#api=".length);
    try {
      const decoded = decodeURIComponent(encoded);
      if (decoded) {
        localStorage.setItem(BASE_KEY, decoded);
      }
    } catch {
      // Malformed hash — ignore, fall back to whatever localStorage has.
    }
    history.replaceState(null, "", window.location.pathname + window.location.search);
  }

  const stored = (localStorage.getItem(BASE_KEY) || "").replace(/\/$/, "");
  if (!stored) return;

  const httpBase = stored;
  const wsBase = httpBase.replace(/^http(s?):\/\//i, (_m, s) => `ws${s}://`);

  const ORIG_FETCH = window.fetch.bind(window);
  window.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    let url: string | undefined;
    if (typeof input === "string") {
      url = input;
    } else if (input instanceof URL) {
      url = input.href;
    } else if (input instanceof Request) {
      url = input.url;
    }
    if (url) {
      if (url.startsWith("/") && isApiPath(url)) {
        return ORIG_FETCH(rewritePath(httpBase, url), init);
      }
      const stripped = maybeStripTauriOrigin(url);
      if (stripped && isApiPath(stripped)) {
        return ORIG_FETCH(rewritePath(httpBase, stripped), init);
      }
    }
    return ORIG_FETCH(input as RequestInfo, init);
  }) as typeof window.fetch;

  const ORIG_WS = window.WebSocket;
  // Replace the global with a subclass that rewrites relative /
  // `tauri://localhost`-bound URLs onto the daemon's ws/wss base before
  // delegating to the real constructor. `extends ORIG_WS` automatically
  // forwards the static constants (`OPEN` / `CONNECTING` / `CLOSING` /
  // `CLOSED`) that the dashboard reads as `WebSocket.OPEN` (TerminalPage,
  // ChatPage retry logic), and gives `instanceof WebSocket` for free —
  // both broken if we used a function constructor with `Object.assign`.
  class PatchedWS extends ORIG_WS {
    constructor(url: string | URL, protocols?: string | string[]) {
      let u = typeof url === "string" ? url : url.href;
      if (u.startsWith("/") && isApiPath(u)) {
        u = wsBase + u;
      } else if (u.startsWith("ws://localhost") || u.startsWith("wss://localhost")) {
        const path = u.replace(/^wss?:\/\/localhost/, "");
        if (isApiPath(path)) u = wsBase + path;
      } else {
        const stripped = maybeStripTauriOrigin(u);
        if (stripped && isApiPath(stripped)) {
          u = wsBase + stripped;
        }
      }
      super(u, protocols);
    }
  }
  window.WebSocket = PatchedWS as unknown as typeof WebSocket;
}
