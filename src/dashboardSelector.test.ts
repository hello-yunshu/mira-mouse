// SPDX-License-Identifier: AGPL-3.0-or-later
// ITERATION-005 §P0-A/P0-B：Dashboard 选择器纯函数 + 覆盖率测试。
// ITERATION-007 §P1-A：子块选择器 + validatePlacement + leading/trailing 覆盖率测试。
import { describe, expect, it } from 'vitest';
import {
  CONTROL_PREFERRED_COUNT,
  CONTROL_MAX_COUNT,
  FOURTH_SLOT_MIN_PRIORITY,
  STATUS_BASE_SLOT_MIN_PRIORITY,
  STATUS_PREFERRED_COUNT,
  STATUS_MAX_COUNT,
  POLLING_MAX_SUBBLOCKS,
  LIGHTING_MAX_SUBBLOCKS,
  LIGHTING_MAX_CANDIDATES,
  selectDashboardControls,
  selectDashboardStatus,
  selectSummarySubblocks,
  selectLightingSubblocks,
  summaryMaxForCapability,
  validatePlacement,
} from './pluginAdapter';
import type { DeviceState, PluginCapability, PluginCapabilityPlacement, PluginField, PluginSummaryItem } from './types';

function makeDevice(state: Record<string, unknown> = {}, overrides: Partial<DeviceState> = {}): DeviceState {
  return {
    name: 'test',
    connection: 'usb',
    batteries: [],
    state,
    capabilities: {},
    pluginCapabilities: [],
    writableMutations: [],
    evidence: 'unknown',
    readonly: false,
    updatedAt: '00:00',
    ...overrides,
  };
}

function makeCapability(
  id: string,
  placements: PluginCapabilityPlacement[],
  overrides: Partial<PluginCapability> = {},
): PluginCapability {
  return {
    id,
    control: 'Select',
    labelKey: `capability.${id}`,
    readOnly: false,
    placements,
    metadata: {
      fields: [{ id: 'value', source: `state.${id}`, editor: 'modal-select' }],
    },
    ...overrides,
  };
}

function controlPlacement(overrides: Partial<PluginCapabilityPlacement>): PluginCapabilityPlacement {
  return {
    region: 'control',
    order: 100,
    span: 1,
    priority: 50,
    dashboardRole: 'candidate',
    fallbackRegion: 'advanced',
    ...overrides,
  };
}

function statusPlacement(overrides: Partial<PluginCapabilityPlacement>): PluginCapabilityPlacement {
  return {
    region: 'status',
    order: 100,
    span: 1,
    priority: 50,
    dashboardRole: 'candidate',
    fallbackRegion: 'advanced',
    ...overrides,
  };
}

// eslint-disable-next-line @typescript-eslint/no-unused-vars
const allAvailable = (_capability: PluginCapability) => true;
// eslint-disable-next-line @typescript-eslint/no-unused-vars
const allHaveContent = (_capability: PluginCapability) => true;
// eslint-disable-next-line @typescript-eslint/no-unused-vars
const allHaveReported = (_capability: PluginCapability) => true;

// ─── P0-A：纯函数 / 输入 Set 不可变 / 重渲染状态持久 ─────────────────────

