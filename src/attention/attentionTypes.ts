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

/** 全局同一时刻可保留的排队光束上限；超出按优先级丢弃队尾。 */
export const MAX_ATTENTION_QUEUE = 8;

/** 设备就绪宽限期：应用启动后极短时间内发现设备在线不算“等待→就绪事件”。 */
export const DEVICE_READY_STARTUP_GRACE_MS = 2500;

// ─── 更新类事件的仲裁入口（P0-2） ────────────────────────────────────────
// 更新事件只有一个目标能消费：固定更新区域当前可见 → 固定区域播放；
// 否则 → 通知浮层播放。调用方（App / Settings / About）都走这一个纯函数，
// 避免“不可见固定行先消费、通知被会话去重拒绝”的竞态。

export type AttentionView = 'dashboard' | 'settings' | 'about';

export type UpdateAttentionTarget =
  | 'notification'
  | 'settings-plugin'
  | 'settings-local-ai'
  | 'about'
  | 'none';

export type UpdateAttentionKind = 'app' | 'plugin' | 'local-ai';

export interface UpdateAttentionContext {
  view: AttentionView;
  /** 设置页当前显示标签；非设置页时可不传。 */
  settingsTab?: string;
}

/** 解析一次更新事件应归属哪个可见目标。规则见 §十一：固定区域可见优先于通知。 */
export function resolveUpdateAttentionTarget(
  kind: UpdateAttentionKind,
  context: UpdateAttentionContext,
): UpdateAttentionTarget {
  switch (kind) {
    case 'app':
      return context.view === 'about' ? 'about' : 'notification';
    case 'plugin':
      return context.view === 'settings' && context.settingsTab === 'plugins'
        ? 'settings-plugin'
        : 'notification';
    case 'local-ai':
      return context.view === 'settings' && context.settingsTab === 'plugins'
        ? 'settings-local-ai'
        : 'notification';
  }
}

// ─── 事件优先级（越大越优先播放） ─────────────────────────────────────────
export const ATTENTION_PRIORITY = {
  'restart-required': 120,
  'update-available': 100,
  'update-installed': 90,
  'lighting-color-applied': 85,
  'lighting-power-on': 75,
  'device-ready': 64,
  'device-reconnected': 60,
  'lighting-effect-applied': 52,
} as const;

// ─── 事件键构造器（与 §九 保持一致；:installed 后缀是本实现为
//     安装完成短闪新增的稳定键，pulse-inner 重启态也是稳定键） ────────────
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

/**
 * Hex 颜色标准化：#rgb / #rrggbb / 任意大小写 → 小写六位 #rrggbb。
 * 无效值（非字符串 / 非合法 hex）返回 undefined。
 */
export function normalizeHexColor(value: unknown): string | undefined {
  if (typeof value !== 'string') return undefined;
  const trimmed = value.trim();
  const match = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(trimmed);
  if (!match) return undefined;
  const raw = match[1].toLowerCase();
  if (raw.length === 3) {
    return `#${raw.split('').map((digit) => `${digit}${digit}`).join('')}`;
  }
  return `#${raw}`;
}

/**
 * 颜色比较标准化：Hex 展开并转小写；非 Hex CSS 字符串 trim 后原样保留；
 * 空字符串返回 undefined。不引入完整 CSS 颜色解析器。
 */
export function normalizeComparableColor(value: unknown): string | undefined {
  if (typeof value !== 'string') return undefined;
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  return normalizeHexColor(trimmed) ?? trimmed;
}

/**
 * Hex → RGB 01 分量。
 * 先经 normalizeHexColor 展开为六位再按 24 位解析，三位 hex 不再做乘 17 修正。
 */
function hexToRgb01(value: string): [r: number, g: number, b: number] | undefined {
  const normalized = normalizeHexColor(value);
  if (!normalized) return undefined;
  const numeric = Number.parseInt(normalized.slice(1), 16);
  return [
    ((numeric >> 16) & 0xff) / 255,
    ((numeric >> 8) & 0xff) / 255,
    (numeric & 0xff) / 255,
  ];
}

