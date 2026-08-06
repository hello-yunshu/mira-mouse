// SPDX-License-Identifier: AGPL-3.0-or-later
// 克制型 Attention Beam 的基础类型、优先级、事件键与颜色解析。
//
// 设计原则：本模块只描述「发生了什么状态变化」，不关心具体商品。状态判定继续
// 依赖插件声明、Zone、lightingRole、accentSource 与现有状态映射，
// 不存在厂商硬编码。

export type AttentionBeamVariant = 'line' | 'pulse-inner' | 'flash';

/**
 * 一条等待播放/正在播放的光束请求。
 * eventKey 是会话级去重键：同一 eventKey 一次会话只播放一次（核心仲裁）。
 * scope 标注该光束应渲染在哪个表面（surface），由 useAttentionFeedback 订阅。
 */
export interface AttentionBeamRequest {
  eventKey: string;
  scope: string;
  variant: AttentionBeamVariant;
  /** 任意 CSS 颜色。调用方按 灯光 Zone → declaredAccent → Accent 降饱和 → 中性色 解析。 */
  color: string;
  /** 视觉部分总时长（ms）。pulse-inner 的 cycle 时长 = durationMs / cycles。 */
  durationMs: number;
  /** 峰值不透明度 0..1，渲染层会按明暗主题再微调。 */
  strength: number;
  /** 播放次数。line / flash 固定 1；pulse-inner 上限 2。 */
  cycles: number;
  /** 总时长外的延时（通知浮层淡入后再生效）。 */
  delayMs?: number;
  /** 半径覆盖：默认继承宿主 border-radius。 */
  radius?: string | number;
  /** 仲裁优先级，越大越先播放。 */
  priority: number;
}

/** HID mutation 的成功语义（保持最小兼容；仅当确需读取 runMutation 结果时使用）。 */
export type MutationResult = { ok: true } | { ok: false; error: unknown };

/** 全局同一时刻可保留的排队光束上限；超出按优先级丢弃队尾。 */
export const MAX_ATTENTION_QUEUE = 8;

/** 设备就绪宽限期：应用启动后极短时间内发现设备在线不算“等待→就绪事件”。 */
export const DEVICE_READY_STARTUP_GRACE_MS = 2500;

// ─── 事件优先级（越大越优先播放） ─────────────────────────────────────────
export const ATTENTION_PRIORITY = {
  'restart-required': 120,
  'update-available': 100,
  'lighting-color-applied': 85,
  'lighting-power-on': 75,
  'device-ready': 64,
  'device-reconnected': 60,
  'lighting-effect-applied': 52,
} as const;

// ─── 事件键构造器（与 §九 保持一致；:installed 后缀是本实现为
//     安装完成短闪新增的稳定键，pulse-inner 重启态也是稳定键） ────────────
export function attentionLightingKey(zoneId: string, kind: 'power' | 'color' | 'effect', value: string): string {
  return `lighting:${zoneId}:${kind}:${value}`;
}

export function attentionAppUpdateKey(version: string): string {
  return `update:app:mira:${version}`;
}

export function attentionAppRestartKey(version: string): string {
  return `restart:app:mira:${version}`;
}

export function attentionPluginUpdateKey(pluginId: string, version: string): string {
  return `update:plugin:${pluginId}:${version}`;
}

export function attentionPluginInstalledKey(pluginId: string, version: string): string {
  return `update:plugin:${pluginId}:${version}:installed`;
}

export function attentionLocalAiUpdateKey(componentId: string, version: string): string {
  return `update:local-ai:${componentId}:${version}`;
}

export function attentionLocalAiInstalledKey(componentId: string, version: string): string {
  return `update:local-ai:${componentId}:${version}:installed`;
}

export function attentionDeviceKey(kind: 'ready' | 'reconnected', identity: string, cycle: number): string {
  return `device-${kind}:${identity}:${cycle}`;
}

// ─── 颜色解析 ─────────────────────────────────────────────────────────────

const DEFAULT_ACCENT = '#ffb3b3';

/** 是否使用 Mira 当前解析后的实际主题（light/dark/system → resolved）。 */
export function attentionIsDarkTheme(): boolean {
  if (typeof document === 'undefined') return false;
  const mode = document.documentElement.dataset.theme;
  if (mode === 'dark') return true;
  if (mode === 'light') return false;
  return typeof window !== 'undefined'
    && typeof window.matchMedia === 'function'
    && window.matchMedia('(prefers-color-scheme: dark)').matches;
}

function hexToRgb01(hex: string): [r: number, g: number, b: number] | undefined {
  const match = /^#([0-9a-f]{6})$/i.exec(hex.trim());
  if (!match) return undefined;
  const value = Number.parseInt(match[1], 16);
  return [((value >> 16) & 0xff) / 255, ((value >> 8) & 0xff) / 255, (value & 0xff) / 255];
}

