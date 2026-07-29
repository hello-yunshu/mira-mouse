// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useRef, useState, type RefObject } from 'react';

/**
 * 滚动淡出状态：分别给出溢出、可向上、可向下三个布尔值，
 * 让调用方按当前位置条件应用顶部/底部 mask，避免内容整体变暗。
 */
export interface ScrollFadeState {
  /** 内容是否溢出容器（scrollHeight > clientHeight）。 */
  overflow: boolean;
  /** 当前不在顶部，向上滚动仍有内容。 */
  canScrollUp: boolean;
  /** 当前不在底部，向下滚动仍有内容。 */
  canScrollDown: boolean;
}

const EMPTY_STATE: ScrollFadeState = {
  overflow: false,
  canScrollUp: false,
  canScrollDown: false,
};

/** 亚像素容差：避免浏览器取整或舍入误差造成的误判。 */
const SCROLL_EPSILON = 1;

/**
 * 统一的滚动淡出检测 hook。
 *
 * 同时观察 container 与 content 两个 ref：
 * - container ResizeObserver：监听容器自身尺寸变化（窗口缩放、flex 重排）。
 * - content  ResizeObserver：监听内容包装层尺寸变化（loading 完成、文本变化、
 *   CSS height transition、insight 卡片数量变化、24h/10d 切换、modal 重排等）。
 * - scroll 事件：用户滚动时实时更新 canScrollUp/canScrollDown。
 * - window resize：补充容器外布局变化。
 * - transitionend / animationend：CSS transition/animation 结束后重新测量。
 * - MutationObserver：作为补充，监听子节点增删与属性变化。
 *
 * 测量合并到 requestAnimationFrame，避免连续 observer 抖动和读取中间布局。
 *
 * 判定（带 epsilon 容差）：
 * ```
 * overflow      = scrollHeight > clientHeight + epsilon
 * canScrollUp   = overflow && scrollTop > epsilon
 * canScrollDown = overflow && scrollTop + clientHeight < scrollHeight - epsilon
 * ```
 * 当 scrollHeight == clientHeight 时不显示任何淡出。
 *
 * 故意无依赖数组：每次渲染后重新检查 ref.current 是否可用，
 * 确保条件渲染的元素（如弹窗打开后才挂载的滚动区）能正确初始化。
 */
export function useScrollFadeState(
  containerRef: RefObject<HTMLElement | null>,
  contentRef?: RefObject<HTMLElement | null>,
): ScrollFadeState {
  const [state, setState] = useState<ScrollFadeState>(EMPTY_STATE);
  // rAF 句柄：合并多次 observer/事件触发的测量，避免抖动。
  const rafRef = useRef<number | null>(null);

  // 故意无依赖数组：每次渲染后重新检查 ref.current 是否可用，
  // 确保条件渲染的元素（如弹窗打开后才挂载的滚动区）能正确初始化。
  // setState 是 useState setter（引用稳定），不会导致无限更新链。
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => {
    const container = containerRef.current;
    if (!container) {
      setState(EMPTY_STATE);
      return;
    }
    const content = contentRef?.current ?? null;

    const measure = () => {
      if (rafRef.current !== null) return;
      rafRef.current = requestAnimationFrame(() => {
        rafRef.current = null;
        const el = containerRef.current;
        if (!el) return;
        const { scrollTop, scrollHeight, clientHeight } = el;
        const overflow = scrollHeight > clientHeight + SCROLL_EPSILON;
        const canScrollUp = overflow && scrollTop > SCROLL_EPSILON;
        const canScrollDown = overflow && scrollTop + clientHeight < scrollHeight - SCROLL_EPSILON;
        setState((prev) => {
          if (prev.overflow === overflow && prev.canScrollUp === canScrollUp && prev.canScrollDown === canScrollDown) {
            return prev;
          }
          return { overflow, canScrollUp, canScrollDown };
        });
      });
    };

    // 初次测量同步执行（合入下一帧）。
    measure();

    const ro = new ResizeObserver(measure);
    ro.observe(container);
    if (content) ro.observe(content);

    const handleScroll = () => measure();
    container.addEventListener('scroll', handleScroll, { passive: true });

    const handleResize = () => measure();
    window.addEventListener('resize', handleResize, { passive: true });

    // CSS height transition / animation 结束后内容高度可能变化，重新测量。
    const handleTransitionEnd = (event: TransitionEvent | AnimationEvent) => {
      const target = event.target;
      if (target instanceof Node && container.contains(target)) {
        measure();
      }
    };
    container.addEventListener('transitionend', handleTransitionEnd as EventListener);
    container.addEventListener('animationend', handleTransitionEnd as EventListener);

    // MutationObserver 作为补充：内容增删/属性变化（如 class、style）时重新测量。
    const mo = new MutationObserver(measure);
    mo.observe(container, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ['style', 'class'],
    });

    return () => {
      ro.disconnect();
      mo.disconnect();
      container.removeEventListener('scroll', handleScroll);
      window.removeEventListener('resize', handleResize);
      container.removeEventListener('transitionend', handleTransitionEnd as EventListener);
      container.removeEventListener('animationend', handleTransitionEnd as EventListener);
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    };
  });

  return state;
}

/**
 * 向后兼容：仅返回 overflow 布尔值。
 *
 * 旧调用方（已全部迁移到 useScrollFadeState）保留此导出避免破坏外部引用。
 * 内部委托 useScrollFadeState，行为与旧实现一致：scrollHeight > clientHeight 时为 true。
 */
export function useScrollOverflow(ref: RefObject<HTMLElement | null>): boolean {
  const { overflow } = useScrollFadeState(ref);
  return overflow;
}
