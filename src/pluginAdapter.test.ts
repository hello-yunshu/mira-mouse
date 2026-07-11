// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from 'vitest';
import {
  resolveFieldLabel,
  resolveFieldValueLabel,
  resolveFieldMutationParams,
  resolveFieldOptions,
  resolveFieldParams,
  resolveLightingMutations,
  resolveMutation,
  resolveStatusField,
  resolveStateMapping,
  resolveSwitchState,
  resolveVisibleWhen,
  resolveZones,
} from './pluginAdapter';
import type { DeviceState, PluginCapability, PluginField } from './types';

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

describe('resolveMutation', () => {
  it('selects the first declaration candidate supported by this device', () => {
    expect(resolveMutation(['legacy-write', 'preferred-write'], ['preferred-write'])).toBe('preferred-write');
    expect(resolveMutation('preferred-write', ['preferred-write'])).toBe('preferred-write');
    expect(resolveMutation(['legacy-write'], ['preferred-write'])).toBeUndefined();
  });
});

describe('resolveFieldParams', () => {
  it('hydrates a grouped mutation from plugin-declared snapshot paths', () => {
    const field: PluginField = {
      id: 'brightness',
      source: 'capabilities.receiverLighting.brightness',
      editor: 'modal-select',
      param: 'brightness',
      params: { target: 'receiver' },
      paramSources: {
        effect: 'capabilities.receiverLighting.effect',
        speed: 'capabilities.receiverLighting.speed',
        brightness: 'capabilities.receiverLighting.brightness',
        option: 'capabilities.receiverLighting.option',
        color: 'capabilities.receiverLighting.color',
      },
    };
    const device = makeDevice({}, {
      capabilities: {
        receiverLighting: { effect: 3, speed: 2, brightness: 1, option: 7, color: '#AABBCC' },
      },
    });

    expect(resolveFieldParams(field, device)).toEqual({
      effect: 3, speed: 2, brightness: 1, option: 7, color: '#AABBCC', target: 'receiver',
    });
    expect(resolveFieldMutationParams(field, device, 4)).toEqual({
      effect: 3, speed: 2, brightness: 4, option: 7, color: '#AABBCC', target: 'receiver',
    });
  });

  it('omits unavailable source values instead of inventing defaults', () => {
    const field: PluginField = {
      id: 'color',
      source: 'state.color',
      editor: 'modal-color',
      param: 'color',
      paramSources: { enabled: 'state.enabled', speed: 'state.speed' },
    };
    expect(resolveFieldMutationParams(field, makeDevice({ enabled: true }), '#112233')).toEqual({
      enabled: true,
      color: '#112233',
    });
  });

  it('uses declared defaults only when the live snapshot omits a grouped parameter', () => {
    const field: PluginField = {
      id: 'effect',
      source: 'state.effect',
      editor: 'modal-select',
      param: 'effect',
      params: { extraColor: '#000000', brightness: 100 },
      paramSources: {
        extraColor: 'state.extraColor',
        brightness: 'state.brightness',
      },
    };
    const device = makeDevice({ brightness: 70 });
    expect(resolveFieldMutationParams(field, device, 3)).toEqual({
      extraColor: '#000000', brightness: 70, effect: 3,
    });
  });
});

describe('resolveFieldOptions', () => {
  it('returns declared options when optionSource is absent', () => {
    const field: PluginField = {
      id: 'polling-rate',
      source: 'state.pollingRate',
      editor: 'modal-select',
      options: [
        { value: 125, labelKey: '125' },
        { value: 250, labelKey: '250' },
      ],
    };
    expect(resolveFieldOptions(field, makeDevice())).toEqual([
      { value: 125, labelKey: '125' },
      { value: 250, labelKey: '250' },
    ]);
  });

  it('merges runtime optionSource values first, then declared, deduped', () => {
    const field: PluginField = {
      id: 'polling-rate',
      source: 'state.pollingRate',
      editor: 'modal-select',
      optionSource: 'state.supportedPollingRates',
      options: [
        { value: 125, labelKey: '125' },
        { value: 250, labelKey: '250' },
        { value: 500, labelKey: '500' },
        { value: 1000, labelKey: '1000' },
      ],
    };
    const device = makeDevice({ supportedPollingRates: [1000, 2000, 4000, 8000] });

    // runtime primitives: 1000 matches declared, others get String(value) labelKey
    // merge order: runtime [1000, 2000, 4000, 8000] then declared [125, 250, 500] (1000 deduped)
    expect(resolveFieldOptions(field, device)).toEqual([
      { value: 1000, labelKey: '1000' },
      { value: 2000, labelKey: '2000' },
      { value: 4000, labelKey: '4000' },
      { value: 8000, labelKey: '8000' },
      { value: 125, labelKey: '125' },
      { value: 250, labelKey: '250' },
      { value: 500, labelKey: '500' },
    ]);
  });
});

