// SPDX-License-Identifier: AGPL-3.0-or-later
// Attention 请求的统一时长计算：全局 Controller 与视觉 Layer 共用同一套
// 总时长，保证「视觉节点结束」与「全局 active 结束、pending 推进」完全一致。

import type { AttentionBeamRequest } from './attentionTypes';

/** 光束时长耗尽后再宽限的清理尾音。 */
export const ATTENTION_BEAM_TAIL_MS = 180;

/** Reduced Motion 下视觉切换为约 220ms 静态淡入淡出后的等效总时长。 */
export const ATTENTION_REDUCED_MOTION_TOTAL_MS = 450;

export function prefersReducedAttentionMotion(): boolean {
  return typeof window !== 'undefined'
    && typeof window.matchMedia === 'function'
    && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

/**
 * 一次请求从开始到结束的总毫秒数。
 * - durationMs 表示视觉部分总时长（不乘以 cycles）；
 * - 不允许负时长；
 * - Reduced Motion 使用同一计算（截断到固定总时长）。
 */
export function attentionRequestTotalMs(
  request: Pick<AttentionBeamRequest, 'delayMs' | 'durationMs'>,
  reducedMotion = prefersReducedAttentionMotion(),
): number {
  const delayMs = Math.max(0, request.delayMs ?? 0);
  const durationMs = Math.max(0, request.durationMs);

  const normalTotal = delayMs + durationMs + ATTENTION_BEAM_TAIL_MS;

  return reducedMotion
    ? Math.min(ATTENTION_REDUCED_MOTION_TOTAL_MS, normalTotal)
    : normalTotal;
}
