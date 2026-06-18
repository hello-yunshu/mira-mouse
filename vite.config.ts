// SPDX-License-Identifier: AGPL-3.0-or-later
import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { strictPort: true },
  test: { environment: 'jsdom', setupFiles: ['./src/test-setup.ts'] },
});
