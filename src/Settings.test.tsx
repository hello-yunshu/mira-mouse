// SPDX-License-Identifier: AGPL-3.0-or-later
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SettingsPage } from './Settings';
import type { AppSettings, PluginCapability } from './types';
import { checkForPluginUpdates } from './plugin-updater';

const { invokeMock, startAutomaticAppUpdateCheckMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  startAutomaticAppUpdateCheckMock: vi.fn(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('./updater', () => ({ startAutomaticAppUpdateCheck: startAutomaticAppUpdateCheckMock }));

const settings: AppSettings = {
  theme: 'system', autostart: false, startHidden: true, lowBatteryThreshold: 20,
  trayShowBatteryTitle: true, trayIncludeReceiverBattery: false, trayShowConnection: true,
  trayIconColor: 'auto', trayRenderMode: 'auto',
  nightModeEnabled: false, nightModeStart: '22:00', nightModeEnd: '07:00',
  nightModeTriggerTime: true, nightModeTriggerTheme: false, nightModeThemeDark: true,
  nightModeTriggerCharging: false, nightModeTriggerLowBattery: false,
  nightModeTargetMouse: true, nightModeTargetReceiver: false,
  telemetryDisabled: true,
  automaticUpdateChecks: true, automaticUpdateInstall: false, automaticPluginUpdateChecks: true,
  localAiAnalysisEnabled: false,
  localAiFeatures: { batteryUsage: true },
  batteryHistoryEnabled: true, batteryHistoryRetentionDays: 30, unusualDrainAlerts: false,
  language: 'auto',
};

// 声明式灯光 capability：仅 mouse 区域可写，receiver 不可写，
// 使 SettingsPage 内部计算的 supportsAnyLighting=true、supportsReceiverLighting=false。
const pluginCapabilities: PluginCapability[] = [
  {
    id: 'lighting',
    control: 'LightingZone',
    labelKey: 'capability.lighting',
    readOnly: false,
    metadata: {
      zones: [
        {
          id: 'mouse',
          labelKey: 'lighting.mouse',
          fields: [
            { id: 'effect', source: 'state.mouseLightEffect', mutation: 'set-mouse-lighting', param: 'effect', editor: 'modal-select', labelKey: 'receiverLighting.field.effect' },
          ],
        },
      ],
    },
  },
];
const writableMutations = ['set-mouse-lighting'];

describe('SettingsPage', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    startAutomaticAppUpdateCheckMock.mockReset();
    window.history.replaceState({}, '', '/');
  });

  it('describes automatic tray color as menu bar background matching on macOS', () => {
    window.history.replaceState({}, '', '/?platform=macos');
    render(<SettingsPage onNavigateAbout={vi.fn()} onThemeChange={vi.fn()} previewMode />);

    expect(screen.getByRole('option', { name: '跟随菜单栏背景' })).toBeInTheDocument();
    expect(screen.getByText('根据菜单栏的实际背景自动切换：深色背景用白色轮廓，浅色背景用黑色轮廓')).toBeInTheDocument();
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
    render(<SettingsPage onNavigateAbout={vi.fn()} onThemeChange={onThemeChange} pluginCapabilities={pluginCapabilities} writableMutations={writableMutations} />);

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
    expect(screen.queryByText('本地 AI 分析')).toBeNull();
    expect(screen.getByText('用于生成 24 小时和 10 天电量图表与本地用电摘要。')).toBeInTheDocument();
    const batteryAiToggle = screen.getByRole('switch', { name: '开启 AI 分析' });
    expect(batteryAiToggle).not.toBeChecked();
    fireEvent.click(batteryAiToggle);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_set', expect.objectContaining({
      settings: expect.objectContaining({
        localAiAnalysisEnabled: true,
        localAiFeatures: expect.objectContaining({ batteryUsage: true }),
        batteryHistoryEnabled: true,
      }),
    })));
    expect(screen.getByText('本地 AI 分析')).toBeInTheDocument();
    const nightModeToggle = screen.getByRole('switch', { name: '启用安静灯光' });
    expect(nightModeToggle).not.toBeDisabled();
    expect(nightModeToggle).not.toBeChecked();
    expect(screen.queryByLabelText('开始时间')).toBeNull();
    fireEvent.click(nightModeToggle);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_set', expect.objectContaining({
      settings: expect.objectContaining({ nightModeEnabled: true }),
    })));
    expect(await screen.findByRole('heading', { name: '触发场景（任一满足即关闭灯光）' })).toHaveClass('settings-subsection-title');
    expect(screen.getByRole('heading', { name: '灯光对象' })).toHaveClass('settings-subsection-title');
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
    const pluginAiToggle = screen.getByRole('switch', { name: '本地 AI 分析' });
    expect(pluginAiToggle).toBeChecked();
    fireEvent.click(screen.getByRole('button', { name: '检查插件更新' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('plugin_updates_check'));
    fireEvent.click(pluginAiToggle);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_set', expect.objectContaining({
      settings: expect.objectContaining({ localAiAnalysisEnabled: false }),
    })));
    fireEvent.click(screen.getByRole('button', { name: '设备' }));
    expect(screen.getByRole('switch', { name: '开启 AI 分析' })).not.toBeChecked();
    expect(screen.queryByText('本地 AI 分析')).toBeNull();
  });

  it('keeps the global AI engine separate from the battery feature scope', async () => {
    const scopedSettings: AppSettings = {
      ...settings,
      localAiAnalysisEnabled: true,
      localAiFeatures: { batteryUsage: false },
    };
    invokeMock.mockImplementation((command: string, payload?: { settings?: AppSettings }) => {
      if (command === 'settings_get') return Promise.resolve(scopedSettings);
      if (command === 'settings_set') return Promise.resolve(payload?.settings);
      if (command === 'autostart_state') return Promise.resolve(false);
      if (command === 'about_info') return Promise.resolve({ bundledPlugins: [] });
      return Promise.resolve(undefined);
    });

    render(<SettingsPage onNavigateAbout={vi.fn()} onThemeChange={vi.fn()} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_get'));

    fireEvent.click(screen.getByRole('button', { name: '插件' }));
    expect(screen.getByRole('switch', { name: '本地 AI 分析' })).toBeChecked();

    fireEvent.click(screen.getByRole('button', { name: '设备' }));
    expect(screen.getByRole('switch', { name: '开启 AI 分析' })).not.toBeChecked();
    expect(screen.queryByText('本地 AI 分析')).toBeNull();
  });

  it('updates and rolls back the local AI bundle as a single unit', async () => {
    let bundleVersion = '0.5.0';
    invokeMock.mockImplementation((command: string, payload?: { component?: string }) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'autostart_state') return Promise.resolve(false);
      if (command === 'about_info') return Promise.resolve({ bundledPlugins: [] });
      if (command === 'local_ai_status') return Promise.resolve({
        ready: true,
        bundleVersion: bundleVersion,
        runtimeVersion: '0.5.0',
        modelPackId: 'mira.battery.default',
        modelPackVersion: bundleVersion,
        rollbackAvailable: true,
      });
      if (command === 'local_ai_updates_check') return Promise.resolve([
        { component: 'bundle', currentVersion: bundleVersion, availableVersion: '0.6.0', updateAvailable: true },
      ]);
      if (command === 'local_ai_update_install') {
        expect(payload?.component).toBe('bundle');
        bundleVersion = '0.6.0';
        return Promise.resolve({ component: 'bundle', version: bundleVersion, previousVersion: '0.5.0', ready: true });
      }
      if (command === 'local_ai_update_rollback') return Promise.resolve({
        ready: true,
        bundleVersion: '0.5.0',
        runtimeVersion: '0.5.0',
        modelPackId: 'mira.battery.default',
        modelPackVersion: '0.5.0',
        rollbackAvailable: false,
      });
      return Promise.resolve(undefined);
    });

    render(<SettingsPage onNavigateAbout={vi.fn()} onThemeChange={vi.fn()} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('local_ai_status'));
    fireEvent.click(screen.getByRole('button', { name: '插件' }));
    expect(screen.getByText('本地 AI 引擎')).toBeInTheDocument();
    expect(screen.getByText('引擎可用')).toBeInTheDocument();
    const localAiItem = screen.getByText('本地 AI 引擎').closest('.plugin-item');
    expect(localAiItem).not.toBeNull();
    expect(localAiItem?.querySelector('.setting-hint')).toHaveTextContent('v0.5.0');
    expect(localAiItem?.querySelector('.plugin-meta .badge')).toHaveTextContent('签名已验证');
    expect(localAiItem?.querySelector('.plugin-meta .badge')).toHaveClass('badge-ok');
    expect(screen.queryByText('随 Mira 打包的 Runtime 与模型，可整体更新或回退；异常时自动回退到 Mira 原有算法。')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: '检查更新' }));
    expect(await screen.findByText('可更新至 v0.6.0')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '更新' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('local_ai_update_install', { component: 'bundle' }));
    await waitFor(() => expect(screen.queryByText('可更新至 v0.6.0')).toBeNull());

    fireEvent.click(screen.getByRole('button', { name: '回退' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('local_ai_update_rollback', { component: 'bundle' }));
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

  it('shows cached plugin update results after opening from an update notification', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'autostart_state') return Promise.resolve(false);
      if (command === 'about_info') return Promise.resolve({
        bundledPlugins: [{
          pluginId: 'mira.example', version: '0.2.0', asset: 'mira.example.mira-plugin',
          sha256: 'test', publisherKeyId: 'test', releaseTag: 'stable', bundleByDefault: false,
          signatureVerified: true, evidence: 'test', source: 'installed',
        }],
      });
      if (command === 'plugin_updates_check') return Promise.resolve([{
        pluginId: 'mira.example', currentVersion: '0.2.0', availableVersion: '0.3.0', updateAvailable: true,
      }]);
      return Promise.resolve(undefined);
    });
    await checkForPluginUpdates();
    const pluginUpdateCheckCalls = invokeMock.mock.calls.filter(([command]) => command === 'plugin_updates_check').length;

    render(<SettingsPage onNavigateAbout={vi.fn()} onThemeChange={vi.fn()} focusPluginUpdateToken={1} />);

    expect(await screen.findByText('可更新至 v0.3.0')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '更新插件' })).toBeInTheDocument();
    expect(invokeMock.mock.calls.filter(([command]) => command === 'plugin_updates_check')).toHaveLength(pluginUpdateCheckCalls);
  });
});
