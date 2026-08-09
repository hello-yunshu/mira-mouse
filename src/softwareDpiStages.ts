// SPDX-License-Identifier: AGPL-3.0-or-later
import type { DeviceState, DpiStage, PluginStageLayout } from './types';

const STORAGE_KEY = 'mira.software-dpi-stages.v1';
const STAGE_COLORS = ['#7ea7d8', '#8fc7b8', '#d4b483', '#c99ac7', '#9a8bd0', '#79b7c6', '#d88f8f', '#a6b879'];

export interface SoftwareDpiStageState {
  values: number[];
  selectedIndex: number;
}

type SoftwareDpiStageStore = Record<string, SoftwareDpiStageState>;

class MemoryStorage implements Storage {
  private values = new Map<string, string>();
  get length() { return this.values.size; }
  clear() { this.values.clear(); }
  getItem(key: string) { return this.values.get(key) ?? null; }
  key(index: number) { return [...this.values.keys()][index] ?? null; }
  removeItem(key: string) { this.values.delete(key); }
  setItem(key: string, value: string) { this.values.set(key, value); }
}

const fallbackStorage = new MemoryStorage();

export function softwareDpiStorage(): Storage {
  try {
    return typeof window !== 'undefined' && window.localStorage ? window.localStorage : fallbackStorage;
  } catch {
    return fallbackStorage;
  }
}

export function isSoftwareDpiLayout(layout: PluginStageLayout): boolean {
  return layout.mode === 'software';
}

export function softwareDpiStageKey(device: DeviceState, capabilityId: string): string {
  const identity = device.historyIdentity?.group
    ?? [device.family, device.name].filter(Boolean).join(':')
    ?? device.name;
  return [device.pluginId ?? 'unknown-plugin', identity, capabilityId].join(':');
}

function stepAligned(value: number, layout: PluginStageLayout): boolean {
  const { min, max, step = 1 } = layout.range;
  if (!Number.isFinite(value) || value < min || value > max) return false;
  const quotient = (value - min) / step;
  return Math.abs(quotient - Math.round(quotient)) < 1e-8;
}

function declaredValues(layout: PluginStageLayout): number[] {
  return (layout.defaultValues ?? [])
    .filter((value) => stepAligned(value, layout))
    .slice(0, 8);
}

function readStore(storage: Storage): SoftwareDpiStageStore {
  try {
    const parsed = JSON.parse(storage.getItem(STORAGE_KEY) ?? '{}') as unknown;
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed)
      ? parsed as SoftwareDpiStageStore
      : {};
  } catch {
    return {};
  }
}

export function loadSoftwareDpiStages(
  storage: Storage,
  key: string,
  layout: PluginStageLayout,
  currentDpi?: number,
): SoftwareDpiStageState {
  const defaults = declaredValues(layout);
  const stored = readStore(storage)[key];
  const storedValues = Array.isArray(stored?.values)
    ? stored.values.filter((value) => stepAligned(value, layout)).slice(0, 8)
    : [];
  const values = storedValues.length === defaults.length && storedValues.length >= 2
    ? storedValues
    : defaults;
  const currentIndex = currentDpi === undefined ? -1 : values.indexOf(currentDpi);
  const selectedIndex = currentIndex >= 0
    ? currentIndex
    : Math.min(Math.max(Number.isInteger(stored?.selectedIndex) ? stored.selectedIndex : 0, 0), Math.max(values.length - 1, 0));
  return { values, selectedIndex };
}

export function saveSoftwareDpiStages(storage: Storage, key: string, state: SoftwareDpiStageState): void {
  const store = readStore(storage);
  store[key] = state;
  storage.setItem(STORAGE_KEY, JSON.stringify(store));
}

export function softwareDpiStages(
  state: SoftwareDpiStageState,
  currentDpi?: number,
): DpiStage[] {
  const activeIndex = currentDpi === undefined ? -1 : state.values.indexOf(currentDpi);
  return state.values.map((value, index) => ({
    value,
    color: STAGE_COLORS[index % STAGE_COLORS.length],
    enabled: true,
    active: index === activeIndex,
  }));
}

export function softwareDpiCurrentValue(layout: PluginStageLayout, device: DeviceState): number | undefined {
  if (!layout.currentValueSource) return undefined;
  const parts = layout.currentValueSource.split('.');
  let current: unknown = device;
  for (const part of parts) {
    if (current == null || typeof current !== 'object') return undefined;
    current = (current as Record<string, unknown>)[part];
  }
  return typeof current === 'number' && Number.isFinite(current) ? current : undefined;
}
