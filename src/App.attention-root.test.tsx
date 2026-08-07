// SPDX-License-Identifier: AGPL-3.0-or-later
// 根节点 Attention 能力类测试（§16）：
// 不要把 JSDOM 的实际 CSS 能力当成生产 WebView，全部通过 mock 检测结果驱动。

import { render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';

const { detectAttentionVisualSupportMock } = vi.hoisted(() => ({
  detectAttentionVisualSupportMock: vi.fn(),
}));

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

vi.mock('./attention', async (importOriginal) => {
  const original = await importOriginal<typeof import('./attention')>();
  return {
    ...original,
    detectAttentionVisualSupport: detectAttentionVisualSupportMock,
  };
});

describe('Attention 根节点能力类（§16）', () => {
  beforeEach(() => {
    invokeMock.mockRejectedValue(new Error('not mocked'));
  });

  afterEach(() => {
    invokeMock.mockReset();
    detectAttentionVisualSupportMock.mockReset();
    document.documentElement.classList.remove('attention-full-line-supported');
    document.documentElement.classList.remove('attention-color-mix-supported');
  });

  it('full support → html 挂载 attention-full-line-supported', () => {
    detectAttentionVisualSupportMock.mockReturnValue({
      registeredCustomProperty: true,
      colorMix: true,
      maskComposite: true,
      fullLineBeam: true,
    });
    render(<App />);
    expect(document.documentElement).toHaveClass('attention-full-line-supported');
    expect(document.documentElement).toHaveClass('attention-color-mix-supported');
  });

  it('no support → className 不存在', () => {
    detectAttentionVisualSupportMock.mockReturnValue({
      registeredCustomProperty: false,
      colorMix: false,
      maskComposite: false,
      fullLineBeam: false,
    });
    render(<App />);
    expect(document.documentElement).not.toHaveClass('attention-full-line-supported');
    expect(document.documentElement).not.toHaveClass('attention-color-mix-supported');
  });

  it('卸载时移除 class（自然还原）', () => {
    detectAttentionVisualSupportMock.mockReturnValue({
      registeredCustomProperty: true,
      colorMix: true,
      maskComposite: true,
      fullLineBeam: true,
    });
    const { unmount } = render(<App />);
    expect(document.documentElement).toHaveClass('attention-full-line-supported');
    unmount();
    expect(document.documentElement).not.toHaveClass('attention-full-line-supported');
    expect(document.documentElement).not.toHaveClass('attention-color-mix-supported');
  });
});