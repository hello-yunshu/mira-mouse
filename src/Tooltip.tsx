// SPDX-License-Identifier: AGPL-3.0-or-later
import { PropsWithChildren, useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

/**
 * Tooltip — 内容通过 React Portal 渲染到 document.body。
 *
 * 原因：trigger 嵌在 `.card` 内，而 `.card` 自身应用了 `backdrop-filter`，
 * 按 CSS 规范它成为一个 backdrop root，会截断后代 `backdrop-filter` 的取样范围。
 * 若 tooltip 直接作为 `.card` 的子节点，其 backdrop-filter 只能取到卡片内部近透明的层，
 * 毛玻璃效果不可见。Portal 到 body 后，tooltip 不再受任何 backdrop-root 祖先影响，
 * backdrop-filter 可以正确取到 macOS 系统级 Vibrancy 背景。
 */
export function Tooltip({ label, children }: PropsWithChildren<{ label: string }>) {
  const triggerRef = useRef<HTMLSpanElement>(null);
  const tooltipRef = useRef<HTMLSpanElement>(null);
  const [mounted, setMounted] = useState(false);
  const [visible, setVisible] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const hoverTimer = useRef<number | null>(null);
  const hideTimer = useRef<number | null>(null);
  const id = useId();

  const cancelHover = () => {
    if (hoverTimer.current != null) {
      window.clearTimeout(hoverTimer.current);
      hoverTimer.current = null;
    }
  };
  const cancelHide = () => {
    if (hideTimer.current != null) {
      window.clearTimeout(hideTimer.current);
      hideTimer.current = null;
    }
  };

  const measure = useCallback(() => {
    const trigger = triggerRef.current;
    const tip = tooltipRef.current;
    if (!trigger || !tip) return;
    const r = trigger.getBoundingClientRect();
    const tipRect = tip.getBoundingClientRect();
    const width = tipRect.width;
    let left = r.right - width;
    if (left < 8) left = 8;
    if (left + width > window.innerWidth - 8) left = window.innerWidth - 8 - width;
    const topAbove = r.top - 8 - tipRect.height;
    const top = topAbove < 8 ? r.bottom + 8 : topAbove;
    setPos({ top, left });
  }, []);

  const showNow = useCallback(() => {
    cancelHide();
    setMounted(true);
  }, []);

  const scheduleShow = useCallback(() => {
    cancelHide();
    cancelHover();
    hoverTimer.current = window.setTimeout(() => {
      hoverTimer.current = null;
      setMounted(true);
    }, 400);
  }, []);

  const hideNow = useCallback(() => {
    cancelHover();
    setVisible(false);
    cancelHide();
    hideTimer.current = window.setTimeout(() => {
      hideTimer.current = null;
      setMounted(false);
      setPos(null);
    }, 140);
  }, []);

  // 挂载后立即测量定位，下一帧触发进入动画
  useLayoutEffect(() => {
    if (!mounted) return;
    measure();
    const raf = requestAnimationFrame(() => setVisible(true));
    return () => cancelAnimationFrame(raf);
  }, [mounted, measure]);

  // 显示期间监听 resize / 滚动（含可滚动祖先）重新定位
  useEffect(() => {
    if (!mounted) return;
    const onScroll = () => measure();
    window.addEventListener('resize', onScroll);
    window.addEventListener('scroll', onScroll, true);
    return () => {
      window.removeEventListener('resize', onScroll);
      window.removeEventListener('scroll', onScroll, true);
    };
  }, [mounted, measure]);

  // 卸载时清理定时器
  useEffect(() => () => { cancelHover(); cancelHide(); }, []);

  return (
    <>
      <span
        className="tooltip"
        ref={triggerRef}
        tabIndex={0}
        aria-label={label}
        aria-describedby={mounted ? id : undefined}
        onMouseEnter={scheduleShow}
        onMouseLeave={hideNow}
        onFocus={showNow}
        onBlur={hideNow}
      >
        {children}
      </span>
      {mounted && createPortal(
        <span
          role="tooltip"
          id={id}
          ref={tooltipRef}
          className="tooltip-content"
          data-show={visible ? 'true' : 'false'}
          style={pos ? { top: `${pos.top}px`, left: `${pos.left}px` } : undefined}
        >
          {label}
        </span>,
        document.body
      )}
    </>
  );
}
