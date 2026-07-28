// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useState, type RefObject } from 'react';

/**
 * 检测滚动容器内容是否溢出（即是否可以滚动）。
 *
 * 仅当 scrollHeight > clientHeight 时返回 true，用于条件性应用
 * mask-image 渐变淡出，避免内容未溢出时底部被遮挡。
 *
 * 通过两个 Observer 协同工作：
 * - ResizeObserver：监听容器自身尺寸变化（如窗口缩放、flex 布局重排）
 * - MutationObserver (subtree)：监听容器内任意 DOM 变化（React 重新渲染、
 *   数据加载后内容增减），触发重新检测
 *
 * 无依赖数组：每次渲染后都会重新检查 ref.current 是否可用，
 * 确保条件渲染的元素（如弹窗打开后才挂载的滚动区）能正确初始化。
 */
export function useScrollOverflow(ref: RefObject<HTMLElement | null>): boolean {
  const [overflow, setOverflow] = useState(false);

  // 故意无依赖数组：每次渲染后重新检查 ref.current 是否可用，
  // 确保条件渲染的元素（如弹窗打开后才挂载的滚动区）能正确初始化。
  // setOverflow 是 useState setter（引用稳定），不会导致无限更新链。
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => {
    const el = ref.current;
    if (!el) {
      // 元素未挂载（弹窗关闭等），重置为 false
      setOverflow(false);
      return;
    }

    const check = () => {
      setOverflow(el.scrollHeight > el.clientHeight + 1);
    };

    check();

    const ro = new ResizeObserver(check);
    ro.observe(el);

    const mo = new MutationObserver(check);
    mo.observe(el, { childList: true, subtree: true });

    return () => {
      ro.disconnect();
      mo.disconnect();
    };
  });

  return overflow;
}
