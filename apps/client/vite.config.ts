import { defineConfig } from 'vite';

export default defineConfig({
  server: {
    port: 5173,
    proxy: {
      // Forward REST calls to the Rust API during development.
      '/api': {
        target: 'http://localhost:3000',
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/api/, ''),
      },
    },
  },
  build: {
    target: 'es2022',
    // Phaser alone is ~1.4 MB minified; raise the limit so the warning only fires on real growth.
    chunkSizeWarningLimit: 2000,
  },
});
