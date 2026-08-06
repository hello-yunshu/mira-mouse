// SPDX-License-Identifier: AGPL-3.0-or-later
// Activity 与 Attention Beam 的最小协调层。
//
// 核心语义：
//   Orb  —— 正在做什么（过程）；
//   Beam —— 什么重要变化刚刚发生（完成事件）。
//
// 同一 scope 内两条规则必须同时成立：
//   1. 一个任务即将产生完成事件（Beam）时，先立即结束同一 scope 的 Orb
//      （跳过最短可见尾段），等 Orb 退出后再提交 Beam；
//   2. 渲染层兜底：Orb 显示期间若同一 scope 已有 Beam 在播放，则不渲染 Orb。
//
// 本模块不复制 Attention Bus，也不引入第二套状态框架：
// 只维护“哪些 scope 当前有可见 Orb”的集合 + 每个 scope 的强制退出提示。
import { useCallback, useSyncExternalStore } from 'react';
import {
  getAttentionBusState,
  onAttentionBusStateChange,
} from '../attention/attentionCore';
import type { AttentionBeamRequest } from '../attention/attentionTypes';
import type { MiraActivityKind } from './activityCatalog';

/** 复用现有 Attention scope 作为活动/完成事件的共同仲裁作用域。 */
export type ActivityScope =
  | 'device:app'
  | 'about-update'
  | 'settings-plugin'
  | 'settings-local-ai';

/**
 * 活动种类 → 它“即将产生的完成事件”所在的 Attention scope。
 * 没有对应 Beam 的活动（导入导出、复制、扫描等）返回 null，无需仲裁。
 */
export function attentionScopeForActivity(
  activity: MiraActivityKind,
): ActivityScope | null {
  switch (activity) {
    case 'awaiting-mouse':
    case 'device-initializing':
      return 'device:app';
    case 'checking-app-update':
      return 'about-update';
    case 'checking-plugin-updates':
      return 'settings-plugin';
    case 'checking-local-ai-updates':
    case 'restoring-local-ai':
      return 'settings-local-ai';
    default:
      return null;
  }
}

/** 当前有可见 Orb（含最短可见尾段）的 scope。 */
const visibleScopes = new Set<ActivityScope>();
/** 每个 scope 的“立即退出”提示计数，供 Orb 组件订阅后跳过最短可见尾段。 */
const hideHints = new Map<ActivityScope, number>();
const hideListeners = new Set<() => void>();

/** Orb 开始显示时注册。幂等。 */
export function beginActivity(scope: ActivityScope): void {
  visibleScopes.add(scope);
}

/** Orb 不再显示时注销。幂等。 */
export function endActivity(scope: ActivityScope): void {
  visibleScopes.delete(scope);
}

/** 该 scope 当前是否显示着 Orb。 */
export function isActivityVisible(scope: ActivityScope): boolean {
  return visibleScopes.has(scope);
}

/** 该 scope 当前是否已有 Beam 在播放（渲染层兜底仲裁）。 */
export function hasActiveBeamForScope(scope: ActivityScope): boolean {
  return getAttentionBusState().active?.scope === scope;
}

/** 订阅某个 scope 的 Beam 活跃状态；活跃时返回 true（渲染层兜底仲裁）。 */
export function useActiveBeamForScope(scope: ActivityScope | null): boolean {
  const subscribe = useCallback((onStoreChange: () => void) => {
    return onAttentionBusStateChange(onStoreChange);
  }, []);
  return useSyncExternalStore(
    subscribe,
    () => scope !== null && hasActiveBeamForScope(scope),
    () => scope !== null && hasActiveBeamForScope(scope),
  );
}

function emitHideHints(): void {
  for (const listener of hideListeners) listener();
}

/** 让该 scope 的 Orb 立即退出（跳过最短可见时间）。 */
export function bumpActivityExitHint(scope: ActivityScope): void {
  hideHints.set(scope, (hideHints.get(scope) ?? 0) + 1);
  emitHideHints();
}

/** 订阅某个 scope 的退出提示计数；计数增长时组件应立即隐藏 Orb。 */
export function useActivityExitHint(scope: ActivityScope | null): number {
  const subscribe = useCallback((onStoreChange: () => void) => {
    hideListeners.add(onStoreChange);
    return () => hideListeners.delete(onStoreChange);
  }, []);
  return useSyncExternalStore(
    subscribe,
    () => (scope ? (hideHints.get(scope) ?? 0) : 0),
    () => (scope ? (hideHints.get(scope) ?? 0) : 0),
  );
}

/**
 * 等待某 scope 的 Orb 退出：
 * - 没有可见 Orb 时立即返回（保持既有同步行为）；
 * - 有可见 Orb 时先发出强制退出提示，再等待至少一个 animation frame，
 *   确保 Orb 已从 DOM 移除。
 */
export async function waitForActivityExit(scope: ActivityScope): Promise<void> {
  if (!isActivityVisible(scope)) return;
  bumpActivityExitHint(scope);
  await new Promise<void>((resolve) => {
    requestAnimationFrame(() => resolve());
  });
}

/**
 * 提交完成事件（Beam）的入口：同一 scope 有可见 Orb 时，先退出 Orb 再提交；
 * 没有 Orb 时同步提交，保证现有测试与调用方的同步语义不变。
 */
export function announceAfterOrbExit(
  scope: ActivityScope,
  announce: (request: AttentionBeamRequest) => boolean,
  request: AttentionBeamRequest,
): void {
  if (!isActivityVisible(scope)) {
    announce(request);
    return;
  }
  void (async () => {
    await waitForActivityExit(scope);
    announce(request);
  })();
}

/** 仅测试使用：清空协调层状态（生产不调用）。 */
export function resetActivityCoordinatorForTests(): void {
  visibleScopes.clear();
  hideHints.clear();
  hideListeners.clear();
}
