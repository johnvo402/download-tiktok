import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  plugins: [solid(), tailwindcss()],
  clearScreen: false,
  server: {
    host: '127.0.0.1',
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ['**/src-tauri/target/**', '**/src-tauri/resources/**', '**/.app-data/**', '**/.release-cache/**', '**/.venv/**', '**/python/build/**', '**/python/dist/**', '**/python/.release*-venv/**', '**/release/**'],
    },
  },
  envPrefix: ['VITE_', 'TAURI_ENV_*'],
  build: { target: ['es2022', 'chrome100', 'safari13'] },
});
