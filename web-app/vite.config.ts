import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [sveltekit()],
  server: {
    port: Number(process.env.WEB_PORT ?? 5173)
  },
  preview: {
    port: Number(process.env.WEB_PORT ?? 4173)
  }
});
