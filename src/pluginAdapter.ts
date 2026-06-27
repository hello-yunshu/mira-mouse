// SPDX-License-Identifier: AGPL-3.0-or-later
// 插件适配层：集中管理 UI ↔ 插件 metadata 契约。
// 此模块不含任何插件特定的字符串常量（mutation 名、灯效名等），
// 所有插件知识均从 capability metadata 强类型字段读取。
import type { DeviceCapabilities, DeviceState, EffectOptions, LightingRole, PluginCapability, PluginValueFormat, RangeSpec } from './types';

export interface PluginOption { value: string | number | boolean; label: string }
export interface PluginSummaryItem {
  label: string;
  source: string;
  unit?: string;
  format?: PluginValueFormat;
  options: PluginOption[];
}

export const MAX_CONTROL_GROUPS = 6;
export const MAX_STATUS_ITEMS = 6;
export const MAX_CONTROL_OPTIONS = 8;
export const MAX_SUMMARY_ITEMS = 4;
export const PLUGIN_VALUE_FORMATS: readonly PluginValueFormat[] = ['sleep', 'color'];

export function pluginValueFormat(value: unknown): PluginValueFormat | undefined {
  return typeof value === 'string' && PLUGIN_VALUE_FORMATS.includes(value as PluginValueFormat)
    ? value as PluginValueFormat
    : undefined;
}

/// 从 capability.metadata.range 强类型字段读取数值范围。
/// 用于 DpiStages / Number / Slider 等需要范围约束的控件，
/// 与 effectOptions.speed/brightness 的 RangeSpec 类型一致。
/// 插件必须通过 metadata.range 声明范围，Host 不再硬编码 50/30000/10/65535 等回退值。
export function pluginRange(capability: PluginCapability): RangeSpec | undefined {
  const raw = capability.metadata.range;
  if (!raw || typeof raw !== 'object') return undefined;
  const range = raw as Record<string, unknown>;
  if (typeof range.min !== 'number' || typeof range.max !== 'number') return undefined;
  return {
    min: range.min,
    max: range.max,
    step: typeof range.step === 'number' ? range.step : undefined,
  };
}

/// 从 effectOptions 提取灯效写入参数的默认值（当设备未报送当前值时使用）。
/// - effect: 首个非 offValue 的灯效值
/// - speed: speedRange.min
/// - brightness: brightnessRange.min
/// 替代 UI 硬编码 effect=1 / speed=0 / brightness=100。
export function effectDefaults(capability: PluginCapability | undefined): {
  effect: number;
  speed: number;
  brightness: number;
} {
  const opts = capability ? effectOptions(capability) : undefined;
  const off = opts?.offValue ?? 0;
  const effect = opts?.effect?.find((e) => e.value !== off)?.value ?? 1;
  const speed = opts?.speed?.min ?? 0;
  const brightness = opts?.brightness?.min ?? 100;
  return { effect, speed, brightness };
}

/// 从 mutation 字段（string 或 string[]）中选取第一个可写的 mutation。
/// 插件可声明多个候选 mutation（如 ['set-polling-rate', 'set-polling-rate-extended']），
/// UI 优先选择设备支持的写入命令。
export function pickMutation(value: unknown, writableMutations: string[]): string | undefined {
  if (typeof value === 'string') return value;
  if (Array.isArray(value)) {
    const candidates = value.filter((m): m is string => typeof m === 'string');
    return candidates.find((m) => writableMutations.includes(m)) ?? candidates[0];
  }
  return undefined;
}

export function parsePluginOptions(value: unknown): PluginOption[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((option) => {
    if (!option || typeof option !== 'object') return [];
    const optionValue = (option as Record<string, unknown>).value;
    const label = (option as Record<string, unknown>).label;
    return (typeof optionValue === 'string' || typeof optionValue === 'number' || typeof optionValue === 'boolean') && typeof label === 'string'
      ? [{ value: optionValue, label }]
      : [];
  });
}

/// 从 capability metadata 提取 mutation 映射。
/// LightingZone 只使用强类型 lightingRole，由插件声明鼠标/接收器角色。
/// lightingRole.mouse/receiver 可为 string 或 string[]：数组时按优先级
/// 选取第一个被 writableMutations 支持的 mutation。
/// 其他能力读取优先级：1) metadata.mutation (string 或 string[]) 2) metadata.mutations (object)
export function pluginMutations(capability: PluginCapability, writableMutations: string[] = []): Record<string, string> {
  const mutations: Record<string, string> = {};
  const lightingRole = capability.control === 'LightingZone'
    ? capability.metadata.lightingRole as LightingRole | undefined
    : undefined;
  if (lightingRole) {
    const applyRole = (role: 'mouse' | 'receiver') => {
      const picked = pickMutation(lightingRole[role], writableMutations);
      if (picked) mutations[role] = picked;
    };
    applyRole('mouse');
    applyRole('receiver');
    return mutations;
  }
  const defaultMutation = pickMutation(capability.metadata.mutation, writableMutations);
  if (defaultMutation) mutations.default = defaultMutation;
  if (capability.metadata.mutations && typeof capability.metadata.mutations === 'object') {
    for (const [key, value] of Object.entries(capability.metadata.mutations as Record<string, unknown>)) {
      if (mutations[key]) continue;
      const picked = pickMutation(value, writableMutations);
      if (picked) mutations[key] = picked;
    }
  }
  return mutations;
}

