import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  server: {
    port: 3002,
    host: true,
    allowedHosts: true,
  },
  build: {
    rollupOptions: {
      input: './index.html',
      output: {
        manualChunks: {
          'vendor-react': ['react', 'react-dom'],
          'vendor-motion': ['framer-motion'],
          'vendor-query': ['@tanstack/react-query'],
        },
      },
    },
  }
})
