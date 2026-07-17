// SPDX-License-Identifier: AGPL-3.0-or-later
import { act, render, screen, cleanup } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { OverlayPortal, OVERLAY_ROOT_ID } from './index';

describe('OverlayPortal', () => {
  beforeEach(() => {
    // 每个测试前清掉残留的 overlay root，确保起点干净。
    document.getElementById(OVERLAY_ROOT_ID)?.remove();
  });

  afterEach(() => {
    cleanup();
    document.getElementById(OVERLAY_ROOT_ID)?.remove();
  });

  it('自动创建 #mira-overlay-root', () => {
    expect(document.getElementById(OVERLAY_ROOT_ID)).toBeNull();
    render(
      <OverlayPortal>
        <span data-testid="child">hello</span>
      </OverlayPortal>,
    );
    const root = document.getElementById(OVERLAY_ROOT_ID);
    expect(root).not.toBeNull();
    expect(root?.dataset.miraOverlayRoot).toBe('true');
  });

  it('子节点挂载到 Overlay Root 而非 body 直接子节点', () => {
    render(
      <OverlayPortal>
        <span data-testid="child">hello</span>
      </OverlayPortal>,
    );
    const child = screen.getByTestId('child');
    const root = document.getElementById(OVERLAY_ROOT_ID);
    expect(root).not.toBeNull();
    expect(root?.contains(child)).toBe(true);
    // body 的直接子节点不包含 child（child 在 overlay root 内部）
    expect(child.parentElement?.closest(`#${OVERLAY_ROOT_ID}`)).toBe(root);
  });

  it('多个浮层不会重复创建 Root', () => {
    const { unmount: unmountFirst } = render(
      <OverlayPortal>
        <span data-testid="a">a</span>
      </OverlayPortal>,
    );
    render(
      <OverlayPortal>
        <span data-testid="b">b</span>
      </OverlayPortal>,
    );
    const roots = document.querySelectorAll(`#${OVERLAY_ROOT_ID}`);
    expect(roots).toHaveLength(1);
    unmountFirst();
  });

  it('最后一个浮层卸载后移除 Root，无残留', () => {
    const { unmount } = render(
      <OverlayPortal>
        <span data-testid="child">hello</span>
      </OverlayPortal>,
    );
    expect(document.getElementById(OVERLAY_ROOT_ID)).not.toBeNull();
    act(() => {
      unmount();
    });
    expect(document.getElementById(OVERLAY_ROOT_ID)).toBeNull();
  });

  it('仍有浮层时卸载其中一个，Root 保留', () => {
    const { unmount: unmountFirst } = render(
      <OverlayPortal>
        <span data-testid="a">a</span>
      </OverlayPortal>,
    );
    render(
      <OverlayPortal>
        <span data-testid="b">b</span>
      </OverlayPortal>,
    );
    act(() => {
      unmountFirst();
    });
    // 仍有第二个浮层，Root 必须保留
    expect(document.getElementById(OVERLAY_ROOT_ID)).not.toBeNull();
    expect(screen.getByTestId('b')).toBeDefined();
  });
});
