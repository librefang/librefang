import { defineConfig, createLogger } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const logger = createLogger();
const origError = logger.error.bind(logger);
logger.error = (msg, opts) => {
  if (typeof msg === "string" && msg.includes("proxy error")) return;
  origError(msg, opts);
};

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
];

export default defineConfig({
  customLogger: logger,
  plugins: [react(), tailwindcss()],
  base: "/dashboard/",
  resolve: {
    dedupe: SINGLETON_DEPS,
  },
  optimizeDeps: {
    include: SINGLETON_DEPS,
  },
  server: {
    host: "0.0.0.0",
    allowedHosts: true,
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
