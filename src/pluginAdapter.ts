// SPDX-License-Identifier: AGPL-3.0-or-later
// 插件适配层：声明式 capability metadata 解析纯函数。所有插件知识均从 metadata 声明字段读取。
import type { DeviceState, DpiStage, PluginCapability, PluginField, PluginFieldOption, PluginMutation, PluginStageLayout, PluginStateMapping, PluginStatusDisplay, PluginSwitch, PluginVisibleWhen, PluginZone, RangeSpec } from './types';
import { resolveLabelKey, resolveRuntimeText } from './i18n';

export const MAX_CONTROL_GROUPS = 6;
export const MAX_STATUS_ITEMS = 6;
export const MAX_CONTROL_OPTIONS = 8;

/** 从插件声明的 mutation 候选中选择设备实际允许的第一项。 */
export function resolveMutation(mutation: PluginMutation | undefined, writableMutations: string[]): string | undefined {
  if (typeof mutation === 'string') return writableMutations.includes(mutation) ? mutation : undefined;
  if (!Array.isArray(mutation)) return undefined;
  return mutation.find((candidate) => writableMutations.includes(candidate));
}

/// 通用路径读取函数。支持点号分隔路径如 'state.mouseLightColor'、'batteries.0.percentage'。
/// 路径前缀决定根对象：state.→device.state, capabilities.→device.capabilities,
/// batteries.→device.batteries, 其他→device 顶层属性。支持数组索引。
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

/// 通用路径写入函数。语义与 readPath 对称。路径前缀同 readPath。
/// 路径中任何中间节点为 null/非对象/数组越界时静默返回（不抛错）。
export function writePath(device: DeviceState, path: string, value: unknown): void {
  const parts = path.split('.');
  if (parts.length === 0) return;
  const head = parts[0];
  let root: unknown;
  if (head === 'state') root = device.state;
  else if (head === 'capabilities') root = device.capabilities;
  else if (head === 'batteries') root = device.batteries;
  else root = (device as unknown as Record<string, unknown>)[head];

  let current: unknown = root;
  for (let i = 1; i < parts.length - 1; i++) {
    if (current == null) return;
    const part = parts[i];
    if (Array.isArray(current)) {
      const idx = Number(part);
      if (!Number.isInteger(idx)) return;
      current = current[idx];
    } else if (typeof current === 'object') {
      current = (current as Record<string, unknown>)[part];
    } else {
      return;
    }
  }
  const lastPart = parts[parts.length - 1];
  if (current == null) return;
  if (Array.isArray(current)) {
    const idx = Number(lastPart);
    if (!Number.isInteger(idx)) return;
    current[idx] = value;
  } else if (typeof current === 'object') {
    (current as Record<string, unknown>)[lastPart] = value;
  }
}

/**
 * 解析插件声明的组合写入参数。
 *
 * 某些设备 mutation 必须一次写入完整结构（例如灯效、速度、亮度、颜色），
 * 即使用户只修改其中一项。插件用 paramSources 声明其余参数的快照路径，Host
 * 仅负责读取、合并，并让本次编辑值覆盖同名参数。
 */
export function resolveFieldParams(field: PluginField, device: DeviceState): Record<string, unknown> {
  const resolved: Record<string, unknown> = {};
  for (const [param, source] of Object.entries(field.paramSources ?? {})) {
    const value = readPath(device, source);
    if (value !== undefined) resolved[param] = value;
  }
  // params 提供插件声明的兜底值；快照中真实存在的读数应优先覆盖兜底。
  return { ...field.params, ...resolved };
}

