// SPDX-License-Identifier: AGPL-3.0-or-later
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import App from './App';
import type { AppSettings, DeviceSnapshot } from './types';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

const settings: AppSettings = {
  theme: 'light', autostart: false, startHidden: false, lowBatteryThreshold: 20,
  trayShowBatteryTitle: true, trayIncludeReceiverBattery: false, trayShowConnection: true,
  nightModeEnabled: false, nightModeStart: '22:00', nightModeEnd: '07:00',
  refreshIntervalSeconds: 5, telemetryDisabled: true,
};

const snapshot: DeviceSnapshot = {
  displayName: 'AM INFINITY 8K MOUSE', connection: 'wireless', batteryPercent: 76,
  charging: false, dpi: 1600, pollingRateHz: 1000, profile: 'Profile 1',
  batteries: [
    { id: 'mouse', label: '鼠标', percentage: 76, charging: false },
    { id: 'receiver', label: '接收器', percentage: 100, charging: false },
  ],
  confirmedLightColor: '#112233', evidence: 'hardware-verified',
  dpiStages: [
    { value: 800, color: '#445566', active: false, enabled: true },
    { value: 1600, color: '#AABBCC', active: true, enabled: true },
  ],
  capabilities: {
    settings: { pollingRate: 1000, motionSync: true, angleSnap: false, liftCutOff: 2, mouseLightStartColor: '#112233', mouseLightEndColor: '#112233', mouseLightEnabled: true },
    receiverLighting: { effect: 3, speed: 5, brightness: 70, color: '#AABBCC' },
    firmwareUsb: { versionRaw: 258 },
    buttonMappings: { '0x00': [1, 0, 0, 0] },
  },
};

describe('real device snapshot mapping', () => {
  it('keeps all plugin capabilities available to the UI', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshot') return Promise.resolve(snapshot);
      return Promise.reject(new Error(`unexpected command ${command}`));
    });
    render(<App />);
    expect(await screen.findByText('AM INFINITY 8K MOUSE')).toBeInTheDocument();
    expect(screen.getByLabelText('当前 DPI：1600')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '800' }));
    expect(screen.getByLabelText('当前 DPI：1600')).toBeInTheDocument();
    expect(screen.getByText(/硬件当前值仍为 1600 DPI/)).toBeInTheDocument();
    expect(document.documentElement.style.getPropertyValue('--accent')).toContain('210');
    fireEvent.click(screen.getByRole('button', { name: /76%/ }));
    expect(screen.getByRole('region', { name: '设备电量' })).toHaveTextContent('鼠标76%');
    expect(screen.getByRole('region', { name: '设备电量' })).toHaveTextContent('接收器100%');
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    expect(screen.queryByRole('tab', { name: '字符灯' })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('tab', { name: '接收器灯光' }));
    expect(screen.getByText('灯效 3')).toBeInTheDocument();
    expect(screen.getByText('70%')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '全部读取信息' }));
    await waitFor(() => expect(screen.getByText('鼠标 USB 固件')).toBeInTheDocument());
    expect(screen.getByText('按键映射')).toBeInTheDocument();
  });
});
