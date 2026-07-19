// SPDX-License-Identifier: AGPL-3.0-or-later
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { ReactNode, RefObject } from 'react';
import { OverlayPortal } from './OverlayPortal';
import { subscribeOverlayStack } from './overlayStack';

/// 水平对齐方式：`end` 让浮层右边缘对齐 trigger 右边缘，`start` 让左边缘对齐。
export type PopoverAlign = 'start' | 'end';

interface PopoverProps {
  open: boolean;
  onClose: () => void;
  /// 触发元素 ref，浮层相对此元素定位。
  triggerRef: RefObject<HTMLElement | null>;
  /// 无障碍标签。
  ariaLabel?: string;
  /// 水平对齐，默认 `'end'`。
  align?: PopoverAlign;
  /// 业务类名（例如 `'log-menu'`），承载视觉样式。
  className?: string;
  children: ReactNode;
}

/// 通用下拉浮层：通过 OverlayPortal 渲染到顶层 `#mira-overlay-root`，
/// 脱离业务组件所在的层叠上下文与滚动容器，避免被父级 `overflow: auto`
/// 裁剪。位置由 trigger 的 `getBoundingClientRect()` + 浮层自身尺寸动态
/// 计算，下方空间不足时自动向上翻转，水平方向夹取到视口安全区内。
///
/// 行为：
/// - 监听 `resize` / `scroll`（capture，覆盖所有祖先滚动容器）重新定位
/// - 点击 trigger / 浮层以外区域关闭
/// - Escape 关闭
/// - 任意 Modal 打开时自动关闭（避免穿透遮罩）
///
/// 参考 [src/Tooltip.tsx] 的 measure + 事件订阅模式，区别在于 Popover 是
/// 点击触发、需要点击外部关闭，且角色为 `menu`。
export function Popover({
  open,
  onClose,
  triggerRef,
  ariaLabel,
  align = 'end',
  className,
  children,
}: PopoverProps) {
  const popoverRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);

  // onClose 存入 ref，避免 effect 依赖数组随父组件重渲染而变化。
  const onCloseRef = useRef(onClose);
  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);

  const measure = useCallback(() => {
    const trigger = triggerRef.current;
    const popover = popoverRef.current;
    if (!trigger || !popover) return;

    const r = trigger.getBoundingClientRect();
    const pr = popover.getBoundingClientRect();
    const margin = 4;
    const safe = 8;

    const menuWidth = pr.width;
    const menuHeight = pr.height;
    const vw = window.innerWidth;
    const vh = window.innerHeight;

    // 水平：默认 end 对齐 trigger 右边缘，start 对齐左边缘
    let left = align === 'end' ? r.right - menuWidth : r.left;
    if (left < safe) left = safe;
    if (left + menuWidth > vw - safe) left = vw - safe - menuWidth;
    if (left < safe) left = safe; // 菜单比视口还宽时仍贴左

    // 垂直：默认下方，下方不够且上方更充裕时翻转到上方
    const belowTop = r.bottom + margin;
    const aboveTop = r.top - margin - menuHeight;
    let top: number;
    if (belowTop + menuHeight <= vh - safe) {
      top = belowTop;
    } else if (aboveTop >= safe) {
      top = aboveTop;
    } else {
      // 上下都不够：选择空间更大的一侧，并夹取到安全区内
      const belowSpace = vh - safe - belowTop;
      const aboveSpace = r.top - safe - margin;
      top = belowSpace >= aboveSpace ? belowTop : aboveTop;
      if (top < safe) top = safe;
      if (top + menuHeight > vh - safe) top = vh - safe - menuHeight;
    }

    setPos({ top, left });
  }, [triggerRef, align]);

  // 挂载后立即测量定位
  useLayoutEffect(() => {
    if (!open) return;
    measure();
  }, [open, measure]);

  // 显示期间监听 resize / 滚动（含可滚动祖先）重新定位
  useEffect(() => {
    if (!open) return;
    const onScroll = () => measure();
    window.addEventListener('resize', onScroll);
    window.addEventListener('scroll', onScroll, true);
    return () => {
      window.removeEventListener('resize', onScroll);
      window.removeEventListener('scroll', onScroll, true);
    };
  }, [open, measure]);

  // 点击外部 / Escape 关闭
  useEffect(() => {
    if (!open) return;

    const onMouseDown = (event: MouseEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      const trigger = triggerRef.current;
      const popover = popoverRef.current;
      if (trigger?.contains(target)) return;
      if (popover?.contains(target)) return;
      onCloseRef.current();
    };

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopImmediatePropagation();
        onCloseRef.current();
      }
    };

    document.addEventListener('mousedown', onMouseDown, true);
    document.addEventListener('keydown', onKeyDown, true);
    return () => {
      document.removeEventListener('mousedown', onMouseDown, true);
      document.removeEventListener('keydown', onKeyDown, true);
    };
  }, [open, triggerRef]);

  // 订阅浮层栈：任何 Modal 打开时立即关闭 Popover，避免穿透到遮罩之上。
  useEffect(() => {
    if (!open) return;
    return subscribeOverlayStack(() => {
      onCloseRef.current();
    });
  }, [open]);

  if (!open) return null;

  return (
    <OverlayPortal>
      <div
        ref={popoverRef}
        role="menu"
        aria-label={ariaLabel}
        className={className}
        style={pos ? { top: `${pos.top}px`, left: `${pos.left}px` } : undefined}
      >
        {children}
      </div>
    </OverlayPortal>
  );
}
