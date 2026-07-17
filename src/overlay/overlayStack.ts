// SPDX-License-Identifier: AGPL-3.0-or-later

// 全局浮层栈：追踪当前打开的 Modal 数量，并通知订阅者（Tooltip / Popover）
// 在 Modal 打开期间隐藏自身，避免背景浮层穿透到遮罩之上。

type Listener = () => void;

let modalCount = 0;
const listeners = new Set<Listener>();

function emit(): void {
  for (const listener of listeners) listener();
}

/// 注册一个已打开的 Modal 层。返回清理函数，在 Modal 关闭时调用。
/// 同时存在多个业务 Modal 视为异常，开发期输出警告。
export function openModalLayer(): () => void {
  modalCount += 1;
  if (modalCount > 1) {
    console.warn('[Mira Overlay] Multiple modal layers are open simultaneously.');
  }
  emit();

  return () => {
    modalCount = Math.max(0, modalCount - 1);
    emit();
  };
}

/// 当前是否有 Modal 处于打开状态。
export function hasOpenModal(): boolean {
  return modalCount > 0;
}

/// 订阅浮层栈变化。返回取消订阅函数。
export function subscribeOverlayStack(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}