// SPDX-License-Identifier: AGPL-3.0-or-later
import { useCallback, useEffect, useReducer } from 'react';
import {
  announceAttentionRequest,
  finishActiveAttentionRequest,
  getAttentionBusState,
  onAttentionBusStateChange,
} from './attentionCore';
import type { AttentionBeamRequest } from './attentionTypes';

export type AttentionBehavior = 'all' | 'lighting' | 'high';

function isLightingEventKey(eventKey: string): boolean {
  return eventKey.startsWith('lighting:');
}

/** 光束自动结束的尾音（时长耗尽后再宽限的清理时间）。 */
const BEAM_TAIL_MS = 140;

/** Reduced Motion 下视觉改为约 220ms 静态淡入淡出后的等效总时长。 */
const REDUCED_MOTION_FALLBACK_MS = 450;

function prefersReducedMotion(): boolean {
  return typeof window !== 'undefined'
    && typeof window.matchMedia === 'function'
    && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

function beamTotalMs(request: AttentionBeamRequest): number {
  return (request.delayMs ?? 0) + request.durationMs * request.cycles + BEAM_TAIL_MS;
}

/**
 * 订阅某 surface 作用域（scope）的 Attention 光束。
 *
 * - beam：当前应渲染在本作用域上的光束（其余作用域对应的活跃光束不会出现在这里）；
 * - announce：发起一次请求，返回是否真正入队（会话去重 / 行为降级会拒绝）。
 *
 * 行为约束：
 * - 同一事件一次会话只播放一次（由 attentionCore 保证）；
 * - 窗口不可见时拒绝并绝不补播；
 * - 同一个 surface 同一时刻最多渲染一个光束（全局单活跃仲裁）；
 * - behavior 用于组件层面的降级：'lighting' 只允许灯光类，「high」只允许
 *   更新 / 设备类。
 */
export function useAttentionFeedback(
  scope: string,
  options: { behavior?: AttentionBehavior } = {},
): {
  beam: AttentionBeamRequest | null;
  announce: (request: AttentionBeamRequest) => boolean;
} {
  const behavior = options.behavior ?? 'all';
  const [, refresh] = useReducer((value: number) => value + 1, 0);

  useEffect(() => onAttentionBusStateChange(refresh), []);

  const announce = useCallback((request: AttentionBeamRequest): boolean => {
    if (behavior === 'lighting' && !isLightingEventKey(request.eventKey)) return false;
    if (behavior === 'high' && isLightingEventKey(request.eventKey)) return false;
    return announceAttentionRequest(request);
  }, [behavior]);

  const active = getAttentionBusState().active;
  const beam = active && active.scope === scope ? active : null;

  // 完成定时与异常清理（surface 卸载 / 作用域切换）：
  // 只有本组件确实在渲染该光束时才负责结束它，且结束为幂等操作。
  useEffect(() => {
    if (!beam) return;
    const total = prefersReducedMotion()
      ? Math.min(REDUCED_MOTION_FALLBACK_MS, beamTotalMs(beam))
      : beamTotalMs(beam);
    const timer = window.setTimeout(() => finishActiveAttentionRequest(), total);
    return () => {
      window.clearTimeout(timer);
      const currentActive = getAttentionBusState().active;
      if (currentActive && currentActive.eventKey === beam.eventKey) {
        finishActiveAttentionRequest();
      }
    };
  }, [beam]);

  return { beam, announce };
}