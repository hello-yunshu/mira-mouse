// SPDX-License-Identifier: AGPL-3.0-or-later
import { render, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { useEffect, useLayoutEffect, useRef, type ReactNode } from 'react';
import { useScrollFadeState, useScrollOverflow, type ScrollFadeState } from './useScrollOverflow';

/**
 * useScrollFadeState / useScrollOverflow 通用滚动淡出检测测试。
 *
 * jsdom 不实现真实布局，scrollHeight/clientHeight 默认为 0，因此通过
 * Object.defineProperty 在容器上注入可控的滚动度量，再触发 scroll 事件
 * 让 hook 重新测量，最后断言 hook 返回的状态。
 *
 * 使用真实计时器：rAF 在 ~16ms 后自然触发，配合 waitFor 等待状态更新。
 */

/** 容器度量的可变引用，便于测试中动态修改。 */
interface ScrollMetrics {
  scrollHeight: number;
  clientHeight: number;
  scrollTop: number;
}

/** 给元素注入可控的 scrollHeight/clientHeight/scrollTop getter/setter。 */
function injectScrollMetrics(el: HTMLElement, metrics: ScrollMetrics): void {
  Object.defineProperty(el, 'scrollHeight', {
    configurable: true,
    get: () => metrics.scrollHeight,
  });
  Object.defineProperty(el, 'clientHeight', {
    configurable: true,
    get: () => metrics.clientHeight,
  });
  Object.defineProperty(el, 'scrollTop', {
    configurable: true,
    get: () => metrics.scrollTop,
    set: (v: number) => { metrics.scrollTop = v; },
  });
}

/** 触发 scroll 事件让 hook 重新测量。 */
function triggerScroll(el: HTMLElement): void {
  el.dispatchEvent(new Event('scroll', { bubbles: false }));
}

/** 测试用组件：使用 useScrollFadeState，并通过 onState 把最新状态外抛。
 *  使用 useLayoutEffect 注入度量，确保在 hook 的 useEffect 之前完成。 */
function ScrollFadeHarness({
  metrics,
  onState,
  useContentRef = true,
  children,
}: {
  metrics: ScrollMetrics;
  onState?: (s: ScrollFadeState) => void;
  useContentRef?: boolean;
  children?: ReactNode;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  // 使用 useLayoutEffect 在 DOM 变更后同步注入度量，先于 useEffect 执行
  useLayoutEffect(() => {
    const el = containerRef.current;
    if (el) injectScrollMetrics(el, metrics);
  }, [metrics]);
  const state = useScrollFadeState(containerRef, useContentRef ? contentRef : undefined);
  useEffect(() => { onState?.(state); }, [state, onState]);
  return (
    <div ref={containerRef} className="scroll-container" style={{ overflowY: 'auto' }}>
      {useContentRef ? (
        <div ref={contentRef} className="scroll-content">{children}</div>
      ) : (
        children
      )}
    </div>
  );
}

/** 兼容旧 API 的测试组件。 */
function OverflowHarness({
  metrics,
  onOverflow,
}: {
  metrics: ScrollMetrics;
  onOverflow?: (v: boolean) => void;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const el = containerRef.current;
    if (el) injectScrollMetrics(el, metrics);
  }, [metrics]);
  const overflow = useScrollOverflow(containerRef);
  useEffect(() => { onOverflow?.(overflow); }, [overflow, onOverflow]);
  return <div ref={containerRef} className="scroll-container" />;
}

describe('useScrollFadeState', () => {
  it('scrollHeight == clientHeight → 无淡出', async () => {
    const metrics: ScrollMetrics = { scrollHeight: 100, clientHeight: 100, scrollTop: 0 };
    const states: ScrollFadeState[] = [];
    const onState = (s: ScrollFadeState) => { states.push(s); };
    render(<ScrollFadeHarness metrics={metrics} onState={onState} />);
    await waitFor(() => expect(states.at(-1)?.overflow).toBe(false));
    expect(states.at(-1)).toEqual({ overflow: false, canScrollUp: false, canScrollDown: false });
  });

  it('scrollHeight < clientHeight → 无淡出', async () => {
    const metrics: ScrollMetrics = { scrollHeight: 50, clientHeight: 100, scrollTop: 0 };
    const states: ScrollFadeState[] = [];
    const onState = (s: ScrollFadeState) => { states.push(s); };
    render(<ScrollFadeHarness metrics={metrics} onState={onState} />);
    await waitFor(() => expect(states.at(-1)?.overflow).toBe(false));
    expect(states.at(-1)).toEqual({ overflow: false, canScrollUp: false, canScrollDown: false });
  });

  it('初始溢出，内容缩短 → 淡出移除', async () => {
    const metrics: ScrollMetrics = { scrollHeight: 200, clientHeight: 100, scrollTop: 0 };
    const states: ScrollFadeState[] = [];
    const onState = (s: ScrollFadeState) => { states.push(s); };
    const { container } = render(<ScrollFadeHarness metrics={metrics} onState={onState} />);
    await waitFor(() => expect(states.at(-1)?.overflow).toBe(true));
    // 内容缩短到不溢出
    metrics.scrollHeight = 100;
    const el = container.querySelector('.scroll-container') as HTMLElement;
    triggerScroll(el);
    await waitFor(() => expect(states.at(-1)?.overflow).toBe(false));
    expect(states.at(-1)).toEqual({ overflow: false, canScrollUp: false, canScrollDown: false });
  });

  it('初始不溢出，内容增长 → 底部淡出出现', async () => {
    const metrics: ScrollMetrics = { scrollHeight: 100, clientHeight: 100, scrollTop: 0 };
    const states: ScrollFadeState[] = [];
    const onState = (s: ScrollFadeState) => { states.push(s); };
    const { container } = render(<ScrollFadeHarness metrics={metrics} onState={onState} />);
    await waitFor(() => expect(states.at(-1)?.overflow).toBe(false));
    // 内容增长到溢出
    metrics.scrollHeight = 250;
    const el = container.querySelector('.scroll-container') as HTMLElement;
    triggerScroll(el);
    // 在顶部：canScrollUp=false, canScrollDown=true
    await waitFor(() => expect(states.at(-1)?.overflow).toBe(true));
    expect(states.at(-1)).toEqual({ overflow: true, canScrollUp: false, canScrollDown: true });
  });

  it('顶部 → 只有底部淡出（canScrollUp=false, canScrollDown=true）', async () => {
    const metrics: ScrollMetrics = { scrollHeight: 300, clientHeight: 100, scrollTop: 0 };
    const states: ScrollFadeState[] = [];
    const onState = (s: ScrollFadeState) => { states.push(s); };
    render(<ScrollFadeHarness metrics={metrics} onState={onState} />);
    await waitFor(() => expect(states.at(-1)?.overflow).toBe(true));
    expect(states.at(-1)).toEqual({ overflow: true, canScrollUp: false, canScrollDown: true });
  });

  it('中间 → 上下都有（canScrollUp=true, canScrollDown=true）', async () => {
    const metrics: ScrollMetrics = { scrollHeight: 300, clientHeight: 100, scrollTop: 0 };
    const states: ScrollFadeState[] = [];
    const onState = (s: ScrollFadeState) => { states.push(s); };
    const { container } = render(<ScrollFadeHarness metrics={metrics} onState={onState} />);
    // 初始 scrollTop=0，先到达顶部
    await waitFor(() => expect(states.at(-1)?.canScrollUp).toBe(false));
    // 滚到中间
    metrics.scrollTop = 100;
    const el = container.querySelector('.scroll-container') as HTMLElement;
    triggerScroll(el);
    await waitFor(() => expect(states.at(-1)?.canScrollUp).toBe(true));
    expect(states.at(-1)).toEqual({ overflow: true, canScrollUp: true, canScrollDown: true });
  });

  it('底部 → 只有顶部淡出（canScrollUp=true, canScrollDown=false）', async () => {
    const metrics: ScrollMetrics = { scrollHeight: 300, clientHeight: 100, scrollTop: 0 };
    const states: ScrollFadeState[] = [];
    const onState = (s: ScrollFadeState) => { states.push(s); };
    const { container } = render(<ScrollFadeHarness metrics={metrics} onState={onState} />);
    // 初始 scrollTop=0
    await waitFor(() => expect(states.at(-1)?.canScrollDown).toBe(true));
    // 滚到底部：scrollTop + clientHeight = scrollHeight
    metrics.scrollTop = 200;
    const el = container.querySelector('.scroll-container') as HTMLElement;
    triggerScroll(el);
    await waitFor(() => expect(states.at(-1)?.canScrollDown).toBe(false));
    expect(states.at(-1)).toEqual({ overflow: true, canScrollUp: true, canScrollDown: false });
  });

  it('亚像素容差：scrollTop=0.5 不视为 canScrollUp=true', async () => {
    const metrics: ScrollMetrics = { scrollHeight: 300, clientHeight: 100, scrollTop: 0 };
    const states: ScrollFadeState[] = [];
    const onState = (s: ScrollFadeState) => { states.push(s); };
    const { container } = render(<ScrollFadeHarness metrics={metrics} onState={onState} />);
    await waitFor(() => expect(states.at(-1)?.canScrollUp).toBe(false));
    // 极小的 scrollTop 仍视为顶部
    metrics.scrollTop = 0.5;
    const el = container.querySelector('.scroll-container') as HTMLElement;
    triggerScroll(el);
    // 等待一下确保没有误判为 canScrollUp
    await new Promise((r) => setTimeout(r, 50));
    expect(states.at(-1)).toEqual({ overflow: true, canScrollUp: false, canScrollDown: true });
  });

  it('亚像素容差：scrollHeight 仅比 clientHeight 大 0.5 不视为溢出', async () => {
    const metrics: ScrollMetrics = { scrollHeight: 100.5, clientHeight: 100, scrollTop: 0 };
    const states: ScrollFadeState[] = [];
    const onState = (s: ScrollFadeState) => { states.push(s); };
    render(<ScrollFadeHarness metrics={metrics} onState={onState} />);
    await waitFor(() => expect(states.length).toBeGreaterThan(0));
    expect(states.at(-1)?.overflow).toBe(false);
  });

  it('不传 contentRef 时仍能正常工作', async () => {
    const metrics: ScrollMetrics = { scrollHeight: 200, clientHeight: 100, scrollTop: 0 };
    const states: ScrollFadeState[] = [];
    const onState = (s: ScrollFadeState) => { states.push(s); };
    render(<ScrollFadeHarness metrics={metrics} onState={onState} useContentRef={false} />);
    await waitFor(() => expect(states.at(-1)?.overflow).toBe(true));
    expect(states.at(-1)).toEqual({ overflow: true, canScrollUp: false, canScrollDown: true });
  });

  it('内容 DOM 变化触发 MutationObserver 重测', async () => {
    const metrics: ScrollMetrics = { scrollHeight: 100, clientHeight: 100, scrollTop: 0 };
    const states: ScrollFadeState[] = [];
    const onState = (s: ScrollFadeState) => { states.push(s); };
    const { container } = render(<ScrollFadeHarness metrics={metrics} onState={onState} />);
    // 等待初始测量完成
    await waitFor(() => expect(states.at(-1)?.overflow).toBe(false));
    // 通过修改度量 + 触发 DOM 变化（MutationObserver 捕获）让 hook 重测
    metrics.scrollHeight = 250;
    const content = container.querySelector('.scroll-content') as HTMLElement;
    // 在内容包装层内新增一个子节点，触发 MutationObserver
    const child = document.createElement('div');
    child.textContent = 'extra content';
    content.appendChild(child);
    // 等待 MutationObserver + rAF 完成
    await waitFor(() => expect(states.at(-1)?.overflow).toBe(true), { timeout: 1500 });
    // 在顶部
    expect(states.at(-1)).toEqual({ overflow: true, canScrollUp: false, canScrollDown: true });
  });

  it('transitionend 事件触发重新测量', async () => {
    const metrics: ScrollMetrics = { scrollHeight: 100, clientHeight: 100, scrollTop: 0 };
    const states: ScrollFadeState[] = [];
    const onState = (s: ScrollFadeState) => { states.push(s); };
    const { container } = render(<ScrollFadeHarness metrics={metrics} onState={onState} />);
    await waitFor(() => expect(states.at(-1)?.overflow).toBe(false));
    // 内容增长（模拟 CSS height transition 结束后的新高度）
    metrics.scrollHeight = 250;
    const el = container.querySelector('.scroll-container') as HTMLElement;
    // 触发 transitionend 事件（target 为容器内的子元素）
    const child = el.querySelector('.scroll-content') as HTMLElement;
    child.dispatchEvent(new TransitionEvent('transitionend', { bubbles: true }));
    await waitFor(() => expect(states.at(-1)?.overflow).toBe(true), { timeout: 1500 });
    expect(states.at(-1)).toEqual({ overflow: true, canScrollUp: false, canScrollDown: true });
  });

  it('animationend 事件触发重新测量', async () => {
    const metrics: ScrollMetrics = { scrollHeight: 100, clientHeight: 100, scrollTop: 0 };
    const states: ScrollFadeState[] = [];
    const onState = (s: ScrollFadeState) => { states.push(s); };
    const { container } = render(<ScrollFadeHarness metrics={metrics} onState={onState} />);
    await waitFor(() => expect(states.at(-1)?.overflow).toBe(false));
    // 内容增长（模拟 CSS animation 结束后的新高度）
    metrics.scrollHeight = 250;
    const el = container.querySelector('.scroll-container') as HTMLElement;
    const child = el.querySelector('.scroll-content') as HTMLElement;
    child.dispatchEvent(new Event('animationend', { bubbles: true }));
    await waitFor(() => expect(states.at(-1)?.overflow).toBe(true), { timeout: 1500 });
    expect(states.at(-1)).toEqual({ overflow: true, canScrollUp: false, canScrollDown: true });
  });

  it('卸载后 listener/observer 清理无报错', async () => {
    const metrics: ScrollMetrics = { scrollHeight: 200, clientHeight: 100, scrollTop: 0 };
    const { container, unmount } = render(<ScrollFadeHarness metrics={metrics} />);
    // 等待 hook 初始化（rAF + ResizeObserver + MutationObserver 注册）
    await waitFor(() => {});
    await new Promise((r) => setTimeout(r, 50));
    // 卸载：cleanup 函数应正确断开 observer、移除 listener、取消 rAF
    expect(() => unmount()).not.toThrow();
    // 卸载后触发事件不应报错（listener 已移除）
    const el = container.querySelector('.scroll-container') as HTMLElement;
    if (el) {
      expect(() => {
        el.dispatchEvent(new Event('scroll'));
        window.dispatchEvent(new Event('resize'));
      }).not.toThrow();
    }
  });
});

describe('useScrollOverflow (向后兼容)', () => {
  it('委托 useScrollFadeState，仅返回 overflow 布尔', async () => {
    const metrics: ScrollMetrics = { scrollHeight: 200, clientHeight: 100, scrollTop: 0 };
    const overflows: boolean[] = [];
    const onOverflow = (v: boolean) => { overflows.push(v); };
    render(<OverflowHarness metrics={metrics} onOverflow={onOverflow} />);
    await waitFor(() => expect(overflows.at(-1)).toBe(true));
  });

  it('scrollHeight == clientHeight 时返回 false', async () => {
    const metrics: ScrollMetrics = { scrollHeight: 100, clientHeight: 100, scrollTop: 0 };
    const overflows: boolean[] = [];
    const onOverflow = (v: boolean) => { overflows.push(v); };
    render(<OverflowHarness metrics={metrics} onOverflow={onOverflow} />);
    await waitFor(() => expect(overflows.at(-1)).toBe(false));
  });
});
