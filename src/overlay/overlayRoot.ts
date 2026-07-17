// SPDX-License-Identifier: AGPL-3.0-or-later

export const OVERLAY_ROOT_ID = 'mira-overlay-root';

/// 确保顶层 Overlay Root 存在。所有全局浮层（Modal / Tooltip / 未来 Popover）
/// 共用同一个根节点，脱离业务组件所在的层叠上下文。
///
/// 调用方需自行保证运行环境存在 document（浏览器 / jsdom 等）。
export function ensureOverlayRoot(): HTMLElement {
  const existing = document.getElementById(OVERLAY_ROOT_ID);
  if (existing) return existing;

  const root = document.createElement('div');
  root.id = OVERLAY_ROOT_ID;
  root.dataset.miraOverlayRoot = 'true';
  document.body.appendChild(root);
  return root;
}
