import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'path';

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  server: {
    port: 3000,
    // Dev convenience for the same-origin API contract: the SPA calls
    // relative /api/v1/* URLs, which the dev server forwards to a locally
    // running backend. A deployed same-origin topology gets the same effect
    // from a shared ingress fronting both services.
    proxy: {
      '/api': 'http://localhost:8080',
    },
  },
});