describe('resolveSwitchState', () => {
  it('returns true when field has no switch declaration', () => {
    const field: PluginField = { id: 'effect', source: 'state.effect', editor: 'modal-select' };
    expect(resolveSwitchState(field, makeDevice({ effect: 0 }))).toBe(true);
  });

  it('returns true when switch source value differs from offValue', () => {
    const field: PluginField = {
      id: 'effect',
      source: 'state.effect',
      editor: 'modal-select',
      switch: { source: 'state.enabled', offValue: 0 },
    };
    expect(resolveSwitchState(field, makeDevice({ enabled: 1 }))).toBe(true);
  });

  it('returns false when switch source value equals offValue', () => {
    const field: PluginField = {
      id: 'effect',
      source: 'state.effect',
      editor: 'modal-select',
      switch: { source: 'state.enabled', offValue: 0 },
    };
    expect(resolveSwitchState(field, makeDevice({ enabled: 0 }))).toBe(false);
  });

  it('returns true when switch source path is missing (undefined !== offValue)', () => {
    const field: PluginField = {
      id: 'effect',
      source: 'state.effect',
      editor: 'modal-select',
      switch: { source: 'state.enabled', offValue: 0 },
    };
    expect(resolveSwitchState(field, makeDevice({}))).toBe(true);
  });

  it('supports string offValue', () => {
    const field: PluginField = {
      id: 'effect',
      source: 'state.effect',
      editor: 'modal-select',
      switch: { source: 'state.mode', offValue: 'off' },
    };
    expect(resolveSwitchState(field, makeDevice({ mode: 'breathing' }))).toBe(true);
    expect(resolveSwitchState(field, makeDevice({ mode: 'off' }))).toBe(false);
  });
});

describe('resolveFieldLabel', () => {
  it('keeps the field title separate from its runtime value label', () => {
    const field: PluginField = {
      id: 'effect',
      source: 'state.effect',
      editor: 'modal-select',
      labelKey: 'receiverLighting.field.effect',
      labelSource: 'capabilities.lighting.effectName',
    };
    const device = makeDevice({}, { capabilities: { lighting: { effectName: '霓虹' } } });
    expect(resolveFieldLabel(field, device)).toBe('灯效');
    expect(resolveFieldValueLabel(field, device)).toBe('霓虹');
  });

  it('falls through to options when the runtime value label is empty', () => {
    const field: PluginField = {
      id: 'effect',
      source: 'state.effect',
      editor: 'modal-select',
      labelSource: 'capabilities.lighting.effectName',
      options: [
        { value: 3, labelKey: '霓虹' },
      ],
    };
    const device = makeDevice({ effect: 3 }, { capabilities: { lighting: { effectName: '' } } });
    expect(resolveFieldValueLabel(field, device)).toBe('霓虹');
  });

  it('falls through to options when labelSource is null', () => {
    const field: PluginField = {
      id: 'effect',
      source: 'state.effect',
      editor: 'modal-select',
      labelSource: 'capabilities.lighting.effectName',
      options: [
        { value: 1, labelKey: '常亮' },
      ],
    };
    const device = makeDevice({ effect: 1 }, { capabilities: { lighting: { effectName: null } } });
    expect(resolveFieldValueLabel(field, device)).toBe('常亮');
  });

  it('returns labelKey when no labelSource and no options match', () => {
    const field: PluginField = {
      id: 'polling-rate',
      source: 'state.pollingRate',
      editor: 'modal-select',
      labelKey: 'custom.untranslated.key',
    };
    expect(resolveFieldLabel(field, makeDevice({ pollingRate: 1000 }))).toBe('custom.untranslated.key');
  });

  it('returns empty title when labelKey is absent', () => {
    const field: PluginField = { id: 'x', source: 'state.x', editor: 'modal-select' };
    expect(resolveFieldLabel(field, makeDevice())).toBe('');
  });

  it('matches value labels by field.source value', () => {
    const field: PluginField = {
      id: 'mode',
      source: 'state.mode',
      editor: 'inline-segmented',
      options: [
        { value: 1, labelKey: '板载' },
        { value: 2, labelKey: '软件' },
      ],
    };
    expect(resolveFieldValueLabel(field, makeDevice({ mode: 2 }))).toBe('软件');
  });
});