/**
 * 灯光 Zone 的实际颜色 → 光束显示色。
 *
 * 只返回普通 CSS 颜色（#rrggbb 或安全字面名称），绝不返回 color-mix：
 * 旧 WebView 不支持 color-mix 时，CSS fallback 仍能生效；混色交给
 * CSS 层的 modern override。绝不改变用户设备的实际灯光颜色：
 * - 近黑：提亮至可见中性灰（不产生彩色）；
 * - 近白：压暗避免过亮（不产生彩色）；
 * - 灰色：保持低色度；
 * - 普通彩色：原样返回，色相完全由原色决定；
 * - 无效 / 非 hex 颜色：trim 后原样返回，不抛异常。
 */
export function attentionColorForZone(color: string, isDark = attentionIsDarkTheme()): string {
  const normalized = normalizeHexColor(color);
  if (!normalized) return color.trim();
  const rgb = hexToRgb01(normalized);
  if (!rgb) return normalized;
  const [r, g, b] = rgb;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const chroma = max - min;
  if (max <= 0.14) {
    // 近黑：暗色主题给中性深灰，亮色主题给中性浅灰（不产生彩色）。
    return isDark ? '#8f8f8f' : '#5f5f5f';
  }
  if (min >= 0.86) {
    // 近白：暗色主题给中性浅灰，亮色主题给中性中灰。
    return isDark ? '#b8b8b8' : '#666666';
  }
  if (chroma < 0.05) {
    // 灰色：保持低色度，不引入额外色相。
    return isDark ? '#a0a0a0' : '#686868';
  }
  // 普通彩色：色相完全由原色决定，原样返回。
  return normalized;
}

/** 当前 Mira 主题的实际 --accent 值（hex 或 oklch）。 */
export function attentionAccentColor(): string {
  if (typeof document === 'undefined') return DEFAULT_ACCENT;
  const value = typeof window !== 'undefined'
    ? window.getComputedStyle(document.documentElement).getPropertyValue('--accent').trim()
    : '';
  return value ? value : DEFAULT_ACCENT;
}

/** 更新类事件：当前 Accent 的降饱和版本（§8.2 第 3 级）。
 * 只返回普通 CSS 颜色，绝不返回 color-mix：旧 WebView 下 CSS fallback 依旧生效，
 * 混色视觉留给 CSS 层 modern override。Accent 为 hex 时按比例向灰混合；
 * 非 hex（如 oklch 现代主题色）原样保留——它本身即是安全的普通 CSS 颜色。 */
export function attentionDesaturatedAccent(): string {
  const accent = attentionAccentColor();
  const normalized = normalizeHexColor(accent);
  return normalized ? mixHexTowardGray(normalized, 0.3) : accent;
}

/** 无可用颜色时的低强度中性色（§8.2 第 4 级）。同样只返回普通安全颜色。 */
export function attentionNeutralColor(): string {
  const accent = attentionAccentColor();
  const normalized = normalizeHexColor(accent);
  return normalized ? mixHexTowardGray(normalized, 0.55) : accent;
}

/**
 * 把 #rrggbb 按比例向中性灰混合（无任何外部颜色库）。
 * amount 越大越接近灰。仅用于把最终产物控制在普通 hex，
 * 不在 TS 层发出 color-mix。
 */
function mixHexTowardGray(hex: string, toward: number): string {
  const rgb = hexToRgb01(hex);
  if (!rgb) return hex;
  const [r, g, b] = rgb.map((channel) => {
    const gray = 0.5;
    return Math.round((channel + (gray - channel) * toward) * 255);
  });
  return `#${[r, g, b].map((channel) => channel.toString(16).padStart(2, '0')).join('')}`;
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

/**
 * 按稳定设备身份隔离的多设备状态机入口（P1-4）。
 * 每个身份（deviceKey / 插件+family）维护独立的 previous / wasReady / cycle，
 * 切换设备、同名设备、一个离线设备切到另一个在线设备都不会互相继承状态。
 * 返回的 outcome 与 reduceDeviceAttention 相同，并已写回该身份的上下文。
 */
export function reduceDeviceAttentionByIdentity(
  contexts: Map<string, DeviceAttentionContext>,
  identity: string,
  nextReady: boolean,
  elapsedMs: number,
): DeviceAttentionOutcome {
  const prev = contexts.get(identity) ?? { previous: undefined, wasReady: false, cycle: 0 };
  const outcome = reduceDeviceAttention(prev, nextReady, elapsedMs);
  contexts.set(identity, { previous: nextReady, wasReady: outcome.wasReady, cycle: outcome.cycle });
  return outcome;
}