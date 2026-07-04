// SPDX-License-Identifier: AGPL-3.0-or-later
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SettingsPage } from './Settings';
import type { AppSettings } from './types';

const { invokeMock, startAutomaticAppUpdateCheckMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  startAutomaticAppUpdateCheckMock: vi.fn(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('./updater', () => ({ startAutomaticAppUpdateCheck: startAutomaticAppUpdateCheckMock }));

const settings: AppSettings = {
  theme: 'system', autostart: false, startHidden: true, lowBatteryThreshold: 20,
  trayShowBatteryTitle: true, trayIncludeReceiverBattery: false, trayShowConnection: true,
  trayIconColor: 'white',
  nightModeEnabled: false, nightModeStart: '22:00', nightModeEnd: '07:00',
  nightModeTriggerTime: true, nightModeTriggerTheme: false, nightModeThemeDark: true,
  nightModeTriggerCharging: false, nightModeTriggerLowBattery: false,
  nightModeTargetMouse: true, nightModeTargetReceiver: false,
  refreshIntervalSeconds: 5, telemetryDisabled: true,
  automaticUpdateChecks: true, automaticUpdateInstall: false, automaticPluginUpdateChecks: true,
  batteryHistoryEnabled: true, batteryHistoryRetentionDays: 10, unusualDrainAlerts: false,
  language: 'auto',
};

describe('SettingsPage', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    startAutomaticAppUpdateCheckMock.mockReset();
  });

  it('loads settings and keeps unsupported controls honest', async () => {
    invokeMock.mockImplementation((command: string, payload?: { settings?: AppSettings }) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'settings_set') return Promise.resolve(payload?.settings);
      if (command === 'autostart_state') return Promise.resolve(false);
      if (command === 'about_info') return Promise.reject(new Error('not available in test'));
      if (command === 'plugin_updates_check') return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    const onThemeChange = vi.fn();
    render(<SettingsPage onNavigateAbout={vi.fn()} onThemeChange={onThemeChange} supportsAnyLighting supportsReceiverLighting={false} />);

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_get'));
    fireEvent.change(screen.getByRole('combobox', { name: '主题模式' }), { target: { value: 'dark' } });
    expect(onThemeChange).toHaveBeenCalledWith('dark');
    expect(screen.getByRole('switch', { name: '显示电量百分比' })).toBeChecked();
    const batteryTitleRow = screen.getByRole('switch', { name: '显示电量百分比' }).closest('.setting-row');
    const iconColorRow = screen.getByRole('combobox', { name: '托盘图标颜色' }).closest('.setting-row');
    expect(batteryTitleRow?.nextElementSibling).toBe(iconColorRow);
    fireEvent.change(screen.getByRole('combobox', { name: '托盘图标颜色' }), { target: { value: 'black' } });
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_set', expect.objectContaining({
      settings: expect.objectContaining({ trayIconColor: 'black' }),
    })));
    fireEvent.click(screen.getByRole('switch', { name: '标题附带接收器电量' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_set', expect.objectContaining({
      settings: expect.objectContaining({ trayIncludeReceiverBattery: true }),
    })));

    fireEvent.click(screen.getByRole('button', { name: '设备' }));
    const nightModeToggle = screen.getByRole('switch', { name: '启用安静灯光' });
    expect(nightModeToggle).not.toBeDisabled();
    expect(nightModeToggle).not.toBeChecked();
    expect(screen.queryByLabelText('开始时间')).toBeNull();
    fireEvent.click(nightModeToggle);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_set', expect.objectContaining({
      settings: expect.objectContaining({ nightModeEnabled: true }),
    })));
    const startInput = await screen.findByLabelText('开始时间');
    fireEvent.change(startInput, { target: { value: '23:00' } });
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_set', expect.objectContaining({
      settings: expect.objectContaining({ nightModeStart: '23:00' }),
    })));
    fireEvent.click(screen.getByRole('switch', { name: '跟随系统主题' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_set', expect.objectContaining({
      settings: expect.objectContaining({ nightModeTriggerTheme: true, nightModeTriggerTime: false }),
    })));
    fireEvent.click(screen.getByRole('switch', { name: '仅在充电时' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_set', expect.objectContaining({
      settings: expect.objectContaining({ nightModeTriggerCharging: true }),
    })));
    const receiverToggle = screen.getByRole('switch', { name: '接收器灯光' });
    expect(receiverToggle).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: '隐私' }));
    expect(screen.getByRole('switch', { name: '禁用遥测' })).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: '插件' }));
    fireEvent.click(screen.getByRole('button', { name: '检查插件更新' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('plugin_updates_check'));
  });

  it('syncs automatic application update scheduling when settings change', async () => {
    invokeMock.mockImplementation((command: string, payload?: { settings?: AppSettings }) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'settings_set') return Promise.resolve(payload?.settings);
      if (command === 'autostart_state') return Promise.resolve(false);
      if (command === 'about_info') return Promise.resolve({ bundledPlugins: [], updaterActive: true });
      return Promise.resolve(undefined);
    });
    render(<SettingsPage onNavigateAbout={vi.fn()} onThemeChange={vi.fn()} />);

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_get'));
    fireEvent.click(screen.getByRole('switch', { name: '自动检查 Mira 更新' }));
    await waitFor(() => expect(startAutomaticAppUpdateCheckMock).toHaveBeenCalledWith(false));

    fireEvent.click(screen.getByRole('switch', { name: '自动检查 Mira 更新' }));
    fireEvent.click(screen.getByRole('switch', { name: '自动下载并安装' }));
    await waitFor(() => expect(startAutomaticAppUpdateCheckMock).toHaveBeenCalledWith(true, true));
  });
});