describe('resolveVisibleWhen', () => {
  it('returns true when condition is undefined', () => {
    expect(resolveVisibleWhen(undefined, makeDevice())).toBe(true);
  });

  it('returns true when value equals eq', () => {
    const device = makeDevice({ connection: 'wireless' });
    expect(resolveVisibleWhen({ path: 'state.connection', eq: 'wireless' }, device)).toBe(true);
  });

  it('returns false when value does not equal eq', () => {
    const device = makeDevice({ connection: 'usb' });
    expect(resolveVisibleWhen({ path: 'state.connection', eq: 'wireless' }, device)).toBe(false);
  });

  it('returns true when value does not equal ne', () => {
    const device = makeDevice({ connection: 'usb' });
    expect(resolveVisibleWhen({ path: 'state.connection', ne: 'wireless' }, device)).toBe(true);
  });

  it('returns false when value equals ne', () => {
    const device = makeDevice({ connection: 'wireless' });
    expect(resolveVisibleWhen({ path: 'state.connection', ne: 'wireless' }, device)).toBe(false);
  });

  it('returns true when value is non-null and no eq/ne', () => {
    const device = makeDevice({ effect: 3 });
    expect(resolveVisibleWhen({ path: 'state.effect' }, device)).toBe(true);
  });

  it('returns false when value is null and no eq/ne', () => {
    const device = makeDevice({});
    expect(resolveVisibleWhen({ path: 'state.effect' }, device)).toBe(false);
  });

  it('reads from capabilities path', () => {
    const device = makeDevice({}, { capabilities: { lighting: { enabled: true } } });
    expect(resolveVisibleWhen({ path: 'capabilities.lighting.enabled', eq: true }, device)).toBe(true);
  });
});

describe('resolveZones', () => {
  function makeCapability(zones: PluginCapability['metadata']['zones']): PluginCapability {
    return {
      id: 'lighting',
      control: 'LightingZone',
      labelKey: 'capability.lighting',
      readOnly: false,
      metadata: { zones },
    };
  }

  it('returns empty array when no zones declared', () => {
    expect(resolveZones(makeCapability(undefined), makeDevice())).toEqual([]);
  });

  it('returns all zones when none have visibleWhen', () => {
    const cap = makeCapability([
      { id: 'mouse', labelKey: 'mouse', fields: [] },
      { id: 'receiver', labelKey: 'receiver', fields: [] },
    ]);
    expect(resolveZones(cap, makeDevice())).toHaveLength(2);
  });

  it('filters zones by visibleWhen eq', () => {
    const cap = makeCapability([
      { id: 'mouse', labelKey: 'mouse', fields: [], visibleWhen: { path: 'state.connection', eq: 'wireless' } },
      { id: 'receiver', labelKey: 'receiver', fields: [] },
    ]);
    expect(resolveZones(cap, makeDevice({ connection: 'usb' }))).toHaveLength(1);
    expect(resolveZones(cap, makeDevice({ connection: 'wireless' }))).toHaveLength(2);
  });

  it('filters zones by visibleWhen ne', () => {
    const cap = makeCapability([
      { id: 'mouse', labelKey: 'mouse', fields: [] },
      { id: 'receiver', labelKey: 'receiver', fields: [], visibleWhen: { path: 'state.hasReceiver', ne: false } },
    ]);
    expect(resolveZones(cap, makeDevice({ hasReceiver: true }))).toHaveLength(2);
    expect(resolveZones(cap, makeDevice({ hasReceiver: false }))).toHaveLength(1);
  });
});

describe('resolveStatusField', () => {
  it('uses a visible sibling when the declared status field is hidden by the connection', () => {
    const capability: PluginCapability = {
      id: 'sleep-time',
      control: 'Number',
      labelKey: 'capability.sleep-time',
      readOnly: false,
      metadata: {
        fields: [
          {
            id: 'bluetooth', source: 'capabilities.settings.bluetoothSleepValue', mutation: 'set-bluetooth-sleep-time',
            param: 'seconds', editor: 'modal-range', format: 'sleep', visibleWhen: { path: 'connection', eq: 'bluetooth' },
          },
          {
            id: 'wireless', source: 'capabilities.settings.wirelessSleepValue', mutation: 'set-wireless-sleep-time',
            param: 'seconds', editor: 'modal-range', format: 'sleep', visibleWhen: { path: 'connection', ne: 'bluetooth' },
          },
        ],
      },
    };

    const bluetooth = makeDevice({}, { connection: 'bluetooth' });
    const wireless = makeDevice({}, { connection: 'wireless' });
    expect(resolveStatusField(capability, 'wireless', bluetooth)?.id).toBe('bluetooth');
    expect(resolveStatusField(capability, 'wireless', wireless)?.id).toBe('wireless');
  });
});

