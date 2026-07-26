// SPDX-License-Identifier: AGPL-3.0-or-later
// 插件适配层：声明式 capability metadata 解析纯函数。所有插件知识均从 metadata 声明字段读取。
import type { DeviceState, DpiStage, PluginCapability, PluginCapabilityPlacement, PluginField, PluginFieldOption, PluginMutation, PluginStageLayout, PluginStateMapping, PluginStatusDisplay, PluginSummaryItem, PluginSwitch, PluginVisibleWhen, PluginZone, RangeSpec } from './types';
import { resolveLabelKey, resolveRuntimeText } from './i18n';

export const MAX_CONTROL_GROUPS = 6;
export const MAX_STATUS_ITEMS = 6;
export const MAX_CONTROL_OPTIONS = 8;

// ─── ITERATION-004 §2.1：Dashboard Priority 全局选择器 ─────────────────────
// 替代旧的 sort(order) + slice(MAX) 行为，使用 priority/fixedSlot/fourthSlotEligible/
// dedupeKey/fallbackRegion 实现统一槽位选择。详见 DASHBOARD_PRIORITY_ALL_PLUGINS.md。

/** 上方控制区：preferred=3, max=4, 第 4 项需 priority>=90 且 fourthSlotEligible。 */
export const CONTROL_PREFERRED_COUNT = 3;
export const CONTROL_MAX_COUNT = 4;
/** 下方状态区：preferred=3, max=4, 第 4 项需 priority>=90 且 fourthSlotEligible。 */
export const STATUS_PREFERRED_COUNT = 3;
export const STATUS_MAX_COUNT = 4;
/** 第 4 槽位最低优先级阈值。 */
export const FOURTH_SLOT_MIN_PRIORITY = 90;
/** 下方基础槽位最低优先级阈值。 */
export const STATUS_BASE_SLOT_MIN_PRIORITY = 60;

/** Dashboard placement region。 */
export type PluginRegion = 'hero' | 'control' | 'status' | 'details';

/** 返回 capability 在指定 region 的所有 placement。 */
export function placementsFor(capability: PluginCapability, region: PluginRegion): NonNullable<PluginCapability['placements']> {
  return (capability.placements ?? []).filter((p) => p.region === region);
}

/** Dashboard 选择上下文：包含去重键已用集合（跨区域共享）。 */
export interface DashboardSelectionContext {
  /** 已使用的 dedupeKey 集合（跨 control/status 共享，防止重复入口）。 */
  usedDedupeKeys: Set<string>;
}

/** P0-A：纯函数选择结果。usedDedupeKeys 是新 Set，包含输入 keys + 本次选中的 keys。 */
export interface DashboardSelectionResult<T> {
  selected: T[];
  fallback: T[];
  usedDedupeKeys: Set<string>;
}

/** 控制区 placement 候选项：capability + placement + 解析后的优先级。 */
export interface ControlCandidate {
  capability: PluginCapability;
  placement: PluginCapabilityPlacement;
  /** 解析后的 priority（默认 0）。 */
  priority: number;
  /** 解析后的 fixedSlot（1/2/3 或 undefined）。 */
  fixedSlot: 1 | 2 | 3 | undefined;
  /** 解析后的 fourthSlotEligible（默认 false）。 */
  fourthSlotEligible: boolean;
  /** 解析后的 dedupeKey。 */
  dedupeKey: string | undefined;
  /** 槽位组 ID（placement.group || capability.id）。 */
  groupId: string;
  /** P0-E：候选槽位位置。leading=核心序列之前；trailing=核心序列之后。默认 trailing。 */
  optionalPosition: 'leading' | 'trailing';
}

