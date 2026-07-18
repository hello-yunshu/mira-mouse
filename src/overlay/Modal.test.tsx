// SPDX-License-Identifier: AGPL-3.0-or-later
import { act, fireEvent, render, screen, cleanup } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Modal, OVERLAY_ROOT_ID } from './index';

describe('Modal', () => {
  beforeEach(() => {
    document.getElementById(OVERLAY_ROOT_ID)?.remove();
  });

  afterEach(() => {
    cleanup();
    document.getElementById(OVERLAY_ROOT_ID)?.remove();
  });

  it('open=false 时不渲染', () => {
    const { container } = render(
      <Modal open={false} onClose={() => {}} title="标题">
        <p>内容</p>
      </Modal>,
    );
    expect(container.firstChild).toBeNull();
    expect(screen.queryByText('内容')).toBeNull();
  });

  it('open=true 时渲染到 Overlay Root', () => {
    render(
      <Modal open onClose={() => {}} title="标题">
        <p>内容</p>
      </Modal>,
    );
    const root = document.getElementById(OVERLAY_ROOT_ID);
    expect(root).not.toBeNull();
    expect(root?.textContent).toContain('内容');
    const dialog = screen.getByRole('dialog', { name: '标题' });
    expect(dialog).toBeDefined();
    // title 只提供无障碍名称，不再额外注入一个隐藏的同名文本节点。
    expect(screen.queryByText('标题')).toBeNull();
  });

  it('Escape 调用 onClose', () => {
    const onClose = vi.fn();
    render(
      <Modal open onClose={onClose} title="标题">
        <p>内容</p>
      </Modal>,
    );
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('点击遮罩调用 onClose', () => {
    const onClose = vi.fn();
    render(
      <Modal open onClose={onClose} title="标题">
        <p>内容</p>
      </Modal>,
    );
    const backdrop = document.querySelector('.modal-backdrop');
    expect(backdrop).not.toBeNull();
    // 模拟点击遮罩本身（而非内部内容）
    fireEvent.mouseDown(backdrop as Element);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('点击内容不关闭', () => {
    const onClose = vi.fn();
    render(
      <Modal open onClose={onClose} title="标题">
        <p>内容</p>
      </Modal>,
    );
    const surface = screen.getByRole('dialog');
    fireEvent.mouseDown(surface);
    expect(onClose).not.toHaveBeenCalled();
  });

  it('打开后焦点进入弹窗', async () => {
    render(
      <Modal open onClose={() => {}} title="标题">
        <button>确认</button>
      </Modal>,
    );
    const dialog = screen.getByRole('dialog');
    // Modal 用 requestAnimationFrame 延迟聚焦，等待 rAF 执行后再断言。
    await act(async () => {
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    });
    const active = document.activeElement;
    expect(dialog.contains(active)).toBe(true);
  });

  it('关闭后焦点回到触发按钮', async () => {
    const TriggerApp = ({ open }: { open: boolean }) => (
      <>
        <button data-testid="trigger">触发</button>
        <Modal open={open} onClose={() => {}} title="标题">
          <p>内容</p>
        </Modal>
      </>
    );
    const { rerender } = render(<TriggerApp open={false} />);
    const trigger = screen.getByTestId('trigger');
    // 先聚焦触发按钮（此时 Modal 未打开，previousFocus 不会被捕获）
    trigger.focus();
    expect(document.activeElement).toBe(trigger);
    // 打开 Modal — useLayoutEffect 捕获 trigger 为 previousFocus，rAF 把焦点移入弹窗
    rerender(<TriggerApp open={true} />);
    await act(async () => {
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    });
    expect(screen.getByRole('dialog').contains(document.activeElement)).toBe(true);
    // 关闭 Modal — cleanup 在 rAF 中恢复焦点到 trigger
    rerender(<TriggerApp open={false} />);
    await act(async () => {
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    });
    expect(document.activeElement).toBe(trigger);
  });

  it('Tab 不逃出弹窗', () => {
    render(
      <Modal open onClose={() => {}} title="标题">
        <button data-testid="first">第一个</button>
        <button data-testid="last">最后一个</button>
      </Modal>,
    );
    const first = screen.getByTestId('first');
    const last = screen.getByTestId('last');
    // 聚焦最后一个按钮，按 Tab 应循环到第一个
    last.focus();
    expect(document.activeElement).toBe(last);
    fireEvent.keyDown(document, { key: 'Tab' });
    expect(document.activeElement).toBe(first);
    // 在第一个按钮上按 Shift+Tab 应循环到最后一个
    fireEvent.keyDown(document, { key: 'Tab', shiftKey: true });
    expect(document.activeElement).toBe(last);
  });

  it('打开后背景 #root 设置 inert 与 aria-hidden', () => {
    const rootDiv = document.createElement('div');
    rootDiv.id = 'root';
    document.body.appendChild(rootDiv);
    try {
      render(
        <Modal open onClose={() => {}} title="标题">
          <p>内容</p>
        </Modal>,
        { container: rootDiv },
      );
      const root = document.getElementById('root');
      expect(root?.hasAttribute('inert')).toBe(true);
      expect(root?.getAttribute('aria-hidden')).toBe('true');
    } finally {
      cleanup();
      rootDiv.remove();
    }
  });

  it('卸载后清理 #root 的 inert 与 aria-hidden', () => {
    const rootDiv = document.createElement('div');
    rootDiv.id = 'root';
    document.body.appendChild(rootDiv);
    try {
      const { rerender } = render(
        <Modal open onClose={() => {}} title="标题">
          <p>内容</p>
        </Modal>,
        { container: rootDiv },
      );
      expect(rootDiv.hasAttribute('inert')).toBe(true);
      // 关闭 Modal
      rerender(
        <Modal open={false} onClose={() => {}} title="标题">
          <p>内容</p>
        </Modal>,
      );
      expect(rootDiv.hasAttribute('inert')).toBe(false);
      expect(rootDiv.hasAttribute('aria-hidden')).toBe(false);
    } finally {
      cleanup();
      rootDiv.remove();
    }
  });
});
