// SPDX-License-Identifier: AGPL-3.0-or-later
// LightingAttention —— 灯光 Zone 的一次性反馈观察器（不渲染任何 DOM）。
//
// 只监听「旧状态 → 新状态」的真实迁移，不依赖弹窗关闭 / 点击 / 本地临时状态：
// - lighting-power-on：仅在 previousEnabled === false && enabled === true 时触发；
// - lighting-color-applied：仅在 Zone 主色「确认写入后的新值」出现时触发
//   （写入失败设备状态不变，自然不会出现新值）；同一颜色不重复触发；
// - lighting-effect-applied：effect 字段值发生真实变化时触发。
//
// 事件键走 §九 的稳定键形：lighting:${zoneId}:kind:value；
// 页面进入、标签切换、轮询读取到相同值都不会触发。

import { useEffect, useRef } from 'react';
import { announceAttentionRequest } from './attentionCore';
import {
  ATTENTION_PRIORITY,
  attentionColorForZone,
  attentionLightingKey,
  type AttentionBeamRequest,
} from './attentionTypes';

type LightingKind = 'power-on' | 'color-applied' | 'effect-applied';

interface LightingAttentionProps {
  zoneId: string;
  /** 当前 Zone 是否开启（zoneLightingEnabled 的结果）。 */
  enabled?: boolean;
  /** 当前 Zone 的主色值（zonePrimaryColor 的结果）。 */
  color?: string;
  /** 当前 Zone 的 effect 字段值（若声明了 lightingRole: 'effect'）。 */
  effectValue?: unknown;
}

interface WatchedSignal {
  zoneId: string;
  signal: string;
}

function requestFor(kind: LightingKind, zoneId: string, value: string, zoneColor?: string): AttentionBeamRequest {
  if (kind === 'power-on') {
    return {
      eventKey: attentionLightingKey(zoneId, 'power', 'on'),
      scope: `lighting:${zoneId}`,
      variant: 'line',
      color: attentionColorForZone(zoneColor?.trim() ? zoneColor! : '#ffb3b3'),
      durationMs: 1750,
      strength: 0.2,
      cycles: 1,
      priority: ATTENTION_PRIORITY['lighting-power-on'],
    };
  }
  if (kind === 'color-applied') {
    return {
      eventKey: attentionLightingKey(zoneId, 'color', value),
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
    eventKey: attentionLightingKey(zoneId, 'effect', value),
    scope: `lighting:${zoneId}`,
    variant: 'line',
    color: attentionColorForZone(zoneColor?.trim() ? zoneColor : '#ffb3b3'),
    durationMs: 1450,
    strength: 0.15,
    cycles: 1,
    priority: ATTENTION_PRIORITY['lighting-effect-applied'],
  };
}

export function LightingAttention({ zoneId, enabled, color, effectValue }: LightingAttentionProps) {
  const watchedRef = useRef<WatchedSignal | undefined>(undefined);

  useEffect(() => {
    const effectText = effectValue == null ? '' : String(effectValue);
    const signal = `${enabled ? 'on' : 'off'}|${color ?? ''}|${effectText}`;
    const previous = watchedRef.current;

    if (!previous || previous.zoneId !== zoneId) {
      // 首次挂载 / 切换 Zone：已有状态不算事件，不触发。
      watchedRef.current = { zoneId, signal };
      return;
    }
    watchedRef.current = { zoneId, signal };
    if (previous.signal === signal) return;

    const [prevEnabled, prevColor, prevEffect] = previous.signal.split('|');
    const [nextEnabled, nextColor, nextEffect] = signal.split('|');

    if (prevEnabled === 'off' && nextEnabled === 'on') {
      announceAttentionRequest(requestFor('power-on', zoneId, 'on', color));
    } else if (prevColor && prevColor !== nextColor && nextColor) {
      announceAttentionRequest(requestFor('color-applied', zoneId, nextColor, nextColor));
    } else if (prevEffect && prevEffect !== nextEffect && nextEffect) {
      announceAttentionRequest(requestFor('effect-applied', zoneId, nextEffect, color));
    }
  }, [zoneId, enabled, color, effectValue]);

  return null;
}