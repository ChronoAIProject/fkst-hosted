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
    // running backend. Production gets the same effect from the shared
    // ingress (see k8s_sample/ingress.yaml).
    proxy: {
      '/api': 'http://localhost:8080',
    },
  },
});
