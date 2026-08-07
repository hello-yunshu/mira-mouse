// SPDX-License-Identifier: AGPL-3.0-or-later
// Activity 与 Attention Beam 的最小协调层。
//
// 核心语义：
//   Orb  —— 正在做什么（过程）；
//   Beam —— 什么重要变化刚刚发生（完成事件）。
//
// 同一 scope 内两条规则必须同时成立：
//   1. 一个任务即将产生完成事件（Beam）时，先立即结束同一 scope 的 Orb
//      （跳过最短可见尾段），等该 scope 的 Orb 真实注销后再提交 Beam；
//   2. 渲染层兜底：Orb 自身在渲染时判断同一 scope 是否已有 Beam 在播，
//      有则不渲染 Orb。
//
// 本模块不复制 Attention Bus，也不承担第二套状态框架：
// - 每个 Orb 以组件级 token 注册，同一 scope 的多个组件互不覆盖；
// - 退出提示是每个 scope 的累计计数，组件以“自挂载以来的增量”感知，
//   历史累计只作为新组件/新 scope 的基线，不会被误当作新事件；
// - 等待退出时订阅真实可见状态，而不是只等一帧，避免窗口隐藏时挂起。
import { useEffect, useReducer, useState, useSyncExternalStore } from 'react';
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
 * 没有对应 Beam 的活动（导出、复制、扫描等）返回 null，无需仲裁。
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

/** 单个 Orb 可见期的注册令牌：同一 scope 多个可见组件互不覆盖。 */
export type ActivityRegistrationToken = symbol;

/** scope → 仍可见的注册令牌集合；最后一个注销后该 scope 才不可见。 */
const visibleTokens = new Map<ActivityScope, Set<ActivityRegistrationToken>>();
/** 每个 scope 的任务代数：新一轮任务启动时递增，用于让过期完成事件失效。 */
const taskEpochs = new Map<ActivityScope, number>();
/** 每个 scope 的“立即退出”提示计数，供 Orb 组件跳过最短可见尾段。 */
const hideHints = new Map<ActivityScope, number>();
const hideListeners = new Set<() => void>();
const visibilityListeners = new Set<() => void>();

/** 等待真实验证注销的超时兜底，避免完成后 Beam 被无限期推迟。 */
export const ACTIVITY_EXIT_TIMEOUT_MS = 1000;

function emitHideHints(): void {
  for (const listener of hideListeners) listener();
}

function emitVisibilityChange(): void {
  for (const listener of visibilityListeners) listener();
}

/** Orb 开始显示时注册。同一 token 重复注册幂等。 */
export function registerVisibleActivity(
  scope: ActivityScope,
  token: ActivityRegistrationToken,
): void {
  let tokens = visibleTokens.get(scope);
  if (!tokens) {
    tokens = new Set<ActivityRegistrationToken>();
    visibleTokens.set(scope, tokens);
  }
  tokens.add(token);
  emitVisibilityChange();
}

/** Orb 不再显示时注销。不存在的 token 或 scope 安全。 */
export function unregisterVisibleActivity(
  scope: ActivityScope,
  token: ActivityRegistrationToken,
): void {
  const tokens = visibleTokens.get(scope);
  if (!tokens) return;
  tokens.delete(token);
  if (tokens.size === 0) visibleTokens.delete(scope);
  emitVisibilityChange();
}

/** 该 scope 当前是否已有一个可见 Orb。 */
export function isActivityVisible(scope: ActivityScope): boolean {
  return (visibleTokens.get(scope)?.size ?? 0) > 0;
}

/**
 * 业务任务开始时递增该 scope 的任务代数（scope → task epoch）。
 * 任务在第 0ms 开始，而 Orb 要等 300ms 才可见注册，因此在等待退出期间
 * 仅凭“scope 是否可见”无法判断是否已经开始了更新一代任务。
 * 代数变化即代表新一代替任已开始，旧任务尚未提交的 completion 应被丢弃。
 */
export function beginActivityTask(scope: ActivityScope): void {
  taskEpochs.set(scope, (taskEpochs.get(scope) ?? 0) + 1);
}

/** 读取某 scope 当前的任务代数（未开始过为 0）。 */
export function currentActivityTaskEpoch(scope: ActivityScope): number {
  return taskEpochs.get(scope) ?? 0;
}

/** 该 scope 当前是否已有 Beam 在播放（渲染层兜底仲裁）。 */
export function hasActiveBeamForScope(scope: ActivityScope): boolean {
  return getAttentionBusState().active?.scope === scope;
}

/** 订阅某个 scope 的 Beam 活跃状态；活跃时返回 true（渲染层兜底仲裁）。 */
export function useActiveBeamForScope(scope: ActivityScope | null): boolean {
  return useSyncExternalStore(
    onAttentionBusStateChange,
    () => scope !== null && hasActiveBeamForScope(scope),
    () => scope !== null && hasActiveBeamForScope(scope),
  );
}

