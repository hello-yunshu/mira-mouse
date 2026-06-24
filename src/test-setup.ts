// SPDX-License-Identifier: AGPL-3.0-or-later
import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';
import i18n from './i18n';

Object.defineProperty(window, '__TAURI_INTERNALS__', {
  configurable: true,
  value: {},
});

// Force Chinese for all tests so assertions based on Chinese text remain valid.
// Mock navigator.language so resolveLanguage('auto') returns 'zh-CN'.
Object.defineProperty(navigator, 'language', { configurable: true, value: 'zh-CN' });
Object.defineProperty(navigator, 'languages', { configurable: true, value: ['zh-CN', 'zh', 'en'] });
void i18n.changeLanguage('zh-CN');

afterEach(cleanup);
