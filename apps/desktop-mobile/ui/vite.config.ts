import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// In development the UI talks to `desktop-dev-server` (the dev HTTP bridge); inside Tauri it
// talks to the app core through `invoke` and the `shoebox://` URI scheme (see src/lib/api.ts).
const bridge = process.env.SHOEBOX_BRIDGE ?? 'http://127.0.0.1:1421';

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    proxy: {
      '/api': bridge,
      '/events': { target: bridge, changeOrigin: true },
      '/blob': bridge,
    },
  },
  build: { target: 'es2022', sourcemap: true },
  test: { environment: 'node', include: ['src/**/*.test.ts'] },
});
