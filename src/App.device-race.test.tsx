// SPDX-License-Identifier: AGPL-3.0-or-later
// 阶段一回归：设备快照请求与事件的到达顺序。
// 旧实现中，首次 device_snapshots 或 device_select 的响应可能晚于更新的
// device 事件返回，并用旧数据覆盖新状态。这两个用例在旧代码上必须失败。
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import type { AppSettings, DeviceSnapshot, DeviceSnapshotEntry } from './types';

vi.mock('thinking-orbs', () => ({
  ThinkingOrb: ({ state }: { state: string }) => (
    <span data-testid="thinking-orb" data-state={state} />
  ),
}));

const { eventCallbacks, invokeMock, listenMock } = vi.hoisted(() => ({
  eventCallbacks: new Map<string, (event: { payload: unknown }) => void>(),
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }));

const settings: AppSettings = {
  theme: 'light', autostart: false, startHidden: false, lowBatteryThreshold: 20,
  trayShowBatteryTitle: true, trayIncludeReceiverBattery: false, trayShowConnection: true,
  trayShowBatteryIcon: false,
  trayIconColor: 'auto', trayRenderMode: 'auto',
  nightModeEnabled: false, nightModeStart: '22:00', nightModeEnd: '07:00',
  nightModeTriggerTime: true, nightModeTriggerTheme: false, nightModeThemeDark: true,
  nightModeTriggerCharging: false, nightModeTriggerLowBattery: false,
  nightModeTargetMouse: true, nightModeTargetReceiver: false,
  telemetryDisabled: true,
  automaticUpdateChecks: true, automaticUpdateInstall: false, automaticPluginUpdateChecks: true,
  automaticLocalAiUpdateChecks: true,
  localAiAnalysisEnabled: false,
  localAiFeatures: { batteryUsage: true },
  batteryHistoryEnabled: true, batteryHistoryRetentionDays: 30, unusualDrainAlerts: false,
  wakeOnUnlock: false,
  language: 'auto',
};

function makeSnapshot(displayName: string): DeviceSnapshot {
  return {
    displayName,
    connection: 'usb',
    charging: false,
    batteries: [],
    capabilities: {},
    pluginCapabilities: [],
    writableMutations: [],
    evidence: 'hardware-verified',
  };
}

function entries(selectedIndex: number, ...snapshots: DeviceSnapshot[]): DeviceSnapshotEntry[] {
  return snapshots.map((snapshot, index) => ({
    deviceKey: `device-${index}`,
    snapshot,
    selected: index === selectedIndex,
  }));
}

beforeEach(() => {
  eventCallbacks.clear();
  invokeMock.mockReset();
  listenMock.mockReset();
  listenMock.mockImplementation(async (
    eventName: string,
    callback: (event: { payload: unknown }) => void,
  ) => {
    eventCallbacks.set(eventName, callback);
    return () => eventCallbacks.delete(eventName);
  });
});

describe('device snapshot ordering', () => {
  it('ignores a late initial device_snapshots response after a newer device event', async () => {
    let resolveInitial!: (value: DeviceSnapshotEntry[]) => void;
    const initial = new Promise<DeviceSnapshotEntry[]>((resolve) => {
      resolveInitial = resolve;
    });
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return initial;
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await waitFor(() => expect(eventCallbacks.has('device-updated')).toBe(true));

    const stale = makeSnapshot('Stale Mouse');
    await act(async () => {
      eventCallbacks.get('device-updated')!({ payload: makeSnapshot('Newest Mouse') });
    });
    expect(await screen.findByRole('heading', { name: 'Newest Mouse' })).toBeInTheDocument();

    await act(async () => {
      resolveInitial(entries(0, stale));
      await initial;
    });
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Newest Mouse' })).toBeInTheDocument());
    expect(screen.queryByRole('heading', { name: 'Stale Mouse' })).not.toBeInTheDocument();
  });

  it('does not let a late device_select response overwrite newer snapshot entries', async () => {
    let resolveSelect!: (value: DeviceSnapshot) => void;
    const select = new Promise<DeviceSnapshot>((resolve) => {
      resolveSelect = resolve;
    });
    const first = makeSnapshot('First Mouse');
    const second = makeSnapshot('Second Mouse');
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(0, first, second));
      if (command === 'device_select') return select;
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect(await screen.findByRole('heading', { name: 'First Mouse' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '切换鼠标' }));
    fireEvent.click(screen.getByText('Second Mouse').closest('button')!);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('device_select', { deviceKey: 'device-1' }));

    const updated = makeSnapshot('Second Mouse Updated');
    await act(async () => {
      eventCallbacks.get('device-snapshots-updated')!({ payload: entries(1, first, updated) });
    });
    expect(await screen.findByRole('heading', { name: 'Second Mouse Updated' })).toBeInTheDocument();

    await act(async () => {
      resolveSelect(second);
      await select;
    });
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Second Mouse Updated' })).toBeInTheDocument());
    expect(screen.queryByRole('heading', { name: 'Second Mouse' })).not.toBeInTheDocument();
  });
});
