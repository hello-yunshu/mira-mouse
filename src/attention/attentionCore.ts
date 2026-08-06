// SPDX-License-Identifier: AGPL-3.0-or-later
// Attention Beam 的全局核心状态：会话级去重、优先级排队与“全局同一时刻
// 最多一个明显光束”的仲裁。
//
// 这里不持有 React 依赖；组件通过 useAttentionFeedback 订阅本总线。

import { MAX_ATTENTION_QUEUE, type AttentionBeamRequest } from './attentionTypes';

export interface AttentionBusState {
  /** 当前正在播放的光束。同一时刻全局最多一个。 */
  active: AttentionBeamRequest | null;
  /** 等待播放的排队请求，按优先级降序。 */
  pending: AttentionBeamRequest[];
}

export type AttentionBusAction =
  | { type: 'announce'; request: AttentionBeamRequest }
  | { type: 'finish' };

export function createInitialAttentionBusState(): AttentionBusState {
  return { active: null, pending: [] };
}

function byPriorityDesc(a: AttentionBeamRequest, b: AttentionBeamRequest): number {
  return b.priority - a.priority;
}

/** 纯归约函数（便于测试）：进入新请求 / 结束当前请求。 */
export function attentionBusReduce(state: AttentionBusState, action: AttentionBusAction): AttentionBusState {
  if (action.type === 'finish') {
    const [next, ...rest] = state.pending;
    return { active: next ?? null, pending: rest };
  }
  const request = action.request;
  if (state.active) {
    const pending = [...state.pending, request]
      .sort(byPriorityDesc)
      .slice(0, MAX_ATTENTION_QUEUE);
    return { active: state.active, pending };
  }
  return { active: request, pending: state.pending };
}

// ─── 运行时单例 ───────────────────────────────────────────────────────────

let busState: AttentionBusState = createInitialAttentionBusState();
const listeners = new Set<() => void>();
const sessionKeys = new Map<string, number>();

function emit(): void {
  for (const listener of listeners) listener();
}

export function getAttentionBusState(): AttentionBusState {
  return busState;
}

export function onAttentionBusStateChange(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** 是否允许本事件在本次会话再次播放（会话级去重）。 */
export function hasAttentionEventPlayedOnce(eventKey: string): boolean {
  return sessionKeys.has(eventKey);
}

/**
 * 提交一条光束请求。返回是否真正排队/播放：
 * - 窗口不可见（document.hidden）时拒绝，且返回窗口后不补播（§4.3）；
 * - 同一 eventKey 一次会话只入队一次（§九）；
 * - 当前已有活跃光束时按优先级进入排队。
 */
export function announceAttentionRequest(request: AttentionBeamRequest): boolean {
  if (typeof document !== 'undefined' && document.hidden) return false;
  if (sessionKeys.has(request.eventKey)) return false;
  sessionKeys.set(request.eventKey, 1);
  busState = attentionBusReduce(busState, { type: 'announce', request });
  emit();
  return true;
}

/** 结束当前活跃光束并推进下一个（幂等：无活跃时无操作）。 */
export function finishActiveAttentionRequest(): void {
  if (!busState.active) return;
  busState = attentionBusReduce(busState, { type: 'finish' });
  emit();
}

/** 仅测试使用：清空会话去重表与当前总线状态（生产不调用）。 */
export function resetAttentionBusForTests(): void {
  busState = createInitialAttentionBusState();
  sessionKeys.clear();
  listeners.clear();
}