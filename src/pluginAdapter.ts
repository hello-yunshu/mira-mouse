// SPDX-License-Identifier: AGPL-3.0-or-later
// 插件适配层：声明式 capability metadata 解析纯函数。
// 此模块不含任何插件特定的字符串常量，所有插件知识均从 capability metadata 声明字段读取。
import type { DeviceState, PluginCapability, PluginField, PluginFieldOption, PluginMutation, PluginStageLayout, PluginStateMapping, PluginStatusDisplay, PluginSwitch, PluginVisibleWhen, PluginZone, RangeSpec } from './types';
import { resolveLabelKey } from './i18n';

export const MAX_CONTROL_GROUPS = 6;
export const MAX_STATUS_ITEMS = 6;
export const MAX_CONTROL_OPTIONS = 8;

/** 从插件声明的 mutation 候选中选择设备实际允许的第一项。 */
export function resolveMutation(mutation: PluginMutation | undefined, writableMutations: string[]): string | undefined {
  if (typeof mutation === 'string') return writableMutations.includes(mutation) ? mutation : undefined;
  if (!Array.isArray(mutation)) return undefined;
  return mutation.find((candidate) => writableMutations.includes(candidate));
}

/// 通用路径读取函数。支持点号分隔路径如 'state.mouseLightColor'、
/// 'capabilities.mouseLighting.effectName'、'batteries.0.percentage'。
/// 路径前缀决定读取根对象：state. → device.state，capabilities. → device.capabilities，
/// batteries. → device.batteries，其他则从 device 顶层属性读取。
/// 支持数组索引（如 batteries.0.percentage）。路径不存在时返回 undefined。
export function readPath(device: DeviceState, path: string): unknown {
  const parts = path.split('.');
  if (parts.length === 0) return undefined;
  const head = parts[0];
  let root: unknown;
  if (head === 'state') {
    root = device.state;
  } else if (head === 'capabilities') {
    root = device.capabilities;
  } else if (head === 'batteries') {
    root = device.batteries;
  } else {
    root = (device as unknown as Record<string, unknown>)[head];
  }
  let current: unknown = root;
  for (let i = 1; i < parts.length; i++) {
    if (current == null) return undefined;
    const part = parts[i];
    if (Array.isArray(current)) {
      const idx = Number(part);
      if (!Number.isInteger(idx)) return undefined;
      current = current[idx];
    } else if (typeof current === 'object') {
      current = (current as Record<string, unknown>)[part];
    } else {
      return undefined;
    }
  }
  return current;
}

/// 对 {path, eq?, ne?} 条件求值。
/// 无 condition 时返回 true；有 eq 时返回 value === eq；有 ne 时返回 value !== ne；
/// 都没有时返回 value != null。
export function resolveVisibleWhen(condition: PluginVisibleWhen | undefined, device: DeviceState): boolean {
  if (!condition) return true;
  const value = readPath(device, condition.path);
  if (condition.eq !== undefined) return value === condition.eq;
  if (condition.ne !== undefined) return value !== condition.ne;
  return value != null;
}

/// 读 field.switch 判断开关状态。
/// 无 switch 时返回 true；否则用 readPath 读取 switch.source 的值，返回 value !== switch.offValue。
export function resolveSwitchState(field: PluginField, device: DeviceState): boolean {
  const sw: PluginSwitch | undefined = field.switch;
  if (!sw) return true;
  const value = readPath(device, sw.source);
  return value !== sw.offValue;
}

/// 标签解析：labelSource 优先 → options 匹配 → labelKey i18n 回退。
/// - 有 labelSource 时用 readPath 读取，若非空则返回 String(value)
/// - 有 options 时用 readPath 读取 field.source 的值，在 options 中匹配，返回 resolveLabelKey(matched.labelKey)
/// - 有 labelKey 时返回 resolveLabelKey(field.labelKey, pluginId)
/// - 都没有时返回空字符串
export function resolveFieldLabel(field: PluginField, device: DeviceState, pluginId?: string): string {
  if (field.labelSource) {
    const value = readPath(device, field.labelSource);
    if (value != null && value !== '') return String(value);
  }
  if (field.options) {
    const value = readPath(device, field.source);
    const match = field.options.find((option) => option.value === value);
    if (match) return resolveLabelKey(match.labelKey, pluginId);
  }
  if (field.labelKey) return resolveLabelKey(field.labelKey, pluginId);
  return '';
}

