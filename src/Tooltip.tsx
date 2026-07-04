// SPDX-License-Identifier: AGPL-3.0-or-later
import { PropsWithChildren, useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

/**
 * Tooltip — 内容通过 React Portal 渲染到 document.body。
 *
 * 原因：tooltip 是临时浮层，需要脱离普通内容 surface 的堆叠上下文，
 * 让自己的特殊玻璃 token 独立工作。Portal 到 body 后，tooltip 不会被卡片、
 * 滚动容器或未来可能新增的 backdrop root 截断取样范围。
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
