// SPDX-License-Identifier: AGPL-3.0-or-later
import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  resolve: {
    // thinking-orbs 的 git 依赖带了自己的 React 18，必须强制去重到宿主
    // React 19；否则它调用另一份 React 的 hooks，dispatcher 为 null，首屏白屏。
    dedupe: ['react', 'react-dom', 'react/jsx-runtime'],
  },
  build: {
    target: 'esnext',
    rollupOptions: {
      output: {
        // React 必须只存在一个运行时副本：App 与懒加载页共享的 chunk
        // 若各自打包一份 React，会得到同一个 null dispatcher，页面首屏白屏。
        manualChunks(id) {
          if (id.includes('node_modules/react/')
            || id.includes('node_modules/react-dom/')
            || id.includes('node_modules/scheduler/')) {
            return 'react-vendor';
          }
        },
      },
    },
  },
  test: { environment: 'jsdom', setupFiles: ['./src/test-setup.ts'] },
});
