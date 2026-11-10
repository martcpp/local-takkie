import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vitest/config'

// Set by `tauri android dev`, so the phone can reach the dev server.
const host = process.env.TAURI_DEV_HOST

export default defineConfig({
  plugins: [svelte()],
  // Svelte's browser build, so tests can mount components.
  resolve: process.env.VITEST ? { conditions: ['browser'] } : undefined,
  test: { environment: 'happy-dom' },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
  },
})
