import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
export default defineConfig({
  plugins: [react()],
  esbuild: { jsx: 'automatic', include: /\.(ts|tsx|js|jsx)$/ },
  build: { rollupOptions: { external: (id) => id.startsWith('@tauri-apps/') } }
})