/**
 * ITERATION-005 §P0-A/P0-B：Dashboard 上方控制区统一选择器（纯函数）。
 *
 * 替代旧的 `sort(order).slice(0, MAX_CONTROL_GROUPS)`，按以下规则选择：
 * 1. 过滤 availability / visibleWhen / content；
 * 2. 按 dedupeKey 去重（跨区域共享 usedDedupeKeys）；
 * 3. 放置 fixedSlot 1/2/3（DPI/回报率/灯光，按固定顺序）；
 * 4. 从剩余候选中选择第 4 项（需 priority>=90 且 fourthSlotEligible）；
 * 5. 未选中项按 fallbackRegion 回退（调用方可用于高级设置页）。
 *
 * P0-A：输入 ReadonlySet<string>，绝不 mutate 调用者 Set；返回新的 Set。
 * P0-B：删除空缺 fixedSlot 回退填充；fixedSlot 1/2/3 仅接受 fixedSlot===1/2/3
 *       的候选；某 fixedSlot 无候选时该位置为空，普通 candidate 绝不填入 fixedSlot。
 * 没有合格第 4 项时只显示 3 项，不显示空占位，不为凑满 4 格降低阈值。
 */
export function selectDashboardControls(
  capabilities: PluginCapability[],
  device: DeviceState,
  availabilityFilter: (capability: PluginCapability) => boolean,
  contentFilter: (capability: PluginCapability) => boolean,
  usedDedupeKeys: ReadonlySet<string>,
): DashboardSelectionResult<ControlCandidate> {
  // P0-A：本地副本，绝不 mutate 调用者 Set。
  const usedKeys = new Set(usedDedupeKeys);

  // 收集所有 control placement 候选项。
  const candidates: ControlCandidate[] = [];
  for (const capability of capabilities) {
    if (!availabilityFilter(capability)) continue;
    if (!resolveVisibleWhen(capability.metadata.visibleWhen, device)) continue;
    if (!contentFilter(capability)) continue;
    for (const placement of placementsFor(capability, 'control')) {
      const candidate: ControlCandidate = {
        capability,
        placement,
        priority: placement.priority ?? 0,
        fixedSlot: placement.fixedSlot,
        fourthSlotEligible: placement.fourthSlotEligible ?? false,
        dedupeKey: placement.dedupeKey,
        groupId: placement.group || capability.id,
        optionalPosition: placement.optionalPosition ?? 'trailing',
      };
      candidates.push(candidate);
    }
  }

  // 按 dedupeKey 去重（同 dedupeKey 只保留 priority 最高的）。
  const byDedupeKey = new Map<string, ControlCandidate>();
  const noDedupeKey: ControlCandidate[] = [];
  for (const candidate of candidates) {
    if (!candidate.dedupeKey) {
      noDedupeKey.push(candidate);
      continue;
    }
    if (usedKeys.has(candidate.dedupeKey)) {
      continue; // 已被其他区域使用（如系统全部读数入口）。
    }
    const existing = byDedupeKey.get(candidate.dedupeKey);
    if (!existing || candidate.priority > existing.priority) {
      byDedupeKey.set(candidate.dedupeKey, candidate);
    }
  }
  const deduped = [...byDedupeKey.values(), ...noDedupeKey];

  // 放置 fixedSlot 1/2/3。P0-B：仅接受 fixedSlot===1/2/3 的候选，不回退填充。
  const fixedSlots: (ControlCandidate | undefined)[] = [undefined, undefined, undefined];
  const remaining: ControlCandidate[] = [];
  for (const candidate of deduped) {
    if (
      candidate.fixedSlot === 1
      || candidate.fixedSlot === 2
      || candidate.fixedSlot === 3
    ) {
      const idx = candidate.fixedSlot - 1;
      if (!fixedSlots[idx]) {
        fixedSlots[idx] = candidate;
        if (candidate.dedupeKey) usedKeys.add(candidate.dedupeKey);
        continue;
      }
    }
    remaining.push(candidate);
  }

  // 排序：priority desc → order asc → groupId asc（stable）。
  remaining.sort((a, b) => {
    if (b.priority !== a.priority) return b.priority - a.priority;
    if (a.placement.order !== b.placement.order) return a.placement.order - b.placement.order;
    return a.groupId < b.groupId ? -1 : a.groupId > b.groupId ? 1 : 0;
  });

  // P0-B：已删除"空缺 fixedSlot 回退填充"逻辑。
  // fixedSlot 1/2/3 仅接受 fixedSlot===1/2/3 的候选；若某 fixedSlot 无候选，
  // 该位置为空（不显示）。普通 candidate 绝不填入 fixedSlot。

  // P0-E：核心序列 DPI→回报率→灯光（fixedSlot 1/2/3），相对顺序不可被打断。
  // 候选只能放在核心序列的 leading（之前）或 trailing（之后），禁止插入核心中间。
  // 最多一个候选，通过 optionalPosition 控制位置（默认 trailing）。
  const coreSequence: ControlCandidate[] = [];
  for (const candidate of fixedSlots) {
    if (candidate) coreSequence.push(candidate);
  }

  // 第 4 槽位（候选）：只有 priority>=90 且 fourthSlotEligible 的候选才竞争。
  // 候选选择：priority desc → order asc → stable id asc（remaining 已按此排序）。
  // 核心三项必须全部就位才允许候选竞争第 4 槽（保持核心序列完整性）。
  let leadingCandidate: ControlCandidate | undefined;
  let trailingCandidate: ControlCandidate | undefined;
  if (coreSequence.length >= CONTROL_PREFERRED_COUNT && coreSequence.length < CONTROL_MAX_COUNT) {
    const fourthCandidate = remaining.find(
      (c) => c.priority >= FOURTH_SLOT_MIN_PRIORITY && c.fourthSlotEligible,
    );
    if (fourthCandidate) {
      if (fourthCandidate.optionalPosition === 'leading') {
        leadingCandidate = fourthCandidate;
      } else {
        trailingCandidate = fourthCandidate;
      }
      if (fourthCandidate.dedupeKey) usedKeys.add(fourthCandidate.dedupeKey);
      remaining.splice(remaining.indexOf(fourthCandidate), 1);
    }
  }

  // 组装最终序列：leading → 核心序列 → trailing。
  // 核心缺失时仍保持剩余核心相对顺序。
  const selected: ControlCandidate[] = [];
  if (leadingCandidate) selected.push(leadingCandidate);
  for (const candidate of coreSequence) selected.push(candidate);
  if (trailingCandidate) selected.push(trailingCandidate);

  // 未选中项作为 fallback 返回（调用方可用于高级设置页）。
  const fallback = remaining.filter((c) => {
    if (c.fixedSlot) return false; // 已在 fixedSlot 但未入选的跳过（不应发生）。
    const region = c.placement.fallbackRegion ?? 'advanced';
    return region !== 'hidden';
  });

  return { selected, fallback, usedDedupeKeys: usedKeys };
}

