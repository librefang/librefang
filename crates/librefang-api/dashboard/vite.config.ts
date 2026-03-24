import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  base: "/dashboard/",
  server: {
    host: "0.0.0.0",
    proxy: {
      "/api": {
        target: "http://127.0.0.1:4545",
        changeOrigin: true,
        configure: (proxy) => {
          proxy.on("error", () => {});
          proxy.on("proxyReq", (proxyReq) => { proxyReq.on("error", () => {}); });
          proxy.on("proxyRes", (proxyRes) => { proxyRes.on("error", () => {}); });
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
