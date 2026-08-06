// SPDX-License-Identifier: AGPL-3.0-or-later
// 灯光 Attention 的“用户 Mutation 关联”（P1-2）。
//
// 灯光 Beam 只允许在用户主动发起、成功写入、且快照确认目标值后才播放。
// 本模块提供一个轻量的 Pending 登记表（模块级单例，非 React 状态）：
// - 灯光字段操作入口在调用 runMutation 前 registerLightingAttention 登记；
// - runMutation 成功返回快照后由 confirmPendingLightingAttention 校验并播放；
// - 失败时调用方 clearPendingLightingAttention，不播放。
//
// 事件键带有 mutationId：同一会话中「再次开启」「切回旧颜色/旧效果」仍能反馈，
// 不再按值永久去重。更新类事件仍保留版本级会话去重。

import { announceAttentionRequest } from './attentionCore';
import {
  ATTENTION_PRIORITY,
  attentionColorForZone,
  type AttentionBeamRequest,
} from './attentionTypes';

export type LightingAttentionKind = 'power-on' | 'color-applied' | 'effect-applied';

export interface PendingLightingAttention {
  id: number;
  zoneId: string;
  kind: LightingAttentionKind;
  expectedValue: unknown;
}

/** 校验时需要的 Zone 灯光状态（由宿主从插件声明 + 设备快照提取）。 */
export interface ZoneLightingState {
  enabled: boolean;
  color: string | undefined;
  effectValue: unknown;
}

let nextLightingAttentionId = 0;
let pendingLightingAttention: PendingLightingAttention | null = null;

/** 灯光字段操作入口在调用 runMutation 前登记。返回 mutationId（事件键的一部分）。 */
export function registerLightingAttention(
  zoneId: string,
  kind: LightingAttentionKind,
  expectedValue: unknown,
): number {
  nextLightingAttentionId += 1;
  pendingLightingAttention = { id: nextLightingAttentionId, zoneId, kind, expectedValue };
  return nextLightingAttentionId;
}

/** 只读查看当前登记（不消费），供宿主在成功回调用 zoneId 提取前后状态。 */
export function peekPendingLightingAttention(id: number): PendingLightingAttention | null {
  return pendingLightingAttention?.id === id ? pendingLightingAttention : null;
}

/** 消费登记。返回 null 表示没有匹配的登记（或已被清除）。 */
export function takePendingLightingAttention(id: number): PendingLightingAttention | null {
  if (pendingLightingAttention?.id !== id) return null;
  const taken = pendingLightingAttention;
  pendingLightingAttention = null;
  return taken;
}

/** 失败时清除登记，保证不播放。 */
export function clearPendingLightingAttention(id: number): void {
  if (pendingLightingAttention?.id === id) pendingLightingAttention = null;
}

/** 仅测试使用：清空 pending 登记（生产不调用）。 */
export function resetPendingLightingAttentionForTests(): void {
  nextLightingAttentionId = 0;
  pendingLightingAttention = null;
}

/** Mutation 成功后的目标值确认：Zone 状态必须与期望一致才算“确认写入成功”。 */
export function verifyLightingAttention(
  pending: PendingLightingAttention,
  zoneState: ZoneLightingState,
): boolean {
  switch (pending.kind) {
    case 'power-on':
      return zoneState.enabled === true;
    case 'color-applied':
      return typeof pending.expectedValue === 'string' && zoneState.color === pending.expectedValue;
    case 'effect-applied':
      return zoneState.effectValue === pending.expectedValue;
  }
}

function lightingStateEqual(a: ZoneLightingState, b: ZoneLightingState): boolean {
  return a.enabled === b.enabled && a.color === b.color && a.effectValue === b.effectValue;
}

/**
 * runMutation 成功后的统一确认入口：
 * - 登记不存在或失败路径已清除 → 不播放；
 * - 目标值未确认（verify 失败）→ 不播放；
 * - 前后状态完全一致（设备没有实际变化）→ 不播放；
 * - 通过后按 mutationId 事件键播放，同一操作可重复反馈。
 */
export function confirmPendingLightingAttention(
  attentionId: number | undefined,
  zoneStates: { before: ZoneLightingState | undefined; after: ZoneLightingState | undefined },
): boolean {
  if (attentionId === undefined) return false;
  const pending = takePendingLightingAttention(attentionId);
  if (!pending) return false;
  if (!zoneStates.after || !verifyLightingAttention(pending, zoneStates.after)) return false;
  if (zoneStates.before && lightingStateEqual(zoneStates.before, zoneStates.after)) return false;
  announceAttentionRequest(lightingAttentionRequest(pending.kind, pending.zoneId, String(pending.expectedValue), pending.id, zoneStates.after.color));
  return true;
}

// ─── 事件键（mutationId 后缀：同一会话的重复操作仍可反馈） ────────────────

/** 事件键中的语义段（power / color / effect），与 LightingAttentionKind 相互独立。 */
export type LightingKeyKind = 'power' | 'color' | 'effect';

export function attentionLightingMutationKey(
  zoneId: string,
  kind: LightingKeyKind,
  value: string,
  mutationId: number,
): string {
  return `lighting:${zoneId}:${kind}:${value}:${mutationId}`;
}

/** 构建灯光 Beam 请求。mutationId 来自注册时返回值；zoneColor 用于取光束显示色。 */
export function lightingAttentionRequest(
  kind: LightingAttentionKind,
  zoneId: string,
  value: string,
  mutationId: number,
  zoneColor?: string,
): AttentionBeamRequest {
  if (kind === 'power-on') {
    return {
      eventKey: attentionLightingMutationKey(zoneId, 'power', 'on', mutationId),
      scope: `lighting:${zoneId}`,
      variant: 'line',
      color: attentionColorForZone(zoneColor?.trim() ? zoneColor : '#ffb3b3'),
      durationMs: 1750,
      strength: 0.2,
      cycles: 1,
      priority: ATTENTION_PRIORITY['lighting-power-on'],
    };
  }
  if (kind === 'color-applied') {
    return {
      eventKey: attentionLightingMutationKey(zoneId, 'color', value, mutationId),
      scope: `lighting:${zoneId}`,
      variant: 'line',
      color: attentionColorForZone(value),
      durationMs: 1700,
      strength: 0.22,
      cycles: 1,
      priority: ATTENTION_PRIORITY['lighting-color-applied'],
    };
  }
  return {
    eventKey: attentionLightingMutationKey(zoneId, 'effect', value, mutationId),
    scope: `lighting:${zoneId}`,
    variant: 'line',
    color: attentionColorForZone(zoneColor?.trim() ? zoneColor : '#ffb3b3'),
    durationMs: 1450,
    strength: 0.15,
    cycles: 1,
    priority: ATTENTION_PRIORITY['lighting-effect-applied'],
  };
}
