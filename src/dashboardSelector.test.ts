// SPDX-License-Identifier: AGPL-3.0-or-later
// ITERATION-005 §P0-A/P0-B：Dashboard 选择器纯函数 + 覆盖率测试。
import { describe, expect, it } from 'vitest';
import {
  CONTROL_PREFERRED_COUNT,
  CONTROL_MAX_COUNT,
  FOURTH_SLOT_MIN_PRIORITY,
  STATUS_BASE_SLOT_MIN_PRIORITY,
  STATUS_PREFERRED_COUNT,
  STATUS_MAX_COUNT,
  selectDashboardControls,
  selectDashboardStatus,
} from './pluginAdapter';
import type { DeviceState, PluginCapability, PluginCapabilityPlacement } from './types';

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

    // 普通候选绝不填入 fixedSlot=2；fixedSlot=2 位置为空。
    expect(selected.find((c) => c.fixedSlot === 2)).toBeUndefined();
    // 仅 2 个 fixedSlot 入选（< CONTROL_PREFERRED_COUNT），第 4 槽位逻辑不激活，
    // extra 进入 fallback 而非 selected。这验证了"不强制填充"和"第 4 槽位需 PREFERRED_COUNT"。
    expect(selected.map((c) => c.capability.id)).toEqual(['dpi', 'lighting']);
    expect(fallback.map((c) => c.capability.id)).toContain('extra');
  });

  it('does not fill the 4th slot when fewer than 3 fixed slots are filled', () => {
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

    // 仅 1 个 fixedSlot 时不应启用第 4 槽（PREFERRED_COUNT 未达到）。
    expect(selected.map((c) => c.capability.id)).toEqual(['dpi']);
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
