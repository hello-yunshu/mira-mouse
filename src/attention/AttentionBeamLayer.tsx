// SPDX-License-Identifier: AGPL-3.0-or-later
// AttentionBeamLayer —— 唯一负责视觉绘制的底层组件。
//
// 约定：
// - position: absolute + inset: 0 + border-radius: inherit + pointer-events: none；
// - 不改变宿主组件尺寸、排版与定位；
// - 一次性动画：line / flash 默认 1 次，pulse-inner 最多 2 次，禁止无限循环；
// - 动画结束后内部视觉节点自动卸载（由父组件在 onFinished 后移除）；
// - 不使用外部 Bloom；光晕只存在于自身边框内部；
// - Reduced Motion：CSS 层切换为约 220ms 的静态边框淡入淡出；
// - aria-hidden，不参与 Tab 顺序，不影响按钮点击。

import { useEffect, useRef, useState, type CSSProperties } from 'react';
import {
  attentionIsDarkTheme,
  attentionNeutralColor,
  type AttentionBeamRequest,
  type AttentionBeamVariant,
} from './attentionTypes';
import {
  attentionRequestTotalMs,
  prefersReducedAttentionMotion,
} from './attentionTiming';

export interface AttentionBeamLayerProps {
  active: boolean;
  eventKey?: string;
  variant?: AttentionBeamVariant;
  color?: string;
  durationMs?: number;
  strength?: number;
  cycles?: number;
  delayMs?: number;
  radius?: string | number;
  onFinished?: () => void;
  /** 便捷入口：传入一条完整请求，优先于上面的单项 props。 */
  request?: AttentionBeamRequest;
}

function clampOpacity(strength: number, isDark: boolean): number {
  const scaled = strength * (isDark ? 1.05 : 0.66);
  return Math.min(0.95, Math.max(0.06, scaled));
}

export function AttentionBeamLayer({
  active,
  eventKey,
  variant = 'line',
  color,
  durationMs = 1600,
  strength = 0.2,
  cycles = 1,
  delayMs = 0,
  radius,
  onFinished,
  request,
}: AttentionBeamLayerProps) {
  const effectiveVariant: AttentionBeamVariant = request?.variant ?? variant;
  const effectiveColor = (request?.color ?? color ?? '').trim() || attentionNeutralColor();
  const effectiveDuration = request?.durationMs ?? durationMs;
  const effectiveStrength = request?.strength ?? strength;
  const effectiveDelay = request?.delayMs ?? delayMs;
  const effectiveRadius = request?.radius ?? radius;
  // durationMs 是视觉部分总时长；line / flash 固定 1 次，pulse-inner 最多 2 次。
  // 每个 cycle 的时长 = durationMs / cycleCount，总时长 = durationMs（不再乘以次数）。
  const cycleCount = effectiveVariant === 'pulse-inner'
    ? Math.min(2, Math.max(1, Math.round(request?.cycles ?? cycles)))
    : 1;
  const cycleDuration = effectiveDuration / cycleCount;
  const eventKeyValue = request?.eventKey ?? eventKey ?? '';

  const isDark = attentionIsDarkTheme();
  const beamOpacity = clampOpacity(effectiveStrength, isDark);
  const [finishedKey, setFinishedKey] = useState<string | null>(null);
  const finishedRef = useRef(false);

  // 新请求到来时在渲染期间重置 finished（React 官方的派生状态模式），
  // 而不是在 effect 里同步 setState；事件键相同（会话去重保证不会重播）
  // 或层已卸载时不重置。
  if (finishedKey !== null && finishedKey !== eventKeyValue) {
    setFinishedKey(null);
  }

  useEffect(() => {
    // inactive 时不启动完成计时器：不渲染时不应在稍后触发 onFinished（P2-1）。
    if (!active) {
      finishedRef.current = false;
      return;
    }
    finishedRef.current = false;
    // 与 AttentionBusController 共用统一时长：Layer 计时只负责视觉节点完成
    // 与 onFinished，不推进全局总线。
    const total = attentionRequestTotalMs(
      { delayMs: effectiveDelay, durationMs: effectiveDuration },
      prefersReducedAttentionMotion(),
    );
    const timer = window.setTimeout(() => {
      if (!finishedRef.current) {
        finishedRef.current = true;
        setFinishedKey(eventKeyValue);
        onFinished?.();
      }
    }, total);
    return () => {
      window.clearTimeout(timer);
    };
  }, [active, eventKeyValue, effectiveDelay, effectiveDuration, onFinished]);

  if (!active || finishedKey !== null) return null;

  const cycleStyle = (index: number): CSSProperties => ({
    animationDelay: `${effectiveDelay + index * cycleDuration}ms`,
  });

  return (
    <div
      className={`attention-beam attention-beam--${effectiveVariant}`}
      data-event-key={eventKeyValue}
      data-variant={effectiveVariant}
      style={{
        '--beam-color': effectiveColor,
        '--beam-o': String(beamOpacity),
        '--beam-duration': `${cycleDuration}ms`,
        borderRadius: effectiveRadius ?? 'inherit',
      } as CSSProperties}
      aria-hidden="true"
    >
      {Array.from({ length: cycleCount }, (_, index) => (
        <span key={index} className="attention-beam__cycle" style={cycleStyle(index)} />
      ))}
    </div>
  );
}