/// 从 capability metadata 提取选项列表。
/// 对于 polling-rate 能力，如果设备运行时报告了 supportedPollingRates，
/// 优先使用设备报告的速率列表（设备能力覆盖插件静态声明）。
export function pluginOptions(capability: PluginCapability, device?: DeviceState): PluginOption[] {
  if (capability.labelKey === 'capability.polling-rate' && device?.supportedPollingRates?.length) {
    return device.supportedPollingRates
      .slice(0, MAX_CONTROL_OPTIONS)
      .map((value) => ({ value, label: `${value} Hz` }));
  }
  return parsePluginOptions(capability.metadata.options).slice(0, MAX_CONTROL_OPTIONS);
}

/// 从 capability metadata 的强类型 effectOptions 字段读取灯效选项。
export function effectOptions(capability: PluginCapability): EffectOptions | undefined {
  const raw = capability.metadata.effectOptions;
  if (!raw || typeof raw !== 'object') return undefined;
  return raw as EffectOptions;
}

/// 从 effectOptions.offValue 读取"关闭"值（替代硬编码 effect === 0）。
export function offValue(capability: PluginCapability): number | undefined {
  return effectOptions(capability)?.offValue;
}

/// 判断某灯效是否需要 extraColor（替代硬编码 effect === 5）。
export function requiresExtraColor(capability: PluginCapability, effectValue: number): boolean {
  const opts = effectOptions(capability);
  if (!opts?.effect) return false;
  const match = opts.effect.find((e) => e.value === effectValue);
  return match?.requiresExtraColor ?? false;
}

/// 检查设备是否支持灯光类 mutation（供 Settings 夜间模式使用）。
/// 只从 LightingZone capability 的 metadata.lightingRole 提取 mutation 名。
/// lightingRole 字段可为 string 或 string[]（按优先级排序的候选）；
/// 要求至少有一个候选被 writableMutations 显式支持。
export function supportsLightingMutation(
  pluginCapabilities: PluginCapability[],
  writableMutations: string[],
  role: 'mouse' | 'receiver',
): boolean {
  for (const cap of pluginCapabilities) {
    if (cap.control !== 'LightingZone') continue;
    const lightingRole = cap.metadata.lightingRole as LightingRole | undefined;
    if (!lightingRole) continue;
    const declared = lightingRole[role];
    const candidates = typeof declared === 'string' ? [declared] : Array.isArray(declared) ? declared : [];
    if (candidates.some((mutation) => writableMutations.includes(mutation))) return true;
  }
  return false;
}

/// 检查设备是否支持任意灯光 mutation（鼠标或接收器）。
export function supportsAnyLighting(
  pluginCapabilities: PluginCapability[],
  writableMutations: string[],
): boolean {
  return supportsLightingMutation(pluginCapabilities, writableMutations, 'mouse')
    || supportsLightingMutation(pluginCapabilities, writableMutations, 'receiver');
}

/// 读取设备支持的灯效值列表。
/// 优先从 device capabilities 的 supportedEffects 字段读取（由插件 protocol 解析提供），
/// 如果未提供则返回 effectOptions 中所有灯效值。
export function supportedEffectValues(
  capability: PluginCapability,
  deviceCapabilities: DeviceCapabilities,
): number[] {
  const opts = effectOptions(capability);
  if (!opts?.effect) return [];
  // 插件可在设备能力中提供 supportedEffects 数组（运行时计算的支持列表）
  const lightingCap = deviceCapabilities?.mouseLighting;
  if (lightingCap && Array.isArray(lightingCap.supportedEffects)) {
    return lightingCap.supportedEffects as number[];
  }
  // 回退：所有声明的灯效都可用
  return opts.effect.map((e) => e.value);
}

export function pluginSummaryItems(capability: PluginCapability): PluginSummaryItem[] {
  if (!Array.isArray(capability.metadata.summary)) return [];
  return capability.metadata.summary.slice(0, MAX_SUMMARY_ITEMS).flatMap((item) => {
    if (!item || typeof item !== 'object') return [];
    const record = item as Record<string, unknown>;
    if (typeof record.label !== 'string' || typeof record.source !== 'string') return [];
    return [{
      label: record.label,
      source: record.source,
      unit: typeof record.unit === 'string' ? record.unit : undefined,
      format: pluginValueFormat(record.format),
      options: parsePluginOptions(record.options),
    }];
  });
}

/// 返回插件声明的能力。Host 不根据设备数据合成可写控件。
export function compatibilityCapabilities(device: DeviceState): PluginCapability[] {
  return device.pluginCapabilities;
}

export type { DeviceState };
