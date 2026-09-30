import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Tauri 会在开发时连 http://localhost:1420，端口必须固定。
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: {
    target: 'es2022',
    minify: 'esbuild',
    sourcemap: false,
    // Radix 和 React Flow 加起来就是个不小的包。这是本地应用，从磁盘加载，
    // 没必要为了压到 500 kB 以下去拆包，把警告阈值调上去就好。
    chunkSizeWarningLimit: 800,
  },
});
