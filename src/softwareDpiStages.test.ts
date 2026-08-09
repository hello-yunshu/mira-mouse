// SPDX-License-Identifier: AGPL-3.0-or-later
import { beforeEach, describe, expect, it } from 'vitest';
import type { DeviceState, PluginStageLayout } from './types';
import {
  loadSoftwareDpiStages,
  saveSoftwareDpiStages,
  softwareDpiCurrentValue,
  softwareDpiStageKey,
  softwareDpiStages,
} from './softwareDpiStages';

const layout: PluginStageLayout = {
  mode: 'software',
  currentValueSource: 'state.dpi',
  defaultValues: [400, 800, 1600, 3200, 6400],
  setMutation: 'set-dpi',
  valueParam: 'dpi',
  range: { min: 100, max: 26000, step: 50 },
};

const device = {
  name: 'Adjustable Mouse', pluginId: 'mira.example', family: 'direct',
  historyIdentity: { group: 'unit-a' }, state: { dpi: 1600 },
} as unknown as DeviceState;

class MemoryStorage implements Storage {
  private values = new Map<string, string>();
  get length() { return this.values.size; }
  clear() { this.values.clear(); }
  getItem(key: string) { return this.values.get(key) ?? null; }
  key(index: number) { return [...this.values.keys()][index] ?? null; }
  removeItem(key: string) { this.values.delete(key); }
  setItem(key: string, value: string) { this.values.set(key, value); }
}

let storage: Storage;

describe('software DPI stages', () => {
  beforeEach(() => { storage = new MemoryStorage(); });

  it('seeds declared stages and follows the reported DPI', () => {
    const key = softwareDpiStageKey(device, 'dpi');
    const state = loadSoftwareDpiStages(storage, key, layout, 1600);
    expect(state).toEqual({ values: [400, 800, 1600, 3200, 6400], selectedIndex: 2 });
    expect(softwareDpiStages(state, 1600).map((stage) => stage.active)).toEqual([false, false, true, false, false]);
  });

  it('persists edited presets per stable device identity', () => {
    const key = softwareDpiStageKey(device, 'dpi');
    saveSoftwareDpiStages(storage, key, { values: [500, 900, 1700, 3300, 6500], selectedIndex: 3 });
    expect(loadSoftwareDpiStages(storage, key, layout)).toEqual({
      values: [500, 900, 1700, 3300, 6500], selectedIndex: 3,
    });
    expect(softwareDpiStageKey({ ...device, connection: 'bluetooth' }, 'dpi')).toBe(key);
  });

  it('reads current DPI only from the plugin-declared path', () => {
    expect(softwareDpiCurrentValue(layout, device)).toBe(1600);
    expect(softwareDpiCurrentValue({ ...layout, currentValueSource: 'state.missing' }, device)).toBeUndefined();
  });
});