export function resolveFieldMutationParams(
  field: PluginField,
  device: DeviceState,
  nextValue: unknown,
): Record<string, unknown> {
  return {
    ...resolveFieldParams(field, device),
    [field.param ?? 'value']: nextValue,
  };
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

/**
 * A dashboard field is useful only after the device has reported its current
 * value.  Action fields are the exception: they intentionally represent an
 * operation rather than a reading.
 *
 * This is a host-wide safety net for optional plugin capabilities.  Plugins
 * should still declare probes/visibleWhen gates, but a missing runtime value
 * must never turn into an editable "not reported" control.
 */
export function fieldHasReportedValue(field: PluginField, device: DeviceState): boolean {
  if (!resolveVisibleWhen(field.visibleWhen, device)) return false;
  if (field.editor === 'inline-action') return true;

  const value = readPath(device, field.switch?.source ?? field.source);
  return value !== undefined && value !== null && value !== '';
}

/// 读 field.switch 判断开关状态。
/// 无 switch 时返回 true；否则用 readPath 读取 switch.source 的值，返回 value !== switch.offValue。
export function resolveSwitchState(field: PluginField, device: DeviceState): boolean {
  const sw: PluginSwitch | undefined = field.switch;
  if (!sw) return true;
  const value = readPath(device, sw.source);
  return value !== sw.offValue;
}

/** 状态卡片与字段控件共用的交互类型。 */
export type PluginFieldInteraction = 'toggle' | 'action' | 'modal' | 'control';

/**
 * 根据字段自己的 editor 契约决定点击行为。
 *
 * 宿主不感知 capability、设备或厂商名称：弹窗字段打开编辑器，开关和动作
 * 直接执行，其余需要多个选项或专用布局的字段回到 capability 控制区。
 */
export function resolveFieldInteraction(field: PluginField): PluginFieldInteraction {
  switch (field.editor) {
    case 'inline-toggle':
      return 'toggle';
    case 'inline-action':
      return 'action';
    case 'modal-select':
    case 'modal-color':
    case 'modal-range':
    case 'modal-number':
    case 'modal-gradient':
      return 'modal';
    default:
      return 'control';
  }
}

/**
 * 解析 inline-toggle 下一次应写入的值。
 *
 * 布尔开关可由 offValue 直接反转；枚举开关优先恢复调用方记住的非关闭值，
 * 再回退到插件声明的第一个非关闭选项。返回 undefined 表示契约没有提供安全
 * 的恢复值，此时宿主不猜测设备语义。
 */
export function resolveSwitchNextValue(
  field: PluginField,
  device: DeviceState,
  rememberedOnValue?: unknown,
): unknown | undefined {
  const sw = field.switch;
  if (!sw) return readPath(device, field.source) !== true;

  const currentValue = readPath(device, sw.source);
  if (currentValue !== sw.offValue) return sw.offValue;
  if (rememberedOnValue !== undefined && rememberedOnValue !== sw.offValue) return rememberedOnValue;

  const declaredOnValue = resolveFieldOptions(field, device)
    .find((option) => option.value !== sw.offValue)?.value;
  if (declaredOnValue !== undefined) return declaredOnValue;
  if (typeof sw.offValue === 'boolean') return !sw.offValue;
  return undefined;
}

/// 字段标题只来自插件声明的 labelKey；运行时 labelSource 和 options 描述的是值。
export function resolveFieldLabel(field: PluginField, device: DeviceState, pluginId?: string): string {
  void device;
  if (field.labelKey) return resolveLabelKey(field.labelKey, pluginId);
  return '';
}

/// 解析字段当前值的友好名称。声明选项的 labelKey 可随当前语言翻译，
/// 因此已知选项优先；运行时 labelSource 只用于插件未声明的动态值。
export function resolveFieldValueLabel(field: PluginField, device: DeviceState, pluginId?: string): string | undefined {
  if (field.options) {
    const value = readPath(device, field.source);
    const match = field.options.find((option) => option.value === value);
    if (match) {
      const resolved = resolveLabelKey(match.labelKey, pluginId);
      if (resolved !== match.labelKey || !match.labelKey.includes('.')) return resolved;
    }
  }
  if (field.labelSource) {
    const value = readPath(device, field.labelSource);
    if (value != null && value !== '') return resolveRuntimeText(String(value), pluginId);
  }
  return undefined;
}

/// 将详情页中协议派生的 labelSource 值重新接回声明式字段选项。
/// 详情页仍保留原始数值；只有插件明确声明了展示名称来源的字段才会本地化。
export function resolveDetailValueLabel(group: string, key: string, device: DeviceState): string | undefined {
  const source = `capabilities.${group}.${key}`;
  for (const capability of device.pluginCapabilities) {
    const fields = [
      ...(capability.metadata.fields ?? []),
      ...(capability.metadata.zones ?? []).flatMap((zone) => zone.fields),
    ];
    const field = fields.find((candidate) => candidate.labelSource === source);
    if (field) return resolveFieldValueLabel(field, device, device.pluginId);
  }
  if (key.endsWith('Name') || key.endsWith('Label')) {
    const value = readPath(device, source);
    if (value != null && value !== '') return resolveRuntimeText(String(value), device.pluginId);
  }
  return undefined;
}

/// 选项解析：合并 field.options 和 field.optionSource。
/// 有 optionSource 时用 readPath 读取运行时选项数组，与 field.options 合并
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

/**
 * 返回状态栏当前应操作的字段。
 *
 * 某些声明会按连接方式提供同一设置的多个字段，例如蓝牙与 2.4G 的休眠
 * 时间。状态栏的首选字段在当前连接不可见时，选择具有相同编辑契约的可见
 * 同级字段；这个选择完全基于声明，不依赖厂商或协议名称。
 */
export function resolveStatusField(
  capability: PluginCapability,
  fieldId: string | undefined,
  device: DeviceState,
): PluginField | undefined {
  if (!fieldId) return undefined;
  const fields = [
    ...(capability.metadata.fields ?? []),
    ...(capability.metadata.zones ?? []).flatMap((zone) => zone.fields),
  ];
  const preferred = fields.find((field) => field.id === fieldId);
  if (!preferred) return undefined;
  if (resolveVisibleWhen(preferred.visibleWhen, device)) return preferred;
  return fields.find((field) => (
    field.id !== preferred.id
    && resolveVisibleWhen(field.visibleWhen, device)
    && field.editor === preferred.editor
    && field.format === preferred.format
    && field.param === preferred.param
  ));
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

/// 解析灯光角色可用性：基于 zones 中 id 为 'mouse'/'receiver' 的区域是否有可写 mutation。
/// 与后端 Capability::lighting_role() 的 zone id 约定一致。
export function resolveLightingRoles(capabilities: PluginCapability[], writableMutations: string[]): { mouse: boolean; receiver: boolean } {
  const roles = { mouse: false, receiver: false };
  for (const capability of capabilities) {
    if (capability.control !== 'LightingZone') continue;
    const zones = capability.metadata.zones;
    if (!zones) continue;
    for (const zone of zones) {
      if (zone.id !== 'mouse' && zone.id !== 'receiver') continue;
      const hasWritable = zone.fields.some((field) => resolveMutation(field.mutation, writableMutations) !== undefined);
      if (hasWritable) roles[zone.id] = true;
    }
  }
  return roles;
}

/// 演示模式 mutation 模拟器。深拷贝 device，遍历 pluginCapabilities 找到匹配 mutation 的可写字段，
/// 通过 field.source 写入新值，并利用 stateMapping 同步 state.* 与 capabilities.* 两侧镜像字段。
/// stageLayout（DPI 分档）单独处理 active/value 的语义性写入。未知 mutation 静默返回原状态。
export function simulateDemoMutation(
  device: DeviceState,
  mutation: string,
  params: Record<string, unknown>,
): DeviceState {
  const next: DeviceState = structuredClone(device);
  const stateMapping = resolveStateMapping(next.pluginCapabilities);

  // 反向索引：snapshot path → state field 名
  const snapshotToStateField: Record<string, string> = {};
  for (const [field, source] of Object.entries(stateMapping)) {
    snapshotToStateField[source] = field;
  }

  /// 同时写入 state.* 与对应 snapshot 路径，保持两端一致。
  const writeSynced = (path: string, value: unknown) => {
    writePath(next, path, value);
    if (path.startsWith('state.')) {
      const field = path.slice('state.'.length);
      const snapshotPath = stateMapping[field];
      if (snapshotPath) writePath(next, snapshotPath, value);
    } else {
      // path 是 snapshot 路径，同步写入对应的 state 字段。
      const stateField = snapshotToStateField[path];
      if (stateField) next.state[stateField] = value;
    }
  };

  for (const capability of next.pluginCapabilities) {
    // 1) stageLayout（DPI 分档）的特殊语义写入
    const layout = capability.metadata.stageLayout;
    if (layout) {
      const selectMutation = resolveMutation(layout.selectMutation, next.writableMutations);
      const setMutation = resolveMutation(layout.setMutation, next.writableMutations);

      if (selectMutation === mutation) {
        const selectParam = layout.selectParam ?? 'value';
        const stageNumber = Number(params[selectParam]);
        const stages = readPath(next, layout.dotsSource) as DpiStage[] | undefined;
        if (stages && Number.isInteger(stageNumber) && stageNumber >= 1 && stageNumber <= stages.length) {
          stages.forEach((stage, i) => { stage.active = (i + 1) === stageNumber; });
        }
        continue;
      }

      if (setMutation === mutation) {
        const stageParam = layout.stageParam ?? 'stage';
        const valueParam = layout.valueParam ?? 'value';
        const stageNumber = Number(params[stageParam]);
        const newValue = Number(params[valueParam]);
        const stages = readPath(next, layout.valueSource) as DpiStage[] | undefined;
        if (stages && Number.isInteger(stageNumber) && stageNumber >= 1 && stageNumber <= stages.length) {
          stages[stageNumber - 1].value = newValue;
        }
        continue;
      }
    }

    // 2) 常规字段：从 zones[].fields 或 metadata.fields 收集
    const fields: PluginField[] = [];
    if (capability.control === 'LightingZone') {
      for (const zone of (capability.metadata.zones ?? [])) {
        fields.push(...zone.fields);
      }
    } else {
      fields.push(...(capability.metadata.fields ?? []));
    }

    for (const field of fields) {
      const fieldMutation = resolveMutation(field.mutation, next.writableMutations);
      if (fieldMutation !== mutation) continue;
      const paramKey = field.param ?? 'value';
      const paramValue = params[paramKey];
      if (paramValue === undefined) continue;
      if (!field.source) continue;
      writeSynced(field.source, paramValue);
    }
  }

  return next;
}

export type { DeviceState };
