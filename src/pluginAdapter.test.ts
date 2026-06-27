// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from 'vitest';
import { pluginOptions } from './pluginAdapter';
import type { DeviceState, PluginCapability } from './types';

describe('pluginOptions', () => {
  it('filters device-reported polling rates through mutation-scoped options', () => {
    const capability: PluginCapability = {
      id: 'polling-rate',
      control: 'Select',
      labelKey: 'capability.polling-rate',
      readOnly: false,
      metadata: {
        options: [125, 250, 500, 1000].map((value) => ({ value, label: `${value} Hz` })),
      },
    };
    const device = { supportedPollingRates: [1000, 2000, 4000, 8000] } as DeviceState;

    expect(pluginOptions(capability, device)).toEqual([{ value: 1000, label: '1000 Hz' }]);
  });
});
