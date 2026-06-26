// SPDX-License-Identifier: AGPL-3.0-or-later
// 插件适配层：集中管理 UI ↔ 插件 metadata 契约。
// 此模块不含任何插件特定的字符串常量（mutation 名、灯效名等），
// 所有插件知识均从 capability metadata 强类型字段读取。
import type { DeviceCapabilities, DeviceState, EffectOptions, LightingRole, PluginCapability } from './types';

export interface PluginOption { value: string | number | boolean; label: string }
export interface PluginSummaryItem {
  label: string;
  source: string;
  unit?: string;
  format?: string;
  options: PluginOption[];
}

export const MAX_CONTROL_GROUPS = 6;
export const MAX_STATUS_ITEMS = 6;
export const MAX_CONTROL_OPTIONS = 8;
export const MAX_SUMMARY_ITEMS = 4;

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
/// LightingZone 优先使用强类型 lightingRole；legacy metadata.mutations 只作回退。
/// 其他能力读取优先级：1) metadata.mutation (string) 2) metadata.mutations (object)
/// 3) Host 默认回退。
/// 默认回退：DpiStages → set-dpi-stage/set-dpi-value, LightingZone → set-mouse-lighting/set-receiver-lighting,
/// 其他可写能力 → set-{id}。这些是 Host 标准默认值，不是插件特定内容。
export function pluginMutations(capability: PluginCapability, writableMutations: string[] = []): Record<string, string> {
  const mutations: Record<string, string> = {};
  const lightingRole = capability.control === 'LightingZone'
    ? capability.metadata.lightingRole as LightingRole | undefined
    : undefined;
  if (lightingRole) {
    const applyRole = (role: 'mouse' | 'receiver') => {
      const mutation = lightingRole[role];
      if (typeof mutation === 'string' && (writableMutations.length === 0 || writableMutations.includes(mutation))) {
        mutations[role] = mutation;
      }
    };
    applyRole('mouse');
    applyRole('receiver');
  }
  if (typeof capability.metadata.mutation === 'string') mutations.default = capability.metadata.mutation;
  if (capability.metadata.mutations && typeof capability.metadata.mutations === 'object') {
    for (const [key, value] of Object.entries(capability.metadata.mutations as Record<string, unknown>)) {
      if (mutations[key]) continue;
      const picked = pickMutation(value, writableMutations);
      if (picked) mutations[key] = picked;
    }
  }
  if (Object.keys(mutations).length === 0 && !capability.readOnly) {
    if (capability.control === 'DpiStages') {
      mutations.select = 'set-dpi-stage';
      mutations.value = 'set-dpi-value';
    } else if (capability.control === 'LightingZone') {
      mutations.mouse = 'set-mouse-lighting';
      mutations.receiver = 'set-receiver-lighting';
    } else {
      mutations.default = `set-${capability.id}`;
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
/// 优先从 LightingZone capability 的 metadata.lightingRole 提取 mutation 名，
/// 回退到 metadata.mutations（兼容旧插件/compat 合成能力），再检查 writableMutations。
export function supportsLightingMutation(
  pluginCapabilities: PluginCapability[],
  writableMutations: string[],
  role: 'mouse' | 'receiver',
): boolean {
  for (const cap of pluginCapabilities) {
    if (cap.control !== 'LightingZone') continue;
    // 优先读强类型 lightingRole
    const lightingRole = cap.metadata.lightingRole as LightingRole | undefined;
    if (lightingRole) {
      const mutation = lightingRole[role];
      if (mutation && writableMutations.includes(mutation)) return true;
    }
    // 回退读 legacy mutations（兼容旧插件/compat 合成能力）
    const mutations = cap.metadata.mutations as Record<string, unknown> | undefined;
    if (mutations) {
      const raw = mutations[role];
      const picked = pickMutation(raw, writableMutations);
      if (picked && writableMutations.includes(picked)) return true;
    }
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
      format: typeof record.format === 'string' ? record.format : undefined,
      options: parsePluginOptions(record.options),
    }];
  });
}

/// 为未声明 pluginCapabilities 的设备合成兼容能力。
/// 当设备有 dpiStages 或 writableMutations 包含 set-dpi-* 时，合成 DpiStages 能力；
/// 当有 supportedPollingRates 时，合成 polling-rate 能力；
/// 当有 lighting 数据时，合成 LightingZone 能力。
/// 这是 Host 的向后兼容回退，确保旧设备（无插件声明）仍可渲染控件。
export function compatibilityCapabilities(device: DeviceState): PluginCapability[] {
  if (device.pluginCapabilities.length > 0) return device.pluginCapabilities;
  const capabilities: PluginCapability[] = [];
  if (device.dpiStages.length > 0 || device.writableMutations.some((mutation) => mutation.startsWith('set-dpi-'))) {
    capabilities.push({
      id: 'compat-dpi', control: 'DpiStages', labelKey: 'capability.dpi', readOnly: false,
      placements: [{ region: 'control', group: 'compat-performance', order: 10, span: 1, icon: 'gauge' }],
      metadata: { source: 'dpiStages', mutations: { select: 'set-dpi-stage', value: 'set-dpi-value' } },
    });
  }
  if (device.supportedPollingRates && device.supportedPollingRates.length > 0) {
    capabilities.push({
      id: 'compat-polling-rate', control: 'Select', labelKey: 'capability.polling-rate', readOnly: false,
      placements: [{ region: 'control', group: 'compat-polling', order: 20, span: 1, icon: 'wave' }],
      metadata: {
        source: 'pollingRate', mutation: 'set-polling-rate', param: 'rate', unit: 'Hz',
        options: device.supportedPollingRates.map((value) => ({ value, label: `${value} Hz` })),
        summary: [],
      },
    });
  }
  if (device.lighting) {
    capabilities.push({
      id: 'compat-lighting', control: 'LightingZone', labelKey: 'capability.lighting', readOnly: false,
      placements: [
        { region: 'control', group: 'compat-lighting', order: 30, span: 1, icon: 'lightbulb' },
        { region: 'status', order: 30, span: 1, icon: 'lightbulb' },
      ],
      metadata: { source: 'lighting.color', mutations: { mouse: 'set-mouse-lighting', receiver: 'set-receiver-lighting' }, lightingRole: { mouse: 'set-mouse-lighting', receiver: 'set-receiver-lighting' } },
    });
  }
  return capabilities;
}

export type { DeviceState };