/**
 * ITERATION-005 §P0-A/P0-B：Dashboard 下方状态区统一选择器（纯函数）。
 *
 * 替代旧的 `sort(order).slice(0, MAX_STATUS_ITEMS)`，按以下规则选择：
 * 1. 过滤 availability / visibleWhen / reported value；
 * 2. 与系统入口、上方入口、全部读数、电量、连接状态去重（共享 usedDedupeKeys）；
 * 3. 按 priority desc、order asc、stable id asc 排序；
 * 4. 选择最多 3 个基础项（priority >= STATUS_BASE_SLOT_MIN_PRIORITY）；
 * 5. 第 4 项单独应用 priority>=90 与 fourthSlotEligible。
 *
 * P0-A：输入 ReadonlySet<string>，绝不 mutate 调用者 Set；返回新的 Set。
 * P0-B：删除"高优先级候选不足时从 deferred 补齐至 PREFERRED_COUNT"逻辑；
 *       只选 priority >= STATUS_BASE_SLOT_MIN_PRIORITY 的候选；不足 3 项就显示少于 3 项。
 * 不显示空占位，不为了布局完整展示低价值项目。
 */
export function selectDashboardStatus(
  capabilities: PluginCapability[],
  device: DeviceState,
  availabilityFilter: (capability: PluginCapability) => boolean,
  hasReportedValue: (capability: PluginCapability) => boolean,
  usedDedupeKeys: ReadonlySet<string>,
): DashboardSelectionResult<ControlCandidate> {
  // P0-A：本地副本，绝不 mutate 调用者 Set。
  const usedKeys = new Set(usedDedupeKeys);

  const candidates: ControlCandidate[] = [];
  for (const capability of capabilities) {
    if (!availabilityFilter(capability)) continue;
    if (!resolveVisibleWhen(capability.metadata.visibleWhen, device)) continue;
    if (!hasReportedValue(capability)) continue;
    for (const placement of placementsFor(capability, 'status')) {
      const candidate: ControlCandidate = {
        capability,
        placement,
        priority: placement.priority ?? 0,
        fixedSlot: placement.fixedSlot,
        fourthSlotEligible: placement.fourthSlotEligible ?? false,
        dedupeKey: placement.dedupeKey,
        groupId: placement.group || capability.id,
        optionalPosition: placement.optionalPosition ?? 'trailing',
      };
      candidates.push(candidate);
    }
  }

  // 按 dedupeKey 去重。
  const byDedupeKey = new Map<string, ControlCandidate>();
  const noDedupeKey: ControlCandidate[] = [];
  for (const candidate of candidates) {
    if (!candidate.dedupeKey) {
      noDedupeKey.push(candidate);
      continue;
    }
    if (usedKeys.has(candidate.dedupeKey)) {
      continue;
    }
    const existing = byDedupeKey.get(candidate.dedupeKey);
    if (!existing || candidate.priority > existing.priority) {
      byDedupeKey.set(candidate.dedupeKey, candidate);
    }
  }
  const deduped = [...byDedupeKey.values(), ...noDedupeKey];

  // 排序：priority desc → order asc → groupId asc（stable）。
  deduped.sort((a, b) => {
    if (b.priority !== a.priority) return b.priority - a.priority;
    if (a.placement.order !== b.placement.order) return a.placement.order - b.placement.order;
    return a.groupId < b.groupId ? -1 : a.groupId > b.groupId ? 1 : 0;
  });

  const selected: ControlCandidate[] = [];
  const deferred: ControlCandidate[] = [];

  // P0-B：基础 3 槽位只选 priority >= STATUS_BASE_SLOT_MIN_PRIORITY 的候选。
  // 已删除"高优先级候选不足时从 deferred 补齐至 PREFERRED_COUNT"逻辑。
  // 不足 3 项就显示少于 3 项，不显示空占位。
  for (const candidate of deduped) {
    if (selected.length >= STATUS_PREFERRED_COUNT) {
      deferred.push(candidate);
      continue;
    }
    if (candidate.priority >= STATUS_BASE_SLOT_MIN_PRIORITY) {
      selected.push(candidate);
      if (candidate.dedupeKey) usedKeys.add(candidate.dedupeKey);
    } else {
      deferred.push(candidate);
    }
  }

  // 第 4 槽位：priority>=90 且 fourthSlotEligible。
  if (selected.length >= STATUS_PREFERRED_COUNT && selected.length < STATUS_MAX_COUNT) {
    const fourthCandidate = deferred.find(
      (c) => c.priority >= FOURTH_SLOT_MIN_PRIORITY && c.fourthSlotEligible,
    );
    if (fourthCandidate) {
      selected.push(fourthCandidate);
      if (fourthCandidate.dedupeKey) usedKeys.add(fourthCandidate.dedupeKey);
    }
  }

  // 把 deferred 中未入选的低优先级项也加入 fallback（调用方可决定是否展示）。
  const fallback: ControlCandidate[] = [];
  for (const candidate of deferred) {
    if (selected.includes(candidate)) continue;
    const region = candidate.placement.fallbackRegion ?? 'advanced';
    if (region !== 'hidden') fallback.push(candidate);
  }

  return { selected, fallback, usedDedupeKeys: usedKeys };
}

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
/// 原型污染防护：__proto__/constructor/prototype 这三个键会污染原型链，
/// 必须在中间遍历和最终赋值处都拒绝。CodeQL 的 js/prototype-pollution-utility
/// 查询需要直接的字符串字面量比较才能将守卫识别为 sanitizer，因此这里使用
/// 内联比较而非 Set.has() 辅助函数。
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
    // 原型污染防护：拒绝 __proto__/constructor/prototype 作为中间路径段。
    if (part === '__proto__' || part === 'constructor' || part === 'prototype') return;
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
  // 原型污染防护：最终赋值前再次拒绝 __proto__/constructor/prototype。
  if (lastPart === '__proto__' || lastPart === 'constructor' || lastPart === 'prototype') return;
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

