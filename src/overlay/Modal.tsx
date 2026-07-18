// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useLayoutEffect, useRef } from 'react';
import type { ReactNode } from 'react';
import { OverlayPortal } from './OverlayPortal';
import { openModalLayer } from './overlayStack';

export type ModalSize = 'small' | 'medium' | 'large';

interface ModalProps {
  open: boolean;
  /// 无障碍标题。可见标题由业务内容自己渲染，避免同一标题出现两次。
  title?: string;
  /// 无 title 时的 aria-label。
  ariaLabel?: string;
  size?: ModalSize;
  /// 内容容器的业务类名（例如 edit-modal / device-details / battery-usage-modal）。
  className?: string;
  /// 遮罩的业务类名（例如 edit-modal-backdrop / details-backdrop）。
  backdropClassName?: string;
  closeOnBackdrop?: boolean;
  closeOnEscape?: boolean;
  onClose: () => void;
  children: ReactNode;
}

function getFocusable(container: HTMLElement): HTMLElement[] {
  const selector = [
    'button:not([disabled])',
    'a[href]',
    'input:not([disabled])',
    'select:not([disabled])',
    'textarea:not([disabled])',
    '[tabindex]:not([tabindex="-1"])',
  ].join(',');

  return Array.from(container.querySelectorAll<HTMLElement>(selector))
    .filter((element) => !element.hasAttribute('hidden'));
}

/// 统一全屏模态窗口。
///
/// 所有内容通过 OverlayPortal 挂到顶层 #mira-overlay-root，脱离业务组件
/// 所在的层叠上下文，彻底避免 DPI / 回报率数字等合成层穿透遮罩。
///
/// 负责：遮罩与内容的毛玻璃样式、入场动画、Escape / 遮罩点击关闭、
/// 焦点进入与限制、背景 inert、焦点恢复、浮层栈注册。
export function Modal({
  open,
  title,
  ariaLabel,
  size = 'medium',
  className,
  backdropClassName,
  closeOnBackdrop = true,
  closeOnEscape = true,
  onClose,
  children,
}: ModalProps) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  // onClose 存入 ref：keydown 回调读取 ref.current() 而非闭包变量，
  // 这样 effect 依赖数组可以去掉 onClose，避免父组件每次重渲染时
  // （onClose 引用不稳定）effect 重跑、焦点陷阱被拆装一遍导致焦点丢失。
  const onCloseRef = useRef(onClose);
  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);

  // 打开后把焦点移入弹窗。useLayoutEffect 避免首帧焦点闪烁。
  useLayoutEffect(() => {
    if (!open) return;

    previousFocusRef.current = document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null;

    const frame = requestAnimationFrame(() => {
      const dialog = dialogRef.current;
      if (!dialog) return;
      const focusable = getFocusable(dialog);
      (focusable[0] ?? dialog).focus({ preventScroll: true });
    });

    return () => cancelAnimationFrame(frame);
  }, [open]);

  useEffect(() => {
    if (!open) return;

    // 背景应用 inert + aria-hidden，阻止键盘焦点逃逸到被遮罩的内容。
    // pointer-events: none 无法拦截键盘，必须用 inert。
    const appRoot = document.getElementById('root');
    appRoot?.setAttribute('inert', '');
    appRoot?.setAttribute('aria-hidden', 'true');

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && closeOnEscape) {
        event.preventDefault();
        // stopImmediatePropagation 阻止 document 上其它 keydown 监听
        // （例如其它浮层、全局快捷键）在同一 Escape 事件上重复触发，
        // 保证只有当前 Modal 消费此次 Escape。
        event.stopImmediatePropagation();
        onCloseRef.current();
        return;
      }

      if (event.key !== 'Tab') return;

      const dialog = dialogRef.current;
      if (!dialog) return;

      const focusable = getFocusable(dialog);
      if (focusable.length === 0) {
        event.preventDefault();
        dialog.focus();
        return;
      }

      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;

      if (event.shiftKey && active === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      }
    };

    document.addEventListener('keydown', handleKeyDown);

    // 注册到浮层栈：Tooltip / Popover 据此在 Modal 打开时关闭自身。
    const releaseModalLayer = openModalLayer();

    return () => {
      document.removeEventListener('keydown', handleKeyDown);
      releaseModalLayer();
      appRoot?.removeAttribute('inert');
      appRoot?.removeAttribute('aria-hidden');

      requestAnimationFrame(() => {
        previousFocusRef.current?.focus?.({ preventScroll: true });
      });
    };
  }, [closeOnEscape, open]);

  if (!open) return null;

  return (
    <OverlayPortal>
      <div
        className={['modal-backdrop', backdropClassName].filter(Boolean).join(' ')}
        data-modal-size={size}
        onMouseDown={(event) => {
          if (closeOnBackdrop && event.target === event.currentTarget) onClose();
        }}
      >
        <div
          ref={dialogRef}
          className={['modal-surface', className].filter(Boolean).join(' ')}
          role="dialog"
          aria-modal="true"
          aria-label={title ?? ariaLabel}
          tabIndex={-1}
        >
          {children}
        </div>
      </div>
    </OverlayPortal>
  );
}
