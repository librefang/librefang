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
        // vite 8 uses rolldown which requires the function form of
        // manualChunks; the object form is rejected at build time.
        manualChunks(id: string) {
          if (id.includes('node_modules/react/') || id.includes('node_modules/react-dom/')) {
            return 'vendor-react'
          }
          if (id.includes('node_modules/framer-motion/')) {
            return 'vendor-motion'
          }
          if (id.includes('node_modules/@tanstack/react-query/')) {
            return 'vendor-query'
          }
          return undefined
        },
      },
    },
  }
})
