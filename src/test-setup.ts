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

// jsdom 不实现 ResizeObserver，OverflowTip 等组件依赖它做溢出检测。
// 提供一个最小桩：observe 时不触发回调（初始检测已在 useEffect 中同步执行）。
class ResizeObserverStub {
  constructor(callback: ResizeObserverCallback) { void callback; }
  observe(target: Element, options?: ResizeObserverOptions): void { void target; void options; }
  unobserve(target: Element): void { void target; }
  disconnect(): void { /* no-op */ }
}
if (!('ResizeObserver' in globalThis)) {
  (globalThis as { ResizeObserver: typeof ResizeObserverStub }).ResizeObserver = ResizeObserverStub;
}

afterEach(cleanup);
