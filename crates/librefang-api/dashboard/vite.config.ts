import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  base: "/react-assets/",
  server: {
    proxy: {
      "/api": {
        target: "http://127.0.0.1:4545",
        changeOrigin: true
      }
    }
  },
  build: {
    outDir: "../static/react",
    emptyOutDir: true
  }
});