/// 选项解析：合并 field.options 和 field.optionSource。
/// 无 optionSource 时直接返回 field.options。
/// 有 optionSource 时用 readPath 读取设备运行时选项数组，与 field.options 合并
/// （optionSource 优先但限制在 MAX_CONTROL_OPTIONS 内）。
export function resolveFieldOptions(field: PluginField, device: DeviceState): PluginFieldOption[] {
  const declared = field.options ?? [];
  if (!field.optionSource) return declared;
  const runtimeRaw = readPath(device, field.optionSource);
  if (!Array.isArray(runtimeRaw)) return declared;
  const runtime: PluginFieldOption[] = runtimeRaw.map((item) => {
    if (item && typeof item === 'object') {
      const record = item as Record<string, unknown>;
      const value = record.value;
      const labelKey = record.labelKey;
      if (
        (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean')
        && typeof labelKey === 'string'
      ) {
        return { value, labelKey };
      }
    }
    // 原始值：尝试匹配 declared 选项获取 labelKey，否则用 String(value)
    const declaredMatch = declared.find((opt) => opt.value === item);
    return declaredMatch ?? { value: item as string | number | boolean, labelKey: String(item) };
  });
  // optionSource 优先：runtime 在前，declared 补足，限制在 MAX_CONTROL_OPTIONS 内
  const merged: PluginFieldOption[] = [];
  const seen = new Set<unknown>();
  for (const option of runtime) {
    if (seen.has(option.value)) continue;
    seen.add(option.value);
    merged.push(option);
  }
  for (const option of declared) {
    if (seen.has(option.value)) continue;
    seen.add(option.value);
    merged.push(option);
  }
  return merged.slice(0, MAX_CONTROL_OPTIONS);
}

/// 读 field.range。
export function resolveFieldRange(field: PluginField): RangeSpec | undefined {
  return field.range;
}

/// 读 capability.metadata.stageLayout。
export function resolveStageLayout(capability: PluginCapability): PluginStageLayout | undefined {
  return capability.metadata.stageLayout;
}

/// 读 capability.metadata.zones，过滤 visibleWhen 后返回可见区域。
export function resolveZones(capability: PluginCapability, device: DeviceState): PluginZone[] {
  const zones = capability.metadata.zones;
  if (!zones) return [];
  return zones.filter((zone) => resolveVisibleWhen(zone.visibleWhen, device));
}

/// 读 capability.metadata.statusDisplay。
export function resolveStatusDisplay(capability: PluginCapability): PluginStatusDisplay | undefined {
  return capability.metadata.statusDisplay;
}

/// 聚合所有 capability 的 metadata.stateMapping，返回合并的字段→source 路径映射。
export function resolveStateMapping(capabilities: PluginCapability[]): PluginStateMapping {
  const mapping: PluginStateMapping = {};
  for (const capability of capabilities) {
    const sm = capability.metadata.stateMapping;
    if (sm) {
      for (const [field, source] of Object.entries(sm)) {
        mapping[field] = source;
      }
    }
  }
  return mapping;
}

/// 从所有 LightingZone capability 的 zones[].fields[].mutation 收集灯光 mutation，
/// 筛选在 writableMutations 中的。替代 supportsLightingMutation/supportsAnyLighting。
export function resolveLightingMutations(capabilities: PluginCapability[], writableMutations: string[]): string[] {
  const result: string[] = [];
  const seen = new Set<string>();
  for (const capability of capabilities) {
    if (capability.control !== 'LightingZone') continue;
    const zones = capability.metadata.zones;
    if (!zones) continue;
    for (const zone of zones) {
      for (const field of zone.fields) {
        const mutation = resolveMutation(field.mutation, writableMutations);
        if (!mutation) continue;
        if (seen.has(mutation)) continue;
        seen.add(mutation);
        result.push(mutation);
      }
    }
  }
  return result;
}

export type { DeviceState };