describe('resolveLightingMutations', () => {
  it('returns empty array when no LightingZone capabilities', () => {
    const caps: PluginCapability[] = [
      { id: 'dpi', control: 'DpiStages', labelKey: 'dpi', readOnly: false, metadata: {} },
    ];
    expect(resolveLightingMutations(caps, ['set-dpi'])).toEqual([]);
  });

  it('collects mutations from zones fields', () => {
    const caps: PluginCapability[] = [
      {
        id: 'lighting',
        control: 'LightingZone',
        labelKey: 'lighting',
        readOnly: false,
        metadata: {
          zones: [
            {
              id: 'mouse',
              labelKey: 'mouse',
              fields: [
                { id: 'effect', source: 'state.effect', editor: 'modal-select', mutation: 'set-mouse-effect' },
                { id: 'color', source: 'state.color', editor: 'modal-color', mutation: 'set-mouse-color' },
              ],
            },
          ],
        },
      },
    ];
    expect(resolveLightingMutations(caps, ['set-mouse-effect', 'set-mouse-color'])).toEqual([
      'set-mouse-effect',
      'set-mouse-color',
    ]);
  });

  it('excludes mutations not in writableMutations', () => {
    const caps: PluginCapability[] = [
      {
        id: 'lighting',
        control: 'LightingZone',
        labelKey: 'lighting',
        readOnly: false,
        metadata: {
          zones: [
            {
              id: 'mouse',
              labelKey: 'mouse',
              fields: [
                { id: 'effect', source: 'state.effect', editor: 'modal-select', mutation: 'set-mouse-effect' },
                { id: 'color', source: 'state.color', editor: 'modal-color', mutation: 'set-mouse-color' },
              ],
            },
          ],
        },
      },
    ];
    expect(resolveLightingMutations(caps, ['set-mouse-effect'])).toEqual(['set-mouse-effect']);
  });

  it('deduplicates mutations across zones', () => {
    const caps: PluginCapability[] = [
      {
        id: 'lighting',
        control: 'LightingZone',
        labelKey: 'lighting',
        readOnly: false,
        metadata: {
          zones: [
            {
              id: 'mouse',
              labelKey: 'mouse',
              fields: [
                { id: 'effect', source: 'state.effect', editor: 'modal-select', mutation: 'set-light-effect' },
              ],
            },
            {
              id: 'receiver',
              labelKey: 'receiver',
              fields: [
                { id: 'effect', source: 'state.receiverEffect', editor: 'modal-select', mutation: 'set-light-effect' },
              ],
            },
          ],
        },
      },
    ];
    expect(resolveLightingMutations(caps, ['set-light-effect'])).toEqual(['set-light-effect']);
  });

  it('skips fields without mutation string', () => {
    const caps: PluginCapability[] = [
      {
        id: 'lighting',
        control: 'LightingZone',
        labelKey: 'lighting',
        readOnly: false,
        metadata: {
          zones: [
            {
              id: 'mouse',
              labelKey: 'mouse',
              fields: [
                { id: 'status', source: 'state.status', editor: 'static-readonly' },
                { id: 'effect', source: 'state.effect', editor: 'modal-select', mutation: 'set-effect' },
              ],
            },
          ],
        },
      },
    ];
    expect(resolveLightingMutations(caps, ['set-effect'])).toEqual(['set-effect']);
  });
});

describe('resolveStateMapping', () => {
  it('returns empty object when no capabilities', () => {
    expect(resolveStateMapping([])).toEqual({});
  });

  it('merges stateMapping from multiple capabilities', () => {
    const caps: PluginCapability[] = [
      {
        id: 'battery',
        control: 'ReadOnlyValue',
        labelKey: 'battery',
        readOnly: true,
        metadata: { stateMapping: { battery: 'batteryPercent', charging: 'charging' } },
      },
      {
        id: 'lighting',
        control: 'LightingZone',
        labelKey: 'lighting',
        readOnly: false,
        metadata: { stateMapping: { mouseLightColor: 'confirmedLightColor' } },
      },
    ];
    expect(resolveStateMapping(caps)).toEqual({
      battery: 'batteryPercent',
      charging: 'charging',
      mouseLightColor: 'confirmedLightColor',
    });
  });

  it('returns empty object when capabilities have no stateMapping', () => {
    const caps: PluginCapability[] = [
      { id: 'dpi', control: 'DpiStages', labelKey: 'dpi', readOnly: false, metadata: {} },
    ];
    expect(resolveStateMapping(caps)).toEqual({});
  });
});