/// 对 {path, eq?, ne?, in?} 条件求值。
/// 无 condition 时返回 true；有 eq 时返回 value === eq；有 ne 时返回 value !== ne；
/// 有 in 时返回 value 是否在数组中；都没有时返回 value != null。
export function resolveVisibleWhen(condition: PluginVisibleWhen | undefined, device: DeviceState): boolean {
  if (!condition) return true;
  const value = readPath(device, condition.path);
  if (condition.eq !== undefined) return value === condition.eq;
  if (condition.ne !== undefined) return value !== condition.ne;
  if (condition.in !== undefined) return Array.isArray(condition.in) && condition.in.indexOf(value) !== -1;
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

/// 读 field.range。当 field.rangeSource 存在时，从设备快照读取动态 max 值
/// （可选 rangeMaxOffset 偏移），覆盖静态 range.max。min 和 step 仍取自静态 range。
export function resolveFieldRange(field: PluginField, device?: DeviceState): RangeSpec | undefined {
  const base = field.range;
  if (!field.rangeSource || !device) return base;
  const raw = readPath(device, field.rangeSource);
  if (typeof raw !== 'number' || !Number.isFinite(raw)) return base;
  const offset = field.rangeMaxOffset ?? 0;
  const dynamicMax = raw + offset;
  if (!base) return { min: 0, max: dynamicMax, step: 1 };
  return { min: base.min, max: dynamicMax, step: base.step };
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
 * P0-E：解析状态栏显示变体。
 *
 * 若 display.variants 存在，返回第一个 visibleWhen 匹配的 variant；
 * 否则返回 display 本身（向后兼容）。
 *
 * 用于 AM35 Sleep family-aware status：同一 capability 在不同 family/connection
 * 下指向不同的 valueSource/onClickField，避免在宿主按 pluginId 硬编码协议分支。
 */
export function resolveStatusDisplayVariant(
  display: PluginStatusDisplay,
  device: DeviceState,
): PluginStatusDisplay {
  if (!display.variants || display.variants.length === 0) return display;
  for (const variant of display.variants) {
    if (resolveVisibleWhen(variant.visibleWhen, device)) return variant;
  }
  // 无匹配 variant 时回退到 display 本身（可能 valueSource 缺失，调用方需处理）。
  return display;
}

/**
 * P1-B：Placement Contract Validator。
 *
 * 返回错误消息或 null（合法）。检查 dashboard placement 的必填字段和非法组合。
 * 运行时兼容旧数据：undefined 的 priority/dashboardRole/fallbackRegion 不算非法
 * （由 selector 用默认值兜底）；此函数只检查"声明了但非法"的组合。
 */
export function validatePlacement(placement: PluginCapabilityPlacement): string | null {
  // fixedSlot 只能是 1/2/3（类型已约束，但运行时旧数据可能越界）。
  if (placement.fixedSlot !== undefined && placement.fixedSlot !== 1 && placement.fixedSlot !== 2 && placement.fixedSlot !== 3) {
    return `fixedSlot must be 1, 2, or 3, got ${String(placement.fixedSlot)}`;
  }
  // fixedSlot 仅对 fixed-core 角色合法。
  const role = placement.dashboardRole ?? 'candidate';
  if (placement.fixedSlot !== undefined && role !== 'fixed-core') {
    return `fixedSlot=${placement.fixedSlot} requires dashboardRole='fixed-core', got '${role}'`;
  }
  // priority 范围检查（0..100）。
  const priority = placement.priority ?? 0;
  if (typeof priority === 'number' && (priority < 0 || priority > 100)) {
    return `priority must be in [0, 100], got ${priority}`;
  }
  // fourthSlotEligible 为 true 时 priority 必须 >= 90。
  if (placement.fourthSlotEligible && priority < FOURTH_SLOT_MIN_PRIORITY) {
    return `fourthSlotEligible=true requires priority>=${FOURTH_SLOT_MIN_PRIORITY}, got priority=${priority}`;
  }
  // status placement 不应有 fixedSlot。
  if (placement.region === 'status' && placement.fixedSlot !== undefined) {
    return `status placement must not declare fixedSlot, got ${placement.fixedSlot}`;
  }
  // hero/details placement 不应有 dashboardRole=fixed-core。
  if ((placement.region === 'hero' || placement.region === 'details') && role === 'fixed-core') {
    return `${placement.region} placement must not use dashboardRole='fixed-core'`;
  }
  return null;
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

// ─── ITERATION-006 §P0-F/P0-G：子块选择器 ──────────────────────────────────
// 通用子块选择纯函数：按 priority desc → order asc → stable id asc 排序，
// 取前 max 项作为选中项，其余作为 fallback。
// 适用于回报率页面（max=3）和灯光页面（max=6，固定两端）。

/** 回报率页面子块上限。 */
export const POLLING_MAX_SUBBLOCKS = 3;
/** 灯光页面子块上限。 */
export const LIGHTING_MAX_SUBBLOCKS = 6;
/** 灯光页面中间候选上限（effect 和 primary-color 之外）。 */
export const LIGHTING_MAX_CANDIDATES = 4;

/** 子块选择结果：selected 为入选项，fallback 为进入 Advanced Settings 的项。 */
export interface SubblockSelection<T> {
  selected: T[];
  fallback: T[];
}

/**
 * P0-F：通用子块选择器（纯函数）。
 *
 * 按 priority desc → order asc → stable id asc 排序，取前 max 项。
 * 不足 max 项时保持实际数量，不补空、不拿低优先级项凑数。
 * 超过 max 项时，前 max 项入选，其余进入 fallback（调用方可用于 Advanced Settings）。
 *
 * 适用于回报率页面（max=3）的 summary items 选择。
 */
export function selectSummarySubblocks(
  items: PluginSummaryItem[],
  max: number,
): SubblockSelection<PluginSummaryItem> {
  const candidates = items.map((item, index) => ({
    item,
    priority: item.priority ?? 0,
    order: index,
    stableId: item.source,
  }));

  candidates.sort((a, b) => {
    if (b.priority !== a.priority) return b.priority - a.priority;
    if (a.order !== b.order) return a.order - b.order;
    return a.stableId < b.stableId ? -1 : a.stableId > b.stableId ? 1 : 0;
  });

  const selected = candidates.slice(0, max).map((c) => c.item);
  const fallback = candidates.slice(max).map((c) => c.item);
  return { selected, fallback };
}

/**
 * P0-G：灯光子块选择器（纯函数）。
 *
 * 固定两端：lightingRole='effect' 最左，lightingRole='primary-color' 最右。
 * 中间最多 4 个 candidate（按 priority desc → id asc 排序）。
 * 总数最多 6（1 effect + 4 candidate + 1 primary-color）。
 *
 * - 多个 effect 只取最高优先级；
 * - 多个 primary-color 只取最高优先级；
 * - 未声明 lightingRole 的字段视为 candidate（向后兼容）；
 * - 次级颜色、比例、raw 字段应声明 presentation='details' 或由 fallback 接收。
 *
 * 返回的 selected 已按 [effect, ...candidates, primary-color] 顺序排列。
 * fallback 包含未入选的 effect/primary-color 候选和超出 4 个的 candidate。
 */
export function selectLightingSubblocks(
  fields: PluginField[],
): SubblockSelection<PluginField> {
  const effects: PluginField[] = [];
  const primaryColors: PluginField[] = [];
  const candidates: PluginField[] = [];

  for (const field of fields) {
    const role = field.lightingRole ?? 'candidate';
    if (role === 'effect') effects.push(field);
    else if (role === 'primary-color') primaryColors.push(field);
    else candidates.push(field);
  }

  const sortByPriority = (a: PluginField, b: PluginField): number => {
    const pa = a.priority ?? 0;
    const pb = b.priority ?? 0;
    if (pb !== pa) return pb - pa;
    return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
  };

  effects.sort(sortByPriority);
  primaryColors.sort(sortByPriority);
  candidates.sort(sortByPriority);

  const selectedEffect = effects[0];
  const selectedPrimaryColor = primaryColors[0];
  const selectedCandidates = candidates.slice(0, LIGHTING_MAX_CANDIDATES);

  const selected: PluginField[] = [];
  if (selectedEffect) selected.push(selectedEffect);
  selected.push(...selectedCandidates);
  if (selectedPrimaryColor) selected.push(selectedPrimaryColor);

  const selectedSet = new Set(selected);
  const fallback: PluginField[] = [];
  for (const field of fields) {
    if (selectedSet.has(field)) continue;
    fallback.push(field);
  }

  return { selected, fallback };
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