describe('P0-A: selectDashboardControls is a pure function', () => {
  it('does not mutate the input usedDedupeKeys Set', () => {
    const dpi = makeCapability('dpi', [controlPlacement({
      group: 'performance', order: 10, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 1, dedupeKey: 'dashboard.dpi',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [dpi] });
    const inputKeys = new Set<string>(['system.all-readings']);

    selectDashboardControls([dpi], device, allAvailable, allHaveContent, inputKeys);

    // 输入 Set 内容必须保持原样：调用者仍可安全复用。
    expect(inputKeys).toEqual(new Set(['system.all-readings']));
    expect(inputKeys.has('dashboard.dpi')).toBe(false);
  });

  it('returns a new Set instance (not the same reference as input)', () => {
    const dpi = makeCapability('dpi', [controlPlacement({
      group: 'performance', order: 10, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 1, dedupeKey: 'dashboard.dpi',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [dpi] });
    const inputKeys = new Set<string>(['system.all-readings']);

    const { usedDedupeKeys } = selectDashboardControls(
      [dpi], device, allAvailable, allHaveContent, inputKeys,
    );

    expect(usedDedupeKeys).not.toBe(inputKeys);
    expect(usedDedupeKeys.has('system.all-readings')).toBe(true);
    expect(usedDedupeKeys.has('dashboard.dpi')).toBe(true);
  });

  it('produces identical output across repeated calls with the same input', () => {
    const dpi = makeCapability('dpi', [controlPlacement({
      group: 'performance', order: 10, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 1, dedupeKey: 'dashboard.dpi',
    })]);
    const polling = makeCapability('polling', [controlPlacement({
      group: 'polling', order: 20, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 2, dedupeKey: 'dashboard.polling',
    })]);
    const lighting = makeCapability('lighting', [controlPlacement({
      group: 'lighting', order: 30, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 3, dedupeKey: 'dashboard.lighting',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [dpi, polling, lighting] });
    const inputKeys = new Set<string>();

    const first = selectDashboardControls([dpi, polling, lighting], device, allAvailable, allHaveContent, inputKeys);
    const second = selectDashboardControls([dpi, polling, lighting], device, allAvailable, allHaveContent, inputKeys);

    expect(second.selected.map((c) => c.capability.id)).toEqual(first.selected.map((c) => c.capability.id));
    expect(second.usedDedupeKeys).toEqual(first.usedDedupeKeys);
    // 输入 Set 在两次调用后仍保持空。
    expect(inputKeys).toEqual(new Set());
  });

  it('does not leak selected keys into subsequent calls through the input Set', () => {
    const dpi = makeCapability('dpi', [controlPlacement({
      group: 'performance', order: 10, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 1, dedupeKey: 'dashboard.dpi',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [dpi] });
    const sharedInput = new Set<string>();

    const first = selectDashboardControls([dpi], device, allAvailable, allHaveContent, sharedInput);
    expect(first.usedDedupeKeys.has('dashboard.dpi')).toBe(true);

    // 第二次调用使用同一 sharedInput，应仍然能选中 dpi（因为 sharedInput 未被 mutate）。
    const second = selectDashboardControls([dpi], device, allAvailable, allHaveContent, sharedInput);
    expect(second.selected).toHaveLength(1);
    expect(second.selected[0].capability.id).toBe('dpi');
  });
});

describe('P0-A: selectDashboardStatus is a pure function', () => {
  it('does not mutate the input usedDedupeKeys Set', () => {
    const sleep = makeCapability('sleep', [statusPlacement({
      order: 10, priority: 80, dedupeKey: 'status.sleep',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [sleep] });
    const inputKeys = new Set<string>(['dashboard.dpi']);

    selectDashboardStatus([sleep], device, allAvailable, allHaveReported, inputKeys);

    expect(inputKeys).toEqual(new Set(['dashboard.dpi']));
    expect(inputKeys.has('status.sleep')).toBe(false);
  });

  it('returns a new Set instance containing input keys plus selected keys', () => {
    const sleep = makeCapability('sleep', [statusPlacement({
      order: 10, priority: 80, dedupeKey: 'status.sleep',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [sleep] });
    const inputKeys = new Set<string>(['dashboard.dpi']);

    const { usedDedupeKeys } = selectDashboardStatus(
      [sleep], device, allAvailable, allHaveReported, inputKeys,
    );

    expect(usedDedupeKeys).not.toBe(inputKeys);
    expect(usedDedupeKeys.has('dashboard.dpi')).toBe(true);
    expect(usedDedupeKeys.has('status.sleep')).toBe(true);
  });

  it('produces identical output across repeated calls with the same input', () => {
    const sleep = makeCapability('sleep', [statusPlacement({
      order: 10, priority: 80, dedupeKey: 'status.sleep',
    })]);
    const profile = makeCapability('profile', [statusPlacement({
      order: 20, priority: 70, dedupeKey: 'status.profile',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [sleep, profile] });
    const inputKeys = new Set<string>();

    const first = selectDashboardStatus([sleep, profile], device, allAvailable, allHaveReported, inputKeys);
    const second = selectDashboardStatus([sleep, profile], device, allAvailable, allHaveReported, inputKeys);

    expect(second.selected.map((c) => c.capability.id)).toEqual(first.selected.map((c) => c.capability.id));
    expect(second.usedDedupeKeys).toEqual(first.usedDedupeKeys);
    expect(inputKeys).toEqual(new Set());
  });
});

// ─── P0-B：selectDashboardControls 覆盖率 ───────────────────────────────

describe('P0-B: selectDashboardControls fixedSlot placement', () => {
  it('places fixedSlot 1/2/3 candidates in order', () => {
    const dpi = makeCapability('dpi', [controlPlacement({
      group: 'performance', order: 10, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 1, dedupeKey: 'dashboard.dpi',
    })]);
    const polling = makeCapability('polling', [controlPlacement({
      group: 'polling', order: 20, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 2, dedupeKey: 'dashboard.polling',
    })]);
    const lighting = makeCapability('lighting', [controlPlacement({
      group: 'lighting', order: 30, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 3, dedupeKey: 'dashboard.lighting',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [lighting, polling, dpi] });

    const { selected } = selectDashboardControls(
      [lighting, polling, dpi], device, allAvailable, allHaveContent, new Set(),
    );

    expect(selected.map((c) => c.capability.id)).toEqual(['dpi', 'polling', 'lighting']);
    expect(selected.map((c) => c.fixedSlot)).toEqual([1, 2, 3]);
  });

  it('leaves a fixedSlot empty when no candidate declares it (no forced backfill)', () => {
    // 缺少 fixedSlot=2 的回报率候选。
    const dpi = makeCapability('dpi', [controlPlacement({
      group: 'performance', order: 10, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 1, dedupeKey: 'dashboard.dpi',
    })]);
    const lighting = makeCapability('lighting', [controlPlacement({
      group: 'lighting', order: 30, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 3, dedupeKey: 'dashboard.lighting',
    })]);
    // 一个高优先级普通候选，但 fourthSlotEligible=true。
    const extra = makeCapability('extra', [controlPlacement({
      group: 'extra', order: 5, priority: 95, dashboardRole: 'candidate',
      fourthSlotEligible: true, dedupeKey: 'dashboard.extra',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [dpi, lighting, extra] });

    const { selected, fallback } = selectDashboardControls(
      [dpi, lighting, extra], device, allAvailable, allHaveContent, new Set(),
    );

    // 普通候选绝不填入 fixedSlot=2；fixedSlot=2 位置为空（不强制回填）。
    expect(selected.find((c) => c.fixedSlot === 2)).toBeUndefined();
    // P0-G relaxed gate：核心缺失时仍允许一个候选（trailing 默认），
    // extra 作为 trailing 候选入选，不填 fixedSlot=2。
    // 序列为 DPI → lighting → extra（核心相对顺序保持，候选在末尾）。
    expect(selected.map((c) => c.capability.id)).toEqual(['dpi', 'lighting', 'extra']);
    expect(selected.find((c) => c.capability.id === 'extra')?.fixedSlot).toBeUndefined();
    expect(fallback.map((c) => c.capability.id)).not.toContain('extra');
  });

  it('fills the 4th slot via P0-G relaxed gate even when fewer than 3 fixed slots are filled', () => {
    // P0-G relaxed gate：仅 1 个 fixedSlot 时仍允许一个候选（leading 或 trailing）。
    const dpi = makeCapability('dpi', [controlPlacement({
      group: 'performance', order: 10, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 1, dedupeKey: 'dashboard.dpi',
    })]);
    const extra = makeCapability('extra', [controlPlacement({
      group: 'extra', order: 5, priority: 95, dashboardRole: 'candidate',
      fourthSlotEligible: true, dedupeKey: 'dashboard.extra',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [dpi, extra] });

    const { selected } = selectDashboardControls(
      [dpi, extra], device, allAvailable, allHaveContent, new Set(),
    );

    // 仅 1 个 fixedSlot + 1 个 trailing candidate = 2 项（P0-G relaxed gate 允许）。
    expect(selected.map((c) => c.capability.id)).toEqual(['dpi', 'extra']);
    expect(selected.find((c) => c.capability.id === 'extra')?.fixedSlot).toBeUndefined();
  });
});

describe('P0-B: selectDashboardControls 4th slot eligibility', () => {
  it('fills the 4th slot with a priority>=90 fourthSlotEligible candidate', () => {
    const fixedCaps = ['dpi', 'polling', 'lighting'].map((id, idx) => makeCapability(id, [controlPlacement({
      group: id, order: (idx + 1) * 10, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: (idx + 1) as 1 | 2 | 3, dedupeKey: `dashboard.${id}`,
    })]));
    const extra = makeCapability('extra', [controlPlacement({
      group: 'extra', order: 5, priority: 95, dashboardRole: 'candidate',
      fourthSlotEligible: true, dedupeKey: 'dashboard.extra',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [...fixedCaps, extra] });

    const { selected } = selectDashboardControls(
      [...fixedCaps, extra], device, allAvailable, allHaveContent, new Set(),
    );

    expect(selected).toHaveLength(CONTROL_MAX_COUNT);
    expect(selected[3].capability.id).toBe('extra');
  });

  it('does not fill the 4th slot when no candidate has fourthSlotEligible=true', () => {
    const fixedCaps = ['dpi', 'polling', 'lighting'].map((id, idx) => makeCapability(id, [controlPlacement({
      group: id, order: (idx + 1) * 10, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: (idx + 1) as 1 | 2 | 3, dedupeKey: `dashboard.${id}`,
    })]));
    const extra = makeCapability('extra', [controlPlacement({
      group: 'extra', order: 5, priority: 95, dashboardRole: 'candidate',
      fourthSlotEligible: false, dedupeKey: 'dashboard.extra',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [...fixedCaps, extra] });

    const { selected } = selectDashboardControls(
      [...fixedCaps, extra], device, allAvailable, allHaveContent, new Set(),
    );

    expect(selected).toHaveLength(CONTROL_PREFERRED_COUNT);
    expect(selected.find((c) => c.capability.id === 'extra')).toBeUndefined();
  });

  it('does not fill the 4th slot when candidate priority < 90', () => {
    const fixedCaps = ['dpi', 'polling', 'lighting'].map((id, idx) => makeCapability(id, [controlPlacement({
      group: id, order: (idx + 1) * 10, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: (idx + 1) as 1 | 2 | 3, dedupeKey: `dashboard.${id}`,
    })]));
    const extra = makeCapability('extra', [controlPlacement({
      group: 'extra', order: 5, priority: 85, dashboardRole: 'candidate',
      fourthSlotEligible: true, dedupeKey: 'dashboard.extra',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [...fixedCaps, extra] });

    const { selected } = selectDashboardControls(
      [...fixedCaps, extra], device, allAvailable, allHaveContent, new Set(),
    );

    expect(selected).toHaveLength(CONTROL_PREFERRED_COUNT);
    expect(selected.find((c) => c.capability.id === 'extra')).toBeUndefined();
  });
});

describe('P0-B: selectDashboardControls dedupeKey handling', () => {
  it('keeps only the highest-priority candidate when multiple share a dedupeKey', () => {
    const lowPrio = makeCapability('low', [controlPlacement({
      group: 'g', order: 5, priority: 70, dashboardRole: 'candidate',
      dedupeKey: 'shared.key',
    })]);
    const highPrio = makeCapability('high', [controlPlacement({
      group: 'g', order: 10, priority: 90, dashboardRole: 'candidate',
      dedupeKey: 'shared.key',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [lowPrio, highPrio] });

    const { selected, fallback } = selectDashboardControls(
      [lowPrio, highPrio], device, allAvailable, allHaveContent, new Set(),
    );

    // 两者都没 fixedSlot，且 priority<90 不可竞争第 4 槽；都进入 fallback 但只保留最高优先级。
    expect(selected).toHaveLength(0);
    const fallbackIds = fallback.map((c) => c.capability.id);
    expect(fallbackIds).toContain('high');
    expect(fallbackIds).not.toContain('low');
  });

  it('skips candidates whose dedupeKey is already in the input Set', () => {
    const dpi = makeCapability('dpi', [controlPlacement({
      group: 'performance', order: 10, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 1, dedupeKey: 'dashboard.dpi',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [dpi] });

    // dpi 的 dedupeKey 已被系统入口占用。
    const { selected } = selectDashboardControls(
      [dpi], device, allAvailable, allHaveContent, new Set(['dashboard.dpi']),
    );

    expect(selected).toHaveLength(0);
  });
});

describe('P0-B: selectDashboardControls fallback region', () => {
  it('returns non-selected candidates as fallback when fallbackRegion is advanced', () => {
    const extra = makeCapability('extra', [controlPlacement({
      group: 'extra', order: 5, priority: 50, dashboardRole: 'candidate',
      fourthSlotEligible: false, dedupeKey: 'dashboard.extra',
      fallbackRegion: 'advanced',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [extra] });

    const { fallback } = selectDashboardControls(
      [extra], device, allAvailable, allHaveContent, new Set(),
    );

    expect(fallback.map((c) => c.capability.id)).toEqual(['extra']);
  });

  it('excludes candidates whose fallbackRegion is hidden', () => {
    const hidden = makeCapability('hidden', [controlPlacement({
      group: 'hidden', order: 5, priority: 50, dashboardRole: 'candidate',
      fourthSlotEligible: false, dedupeKey: 'dashboard.hidden',
      fallbackRegion: 'hidden',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [hidden] });

    const { fallback } = selectDashboardControls(
      [hidden], device, allAvailable, allHaveContent, new Set(),
    );

    expect(fallback).toHaveLength(0);
  });
});

describe('P0-B: selectDashboardControls filtering', () => {
  it('filters out capabilities that fail availabilityFilter', () => {
    const dpi = makeCapability('dpi', [controlPlacement({
      group: 'performance', order: 10, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 1, dedupeKey: 'dashboard.dpi',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [dpi] });

    const { selected } = selectDashboardControls(
      [dpi], device, () => false, allHaveContent, new Set(),
    );

    expect(selected).toHaveLength(0);
  });

  it('filters out capabilities that fail contentFilter', () => {
    const dpi = makeCapability('dpi', [controlPlacement({
      group: 'performance', order: 10, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 1, dedupeKey: 'dashboard.dpi',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [dpi] });

    const { selected } = selectDashboardControls(
      [dpi], device, allAvailable, () => false, new Set(),
    );

    expect(selected).toHaveLength(0);
  });

  it('filters out capabilities whose metadata.visibleWhen does not match', () => {
    const bluetoothOnly = makeCapability('bt', [controlPlacement({
      group: 'g', order: 10, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 1, dedupeKey: 'dashboard.bt',
    })], { metadata: { visibleWhen: { path: 'connection', eq: 'bluetooth' } } });
    const device = makeDevice({}, { connection: 'usb', pluginCapabilities: [bluetoothOnly] });

    const { selected } = selectDashboardControls(
      [bluetoothOnly], device, allAvailable, allHaveContent, new Set(),
    );

    expect(selected).toHaveLength(0);
  });
});

// ─── P0-B：selectDashboardStatus 覆盖率 ─────────────────────────────────

describe('P0-B: selectDashboardStatus base slot selection', () => {
  it('selects up to 3 candidates with priority >= STATUS_BASE_SLOT_MIN_PRIORITY', () => {
    const sleep = makeCapability('sleep', [statusPlacement({
      order: 10, priority: 80, dedupeKey: 'status.sleep',
    })]);
    const profile = makeCapability('profile', [statusPlacement({
      order: 20, priority: 70, dedupeKey: 'status.profile',
    })]);
    const lighting = makeCapability('lighting', [statusPlacement({
      order: 30, priority: 65, dedupeKey: 'status.lighting',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [sleep, profile, lighting] });

    const { selected } = selectDashboardStatus(
      [sleep, profile, lighting], device, allAvailable, allHaveReported, new Set(),
    );

    expect(selected.map((c) => c.capability.id)).toEqual(['sleep', 'profile', 'lighting']);
  });

  it('does not backfill base slots when fewer than 3 candidates meet the priority threshold', () => {
    // 只有一个 priority>=60 的候选；另一个 priority=40 应被排除。
    const sleep = makeCapability('sleep', [statusPlacement({
      order: 10, priority: 80, dedupeKey: 'status.sleep',
    })]);
    const low = makeCapability('low', [statusPlacement({
      order: 20, priority: 40, dedupeKey: 'status.low',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [sleep, low] });

    const { selected, fallback } = selectDashboardStatus(
      [sleep, low], device, allAvailable, allHaveReported, new Set(),
    );

    expect(selected.map((c) => c.capability.id)).toEqual(['sleep']);
    // 低优先级项进入 fallback。
    expect(fallback.map((c) => c.capability.id)).toEqual(['low']);
  });

  it('selects fewer than 3 when only 1 candidate is available (no placeholder)', () => {
    const sleep = makeCapability('sleep', [statusPlacement({
      order: 10, priority: 80, dedupeKey: 'status.sleep',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [sleep] });

    const { selected } = selectDashboardStatus(
      [sleep], device, allAvailable, allHaveReported, new Set(),
    );

    expect(selected).toHaveLength(1);
  });
});

describe('P0-B: selectDashboardStatus 4th slot eligibility', () => {
  it('fills the 4th slot with a priority>=90 fourthSlotEligible deferred candidate', () => {
    // base 3 优先级高于 extra，确保 extra 进入 deferred 而非基础槽。
    const base = ['a', 'b', 'c'].map((id, idx) => makeCapability(id, [statusPlacement({
      order: (idx + 1) * 10, priority: 95 - idx, dedupeKey: `status.${id}`,
    })]));
    const extra = makeCapability('extra', [statusPlacement({
      order: 100, priority: 90, dedupeKey: 'status.extra',
      fourthSlotEligible: true,
    })]);
    const device = makeDevice({}, { pluginCapabilities: [...base, extra] });

    const { selected } = selectDashboardStatus(
      [...base, extra], device, allAvailable, allHaveReported, new Set(),
    );

    expect(selected).toHaveLength(STATUS_MAX_COUNT);
    expect(selected[3].capability.id).toBe('extra');
  });

  it('does not fill the 4th slot when deferred candidate priority < 90', () => {
    const base = ['a', 'b', 'c'].map((id, idx) => makeCapability(id, [statusPlacement({
      order: (idx + 1) * 10, priority: 95 - idx, dedupeKey: `status.${id}`,
    })]));
    const extra = makeCapability('extra', [statusPlacement({
      order: 100, priority: 85, dedupeKey: 'status.extra',
      fourthSlotEligible: true,
    })]);
    const device = makeDevice({}, { pluginCapabilities: [...base, extra] });

    const { selected } = selectDashboardStatus(
      [...base, extra], device, allAvailable, allHaveReported, new Set(),
    );

    expect(selected).toHaveLength(STATUS_PREFERRED_COUNT);
  });

  it('does not activate the 4th slot when fewer than 3 base candidates exist', () => {
    // 仅 2 个候选（其一为 fourthSlotEligible），均进入基础槽；4 槽逻辑不激活。
    const sleep = makeCapability('sleep', [statusPlacement({
      order: 10, priority: 80, dedupeKey: 'status.sleep',
    })]);
    const extra = makeCapability('extra', [statusPlacement({
      order: 100, priority: 95, dedupeKey: 'status.extra',
      fourthSlotEligible: true,
    })]);
    const device = makeDevice({}, { pluginCapabilities: [sleep, extra] });

    const { selected } = selectDashboardStatus(
      [sleep, extra], device, allAvailable, allHaveReported, new Set(),
    );

    // 两个候选都进入基础槽（priority>=60），selected.length=2 < PREFERRED_COUNT，
    // 第 4 槽位逻辑不激活，所以不会出现"为凑满 4 格而降低阈值"的情况。
    expect(selected).toHaveLength(2);
    // extra 在基础槽（按 priority desc 排序应排第一），而非第 4 槽。
    expect(selected[0].capability.id).toBe('extra');
    expect(selected[1].capability.id).toBe('sleep');
  });
});

describe('P0-B: selectDashboardStatus dedupeKey handling', () => {
  it('skips candidates whose dedupeKey is already used by control region', () => {
    // sleep 与 control 区共享 dedupeKey 'shared.key'。
    const sleep = makeCapability('sleep', [statusPlacement({
      order: 10, priority: 80, dedupeKey: 'shared.key',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [sleep] });

    const { selected } = selectDashboardStatus(
      [sleep], device, allAvailable, allHaveReported, new Set(['shared.key']),
    );

    expect(selected).toHaveLength(0);
  });

  it('deduplicates same dedupeKey status candidates keeping the highest priority', () => {
    const low = makeCapability('low', [statusPlacement({
      order: 5, priority: 70, dedupeKey: 'status.shared',
    })]);
    const high = makeCapability('high', [statusPlacement({
      order: 10, priority: 80, dedupeKey: 'status.shared',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [low, high] });

    const { selected } = selectDashboardStatus(
      [low, high], device, allAvailable, allHaveReported, new Set(),
    );

    expect(selected).toHaveLength(1);
    expect(selected[0].capability.id).toBe('high');
  });
});

describe('P0-B: selectDashboardStatus filtering', () => {
  it('filters out capabilities with no reported value', () => {
    const sleep = makeCapability('sleep', [statusPlacement({
      order: 10, priority: 80, dedupeKey: 'status.sleep',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [sleep] });

    const { selected } = selectDashboardStatus(
      [sleep], device, allAvailable, () => false, new Set(),
    );

    expect(selected).toHaveLength(0);
  });

  it('filters out capabilities whose visibleWhen does not match', () => {
    const bluetoothOnly = makeCapability('bt', [statusPlacement({
      order: 10, priority: 80, dedupeKey: 'status.bt',
    })], { metadata: { visibleWhen: { path: 'connection', eq: 'bluetooth' } } });
    const device = makeDevice({}, { connection: 'usb', pluginCapabilities: [bluetoothOnly] });

    const { selected } = selectDashboardStatus(
      [bluetoothOnly], device, allAvailable, allHaveReported, new Set(),
    );

    expect(selected).toHaveLength(0);
  });

  it('returns deferred low-priority candidates in fallback', () => {
    const sleep = makeCapability('sleep', [statusPlacement({
      order: 10, priority: 80, dedupeKey: 'status.sleep',
    })]);
    const low = makeCapability('low', [statusPlacement({
      order: 20, priority: 40, dedupeKey: 'status.low',
      fallbackRegion: 'advanced',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [sleep, low] });

    const { fallback } = selectDashboardStatus(
      [sleep, low], device, allAvailable, allHaveReported, new Set(),
    );

    expect(fallback.map((c) => c.capability.id)).toEqual(['low']);
  });

  it('excludes hidden fallbackRegion candidates from fallback', () => {
    const low = makeCapability('low', [statusPlacement({
      order: 20, priority: 40, dedupeKey: 'status.low',
      fallbackRegion: 'hidden',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [low] });

    const { fallback } = selectDashboardStatus(
      [low], device, allAvailable, allHaveReported, new Set(),
    );

    expect(fallback).toHaveLength(0);
  });
});

// ─── 常量约束 ────────────────────────────────────────────────────────────

describe('Dashboard selector constants', () => {
  it('exposes the expected thresholds', () => {
    expect(CONTROL_PREFERRED_COUNT).toBe(3);
    expect(CONTROL_MAX_COUNT).toBe(4);
    expect(STATUS_PREFERRED_COUNT).toBe(3);
    expect(STATUS_MAX_COUNT).toBe(4);
    expect(FOURTH_SLOT_MIN_PRIORITY).toBe(90);
    expect(STATUS_BASE_SLOT_MIN_PRIORITY).toBe(60);
  });
});

// ─── P1-A：子块选择器常量 ────────────────────────────────────────────────

describe('Subblock selector constants', () => {
  it('exposes the expected subblock limits', () => {
    expect(POLLING_MAX_SUBBLOCKS).toBe(3);
    expect(LIGHTING_MAX_SUBBLOCKS).toBe(6);
    expect(LIGHTING_MAX_CANDIDATES).toBe(4);
  });
});

// ─── P0-F：selectSummarySubblocks 覆盖率 ────────────────────────────────

describe('P0-F: selectSummarySubblocks', () => {
  function makeSummary(source: string, priority?: number): PluginSummaryItem {
    return { source, priority };
  }

  it('selects up to max items by priority desc', () => {
    const items = [
      makeSummary('a', 50),
      makeSummary('b', 90),
      makeSummary('c', 70),
      makeSummary('d', 30),
    ];
    const { selected, fallback } = selectSummarySubblocks(items, 3);
    expect(selected.map((i) => i.source)).toEqual(['b', 'c', 'a']);
    expect(fallback.map((i) => i.source)).toEqual(['d']);
  });

  it('returns all items as selected when count <= max', () => {
    const items = [makeSummary('a', 50), makeSummary('b', 90)];
    const { selected, fallback } = selectSummarySubblocks(items, 3);
    expect(selected.map((i) => i.source)).toEqual(['b', 'a']);
    expect(fallback).toHaveLength(0);
  });

  it('preserves declaration order for equal priorities (stable sort)', () => {
    const items = [
      makeSummary('first', 80),
      makeSummary('second', 80),
      makeSummary('third', 80),
    ];
    const { selected } = selectSummarySubblocks(items, 2);
    expect(selected.map((i) => i.source)).toEqual(['first', 'second']);
  });

  it('handles empty input', () => {
    const { selected, fallback } = selectSummarySubblocks([], 3);
    expect(selected).toHaveLength(0);
    expect(fallback).toHaveLength(0);
  });

  it('caps any plugin polling summary at three by placement semantics and priority', () => {
    const thirdPartyPolling = makeCapability('vendor-neutral-rate', [
      controlPlacement({
        group: 'polling',
        priority: 40,
        dedupeKey: 'third-party.polling',
      }),
    ]);
    const items = [
      makeSummary('state.low', 10),
      makeSummary('state.high', 100),
      makeSummary('state.midHigh', 80),
      makeSummary('state.mid', 60),
    ];
    const { selected, fallback } = selectSummarySubblocks(
      items,
      summaryMaxForCapability(thirdPartyPolling),
    );

    expect(selected.map((item) => item.source)).toEqual([
      'state.high',
      'state.midHigh',
      'state.mid',
    ]);
    expect(fallback.map((item) => item.source)).toEqual(['state.low']);
  });
});

// ─── P0-G：selectLightingSubblocks 覆盖率 ───────────────────────────────

describe('P0-G: selectLightingSubblocks', () => {
  function makeField(id: string, role?: PluginField['lightingRole'], priority?: number): PluginField {
    return { id, source: `state.${id}`, editor: 'modal-select', lightingRole: role, priority };
  }

  it('places effect at the left and primary-color at the right', () => {
    const fields = [
      makeField('speed', 'candidate', 80),
      makeField('color', 'primary-color', 100),
      makeField('effect', 'effect', 100),
      makeField('brightness', 'candidate', 70),
    ];
    const { selected } = selectLightingSubblocks(fields);
    expect(selected.map((f) => f.id)).toEqual(['effect', 'speed', 'brightness', 'color']);
  });

  it('keeps effect at the left even when other fields have higher priority', () => {
    const fields = [
      makeField('high', 'candidate', 95),
      makeField('effect', 'effect', 50),
      makeField('color', 'primary-color', 100),
    ];
    const { selected } = selectLightingSubblocks(fields);
    expect(selected[0].id).toBe('effect');
    expect(selected[selected.length - 1].id).toBe('color');
  });

  it('selects only the highest-priority effect when multiple exist', () => {
    const fields = [
      makeField('effect-low', 'effect', 50),
      makeField('effect-high', 'effect', 100),
      makeField('color', 'primary-color', 100),
    ];
    const { selected, fallback } = selectLightingSubblocks(fields);
    expect(selected.find((f) => f.lightingRole === 'effect')?.id).toBe('effect-high');
    expect(fallback.find((f) => f.id === 'effect-low')).toBeDefined();
  });

  it('limits candidates to LIGHTING_MAX_CANDIDATES', () => {
    const fields = [
      makeField('effect', 'effect', 100),
      makeField('c1', 'candidate', 90),
      makeField('c2', 'candidate', 80),
      makeField('c3', 'candidate', 70),
      makeField('c4', 'candidate', 60),
      makeField('c5', 'candidate', 50),
      makeField('c6', 'candidate', 40),
      makeField('color', 'primary-color', 100),
    ];
    const { selected, fallback } = selectLightingSubblocks(fields);
    const candidates = selected.filter((f) => f.lightingRole === 'candidate' || !f.lightingRole);
    expect(candidates).toHaveLength(LIGHTING_MAX_CANDIDATES);
    expect(fallback.find((f) => f.id === 'c5')).toBeDefined();
    expect(fallback.find((f) => f.id === 'c6')).toBeDefined();
  });

  it('treats fields without lightingRole as candidate', () => {
    const fields = [
      makeField('effect', 'effect', 100),
      makeField('untagged'),
      makeField('color', 'primary-color', 100),
    ];
    const { selected } = selectLightingSubblocks(fields);
    expect(selected.map((f) => f.id)).toEqual(['effect', 'untagged', 'color']);
  });

  it('works with only candidate fields (no effect or primary-color)', () => {
    const fields = [
      makeField('a', 'candidate', 90),
      makeField('b', 'candidate', 80),
    ];
    const { selected } = selectLightingSubblocks(fields);
    expect(selected.map((f) => f.id)).toEqual(['a', 'b']);
  });
});

// ─── P0-E/P0-G：leading/trailing candidate 行为 ──────────────────────────

describe('P0-E/P0-G: leading/trailing candidate placement', () => {
  it('places a leading candidate before the core sequence', () => {
    const dpi = makeCapability('dpi', [controlPlacement({
      group: 'performance', order: 10, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 1, dedupeKey: 'dashboard.dpi',
    })]);
    const polling = makeCapability('polling', [controlPlacement({
      group: 'polling', order: 20, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 2, dedupeKey: 'dashboard.polling',
    })]);
    const lighting = makeCapability('lighting', [controlPlacement({
      group: 'lighting', order: 30, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 3, dedupeKey: 'dashboard.lighting',
    })]);
    const leading = makeCapability('leading', [controlPlacement({
      group: 'leading', order: 5, priority: 95, dashboardRole: 'candidate',
      fourthSlotEligible: true, dedupeKey: 'dashboard.leading',
      optionalPosition: 'leading',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [dpi, polling, lighting, leading] });

    const { selected } = selectDashboardControls(
      [dpi, polling, lighting, leading], device, allAvailable, allHaveContent, new Set(),
    );

    expect(selected.map((c) => c.capability.id)).toEqual(['leading', 'dpi', 'polling', 'lighting']);
  });

  it('places a trailing candidate after the core sequence', () => {
    const dpi = makeCapability('dpi', [controlPlacement({
      group: 'performance', order: 10, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 1, dedupeKey: 'dashboard.dpi',
    })]);
    const polling = makeCapability('polling', [controlPlacement({
      group: 'polling', order: 20, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 2, dedupeKey: 'dashboard.polling',
    })]);
    const lighting = makeCapability('lighting', [controlPlacement({
      group: 'lighting', order: 30, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 3, dedupeKey: 'dashboard.lighting',
    })]);
    const trailing = makeCapability('trailing', [controlPlacement({
      group: 'trailing', order: 100, priority: 95, dashboardRole: 'candidate',
      fourthSlotEligible: true, dedupeKey: 'dashboard.trailing',
      optionalPosition: 'trailing',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [dpi, polling, lighting, trailing] });

    const { selected } = selectDashboardControls(
      [dpi, polling, lighting, trailing], device, allAvailable, allHaveContent, new Set(),
    );

    expect(selected.map((c) => c.capability.id)).toEqual(['dpi', 'polling', 'lighting', 'trailing']);
  });

  it('defaults optionalPosition to trailing when not declared', () => {
    const dpi = makeCapability('dpi', [controlPlacement({
      group: 'performance', order: 10, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 1, dedupeKey: 'dashboard.dpi',
    })]);
    const polling = makeCapability('polling', [controlPlacement({
      group: 'polling', order: 20, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 2, dedupeKey: 'dashboard.polling',
    })]);
    const lighting = makeCapability('lighting', [controlPlacement({
      group: 'lighting', order: 30, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 3, dedupeKey: 'dashboard.lighting',
    })]);
    const extra = makeCapability('extra', [controlPlacement({
      group: 'extra', order: 100, priority: 95, dashboardRole: 'candidate',
      fourthSlotEligible: true, dedupeKey: 'dashboard.extra',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [dpi, polling, lighting, extra] });

    const { selected } = selectDashboardControls(
      [dpi, polling, lighting, extra], device, allAvailable, allHaveContent, new Set(),
    );

    expect(selected.map((c) => c.capability.id)).toEqual(['dpi', 'polling', 'lighting', 'extra']);
  });

  it('allows a candidate when core is incomplete (P0-G relaxed gate)', () => {
    const dpi = makeCapability('dpi', [controlPlacement({
      group: 'performance', order: 10, priority: 100, dashboardRole: 'fixed-core',
      fixedSlot: 1, dedupeKey: 'dashboard.dpi',
    })]);
    const trailing = makeCapability('trailing', [controlPlacement({
      group: 'trailing', order: 100, priority: 95, dashboardRole: 'candidate',
      fourthSlotEligible: true, dedupeKey: 'dashboard.trailing',
      optionalPosition: 'trailing',
    })]);
    const device = makeDevice({}, { pluginCapabilities: [dpi, trailing] });

    const { selected } = selectDashboardControls(
      [dpi, trailing], device, allAvailable, allHaveContent, new Set(),
    );

    // 仅 1 个 fixedSlot + 1 个 trailing candidate = 2 项（不再要求核心三项全部就位）
    expect(selected.map((c) => c.capability.id)).toEqual(['dpi', 'trailing']);
  });
});

// ─── P1-B：validatePlacement 增强 ───────────────────────────────────────

describe('P1-B: validatePlacement enhanced checks', () => {
  function validPlacement(): PluginCapabilityPlacement {
    return {
      region: 'control',
      order: 10,
      span: 1,
      priority: 80,
      dashboardRole: 'candidate',
      fallbackRegion: 'advanced',
      dedupeKey: 'dashboard.test',
    };
  }

  it('returns null for a valid placement', () => {
    expect(validatePlacement(validPlacement())).toBeNull();
  });

  it('rejects empty dedupeKey string', () => {
    const p = validPlacement();
    p.dedupeKey = '';
    expect(validatePlacement(p)).toContain('dedupeKey must be a non-empty string');
  });

  it('rejects fallbackRegion=details (no longer accepted)', () => {
    const p = validPlacement();
    p.fallbackRegion = 'details' as PluginCapabilityPlacement['fallbackRegion'];
    expect(validatePlacement(p)).toContain("fallbackRegion must be one of advanced|inventory|hidden");
  });

  it('accepts fallbackRegion=inventory', () => {
    const p = validPlacement();
    p.fallbackRegion = 'inventory';
    expect(validatePlacement(p)).toBeNull();
  });

  it('rejects optionalPosition on fixed-core role', () => {
    const p = validPlacement();
    p.dashboardRole = 'fixed-core';
    p.fixedSlot = 1;
    p.optionalPosition = 'leading';
    expect(validatePlacement(p)).toContain("optionalPosition requires dashboardRole='candidate'");
  });

  it('accepts optionalPosition on candidate role', () => {
    const p = validPlacement();
    p.optionalPosition = 'leading';
    expect(validatePlacement(p)).toBeNull();
  });

  it('rejects an empty compactLabelKey', () => {
    const p = validPlacement();
    p.compactLabelKey = '   ';
    expect(validatePlacement(p)).toContain('compactLabelKey must be a non-empty string');
  });

  it('rejects fixedSlot on status region', () => {
    const p = validPlacement();
    p.region = 'status';
    p.fixedSlot = 1;
    p.dashboardRole = 'fixed-core';
    expect(validatePlacement(p)).toContain('status placement must not declare fixedSlot');
  });

  it('rejects priority out of range', () => {
    const p = validPlacement();
    p.priority = 150;
    expect(validatePlacement(p)).toContain('priority must be in [0, 100]');
  });
});
