import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  server: {
    port: 5173,
    proxy: Object.fromEntries(['/auth', '/status', '/notifications', '/settings', '/logs', '/wan', '/system', '/appearance', '/healthz', '/readyz'].map(path => [path, 'http://127.0.0.1:19002'])),
  },
  build: { target: 'es2022' },
})