/** 让该 scope 的 Orb 立即退出（跳过最短可见时间）。 */
export function bumpActivityExitHint(scope: ActivityScope): void {
  hideHints.set(scope, (hideHints.get(scope) ?? 0) + 1);
  emitHideHints();
}

/**
 * 订阅某个 scope 的退出提示。返回“自本实例挂载以来”的增量：
 * 新挂载 / scope 变化后，把已累计的退出提示当作基线，不作为新事件，
 * 避免重挂载后第一轮任务被上一轮历史提示整轮抑制（P0-1）。
 */
export function useActivityExitHint(scope: ActivityScope | null): number {
  const current = scope ? (hideHints.get(scope) ?? 0) : 0;
  const [, refresh] = useReducer((value: number) => value + 1, 0);
  const [baseline, setBaseline] = useState<{
    scope: ActivityScope | null;
    hint: number;
  }>(() => ({ scope, hint: current }));

  useEffect(() => {
    const listener = () => refresh();
    hideListeners.add(listener);
    return () => {
      hideListeners.delete(listener);
    };
  }, []);

  // 基线随 scope 变化重置：setState-during-render 是“由 props 推导 state”
  // 的 React 模式（与 InlineRangeSlider 一致），scope 变化的那次渲染以 0
  // 过渡，新基线渲染前不会触发任何假的退出提示。
  if (baseline.scope !== scope) {
    setBaseline({ scope, hint: current });
    return 0;
  }
  return current - baseline.hint;
}

export type ActivityExitOutcome = 'exited' | 'already-hidden' | 'timed-out';

/**
 * 等待某 scope 的 Orb 真实注销：
 * - 当前不可见时立即返回 'already-hidden'（保持既有同步语义）；
 * - 有可见 Orb 时先发出“立即退出”提示，再等待真实可见状态变化；
 * - 以 setTimeout 超时兜底，不依赖 rAF（窗口隐藏也不会挂起），
 *   并保证监听器与计时器在 settled 后都被清理。
 */
export async function waitForActivityExit(
  scope: ActivityScope,
): Promise<ActivityExitOutcome> {
  if (!isActivityVisible(scope)) return 'already-hidden';
  bumpActivityExitHint(scope);
  return new Promise<ActivityExitOutcome>((resolve) => {
    let settled = false;
    let timer = 0;
    const onVisibilityChange = () => {
      if (isActivityVisible(scope)) return;
      settled = true;
      visibilityListeners.delete(onVisibilityChange);
      window.clearTimeout(timer);
      resolve('exited');
    };
    const onTimeout = () => {
      if (settled) return;
      settled = true;
      visibilityListeners.delete(onVisibilityChange);
      resolve('timed-out');
    };
    visibilityListeners.add(onVisibilityChange);
    timer = window.setTimeout(onTimeout, ACTIVITY_EXIT_TIMEOUT_MS);
  });
}

/**
 * 提交完成事件（Beam）的入口：同一 scope 有可见 Orb 时，先退出 Orb 再提交；
 * 没有 Orb 时同步提交，保证现有测试与调用方的同步语义不变。退出等待超时
 * 说明该 scope 的 Orb 仍在显示（任务可能尚未结束），不提交这轮可能过期的
 * Beam，避免制造 Orb 与 Beam 的长时间重叠。Orb 退出后若同 scope 已有新
 * 任务注册，同样不提交这轮 Beam（过期完成反馈）。
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
  // 记录等待开始时的代数。等待期间同 scope 若开始了新一代任务（代数前进），
  // 说明旧任务的完成反馈已过期，不得提交这轮可能覆盖新任务过程语义的 Beam。
  const epochAtWaitStart = currentActivityTaskEpoch(scope);
  void (async () => {
    const outcome = await waitForActivityExit(scope);
    // 提交前再同步确认：wait 解析到 "exited" 之后的微任务间隙，同 scope
    // 可能已有新任务注册新 token（旧任务完成、新任务立刻开始）。此时不再
    // 提交旧任务的完成 Beam，避免新任务的 Orb 被过期完成反馈压制。
    if (
      outcome === 'exited'
      && !isActivityVisible(scope)
      && currentActivityTaskEpoch(scope) === epochAtWaitStart
    ) {
      announce(request);
    }
  })();
}

/** 仅测试使用：清空注册、提示、代数与全部监听器（生产不调用）。 */
export function resetActivityCoordinatorForTests(): void {
  visibleTokens.clear();
  taskEpochs.clear();
  hideHints.clear();
  hideListeners.clear();
  visibilityListeners.clear();
}