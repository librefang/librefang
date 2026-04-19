import { defineConfig, createLogger } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath } from "node:url";
import { resolve, dirname } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const reactRoot = resolve(__dirname, "node_modules/react");
const reactDomRoot = resolve(__dirname, "node_modules/react-dom");

const logger = createLogger();
const origError = logger.error.bind(logger);
logger.error = (msg, opts) => {
  if (typeof msg === "string" && msg.includes("proxy error")) return;
  origError(msg, opts);
};

// Every React-consuming dep MUST be pre-bundled up-front so Vite doesn't
// re-optimize mid-session when a lazy route first pulls one. Re-optimization
// flips the `?v=xxx` hash, the browser loads the newly-hashed React chunk,
// and any already-loaded module still referencing the old hash now sees a
// DIFFERENT React instance — resulting in dispatcher=null / "Cannot read
// properties of null (reading 'useContext')" on hook calls.
const SINGLETON_DEPS = [
  "react",
  "react-dom",
  "react-dom/client",
  "react/jsx-runtime",
  "react/jsx-dev-runtime",
  "@tanstack/react-query",
  "@tanstack/react-router",
  "react-i18next",
  "i18next",
  "cmdk",
  "react-markdown",
  "recharts",
  "@xyflow/react",
  "zustand",
  "lucide-react",
  "lucide-react/dynamic",
];

export default defineConfig({
  customLogger: logger,
  plugins: [react(), tailwindcss()],
  base: "/dashboard/",
  resolve: {
    dedupe: SINGLETON_DEPS,
    // Force every `import "react"` / `import "react-dom"` to resolve to the
    // SAME absolute path regardless of which pnpm symlink chain requested it.
    // Without this, Vite's pre-bundler can treat paths that symlink to the
    // same file as distinct modules, producing multiple React instances at
    // runtime and breaking hook calls with "Cannot read properties of null
    // (reading 'useContext')".
    alias: [
      { find: /^react$/, replacement: reactRoot },
      { find: /^react\/(.*)$/, replacement: `${reactRoot}/$1` },
      { find: /^react-dom$/, replacement: reactDomRoot },
      { find: /^react-dom\/(.*)$/, replacement: `${reactDomRoot}/$1` },
    ],
  },
  optimizeDeps: {
    include: SINGLETON_DEPS,
  },
  server: {
    host: "0.0.0.0",
    allowedHosts: true,
    // Eagerly transform every lazy page at startup so any React-coupled
    // dep they drag in gets folded into the initial optimize pass instead
    // of triggering a mid-session re-optimization.
    warmup: {
      clientFiles: [
        "./src/main.tsx",
        "./src/App.tsx",
        "./src/pages/*.tsx",
      ],
    },
    proxy: {
      "/api": {
        target: "http://127.0.0.1:4545",
        changeOrigin: true,
        ws: true,
        timeout: 300_000,
        proxyTimeout: 300_000,
        configure: (proxy) => {
          type Emitter = { on(event: string, fn: (...args: never[]) => void): void };
          const p = proxy as unknown as Emitter;
          p.on("error", () => {});
          p.on("proxyReq", (proxyReq: Emitter) => { proxyReq.on("error", () => {}); });
          p.on("proxyRes", (proxyRes: Emitter) => { proxyRes.on("error", () => {}); });
        }
      }
    }
  },
  build: {
    outDir: "../static/react",
    emptyOutDir: true,
    rollupOptions: {
      output: {
        manualChunks: {
          vendor: ["react", "react-dom"],
          router: ["@tanstack/react-router", "@tanstack/react-query"],
          charts: ["recharts"],
          flow: ["@xyflow/react"],
        }
      }
    }
  }
});
