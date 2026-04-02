import { defineConfig, createLogger } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const logger = createLogger();
const origError = logger.error.bind(logger);
logger.error = (msg, opts) => {
  if (typeof msg === "string" && msg.includes("proxy error")) return;
  origError(msg, opts);
};

export default defineConfig({
  customLogger: logger,
  plugins: [react(), tailwindcss()],
  base: "/dashboard/",
  server: {
    host: "0.0.0.0",
    allowedHosts: true,
    proxy: {
      "/api": {
        target: "http://127.0.0.1:4545",
        changeOrigin: true,
        ws: true,
        configure: (proxy) => {
          proxy.options.proxyTimeout = 300_000;
          proxy.options.timeout = 300_000;
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