function rgb01ToOklch(r: number, g: number, b: number): { l: number; c: number; h: number } {
  const rl = Math.cbrt(r);
  const gl = Math.cbrt(g);
  const bl = Math.cbrt(b);
  const l = 0.4122214708 * rl + 0.5363325363 * gl + 0.0514459929 * bl;
  const m = 0.2119034982 * rl + 0.6806995451 * gl + 0.1073969566 * bl;
  const s = 0.0883024619 * rl + 0.2817188376 * gl + 0.6299787005 * bl;
  const l_ = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const m_ = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const s_ = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  const lp = l_ - 0.5;
  const mp = m_ - 0.5;
  const sp = s_ - 0.5;
  const c = Math.sqrt(lp * lp + mp * mp + sp * sp);
  const h = (Math.atan2(sp, lp) * 180) / Math.PI + 180;
  return { l: l_, c, h: ((h % 360) + 360) % 360 };
}

/**
 * 灯光 Zone 的实际颜色 → 光束显示色。
 * 只修正 UI 可见性，绝不改变用户设备的实际灯光颜色。
 * 极端暗色校正：红调压低彩度避免刺眼；黑色/近黑提亮至可见下限。
 */
export function attentionColorForZone(color: string, isDark = attentionIsDarkTheme()): string {
  const rgb = hexToRgb01(color);
  if (!rgb) return color;
  const { l, c, h } = rgb01ToOklch(rgb[0], rgb[1], rgb[2]);
  const isRedHue = h < 25 || h > 315;
  const targetL = isDark
    ? Math.min(86, Math.max(62, 64 + (l - 0.5) * 20))
    : Math.min(70, Math.max(55, 60 + (l - 0.5) * 16));
  const chromaMult = isDark ? (isRedHue ? 0.55 : 0.75) : (isRedHue ? 0.45 : 0.62);
  const cap = isDark ? (isRedHue ? 0.09 : 0.13) : (isRedHue ? 0.07 : 0.1);
  const beamC = Math.min(Math.max(c * chromaMult, 0.015), cap);
  return `oklch(${targetL.toFixed(1)}% ${beamC.toFixed(3)} ${h.toFixed(0)})`;
}

/** 当前 Mira 主题的实际 --accent 值（hex 或 oklch）。 */
export function attentionAccentColor(): string {
  if (typeof document === 'undefined') return DEFAULT_ACCENT;
  const value = typeof window !== 'undefined'
    ? window.getComputedStyle(document.documentElement).getPropertyValue('--accent').trim()
    : '';
  return value ? value : DEFAULT_ACCENT;
}

/** 更新类事件：当前 Accent 的降饱和版本（§8.2 第 3 级）。 */
export function attentionDesaturatedAccent(): string {
  return `color-mix(in oklch, ${attentionAccentColor()}, #8a8a8a 30%)`;
}

/** 无可用颜色时的低强度中性色（§8.2 第 4 级）。 */
export function attentionNeutralColor(): string {
  return `color-mix(in oklch, ${attentionAccentColor()}, #8a8a8a 55%)`;
}

// ─── 设备就绪 / 重连状态机（纯函数，便于测试） ─────────────────────────────

export interface DeviceAttentionContext {
  /** 上一次观察到的就绪值（undefined 表示本会话首次观察）。 */
  previous: boolean | undefined;
  /** 本会话是否已经出现过一次到达就绪。 */
  wasReady: boolean;
  /** 断开/重新连接的周期编号，用于 device-* 事件键。 */
  cycle: number;
}

export interface DeviceAttentionOutcome {
  action: 'none' | 'ready' | 'reconnected';
  wasReady: boolean;
  cycle: number;
}

/**
 * 设备可见性状态归约：
 * - 首次观察（previous === undefined）不触发任何事件：
 *   · 启动即在线不算连接事件；
 *   · 启动即不在线只标记 wasReady=false（等待就绪方向）。
 * - 等待/无设备 → 就绪：如果本会话之前从未就绪，则视为 device-ready；
 * - 就绪 → 断开 → 就绪：真实断开再恢复才视为 device-reconnected；
 * - 就绪 → 断开：不播放（§7 禁止“设备断开”光束），只累计周期。
 */
export function reduceDeviceAttention(
  prev: DeviceAttentionContext,
  nextReady: boolean,
  elapsedMs: number,
): DeviceAttentionOutcome {
  if (prev.previous === undefined) {
    return { action: 'none', wasReady: nextReady, cycle: prev.cycle };
  }
  if (prev.previous === nextReady) {
    return { action: 'none', wasReady: prev.wasReady, cycle: prev.cycle };
  }
  if (!prev.previous && nextReady) {
    // 就绪方向：曾就绪过后再恢复 → 重连；否则为首次就绪。
    return {
      action: prev.wasReady && elapsedMs > DEVICE_READY_STARTUP_GRACE_MS ? 'reconnected' : 'ready',
      wasReady: true,
      cycle: prev.cycle,
    };
  }
  // 就绪 → 断开：累计周期序号，供重连时使用。
  return { action: 'none', wasReady: true, cycle: prev.cycle + 1 };
}