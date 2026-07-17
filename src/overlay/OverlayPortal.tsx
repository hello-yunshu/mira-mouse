// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { ensureOverlayRoot } from './overlayRoot';

interface OverlayPortalProps {
  children: ReactNode;
}

/// 把子节点 Portal 到统一 Overlay Root。
///
/// Overlay Root 是一个 position: fixed 的全屏容器，本身不创建合成层、不裁切、
/// 不带 backdrop-filter，仅作为浮层的公共层叠上下文宿主。
///
/// 通过 useState 惰性初始化在首次渲染期间取得容器，避免 effect 内 setState
/// 触发的级联渲染；SSR 环境下 document 不存在时返回 null，子节点不渲染。
export function OverlayPortal({ children }: OverlayPortalProps) {
  const [container] = useState<HTMLElement | null>(() =>
    typeof document !== 'undefined' ? ensureOverlayRoot() : null,
  );

  useEffect(() => {
    return () => {
      // 最后一个浮层卸载后移除根节点，保持 DOM 干净。
      if (container && container.childElementCount === 0) container.remove();
    };
  }, [container]);

  if (!container) return null;
  return createPortal(children, container);
}

