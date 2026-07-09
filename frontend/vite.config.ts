import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'path';

// The production build is served as a GitHub Pages *project* site under
// /fkst-hosted/, so CI builds with VITE_BASE=/fkst-hosted/. Dev, preview, and
// tests stay at root ('/'). If a custom domain is added later, drop VITE_BASE
// (or set it to '/') so assets resolve from the domain root.
export default defineConfig({
  base: process.env.VITE_BASE || '/',
  plugins: [react()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  server: {
    port: 3000,
  },
});
