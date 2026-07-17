// SPDX-License-Identifier: AGPL-3.0-or-later
import { useSyncExternalStore } from 'react';
import { hasOpenModal, subscribeOverlayStack } from './overlayStack';

/// 订阅浮层栈变化，返回当前是否有 Modal 处于打开状态。
/// 用于在 Modal 打开期间禁用背景交互（例如通知点击行为）。
export function useHasOpenModal(): boolean {
  return useSyncExternalStore(
    subscribeOverlayStack,
    hasOpenModal,
    // SSR 快照：服务端永远视为没有 Modal。
    () => false,
  );
}