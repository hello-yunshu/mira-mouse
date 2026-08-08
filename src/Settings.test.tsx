// SPDX-License-Identifier: AGPL-3.0-or-later
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SettingsPage } from './Settings';
import type { AppSettings, PluginCapability } from './types';
import { checkForPluginUpdates } from './plugin-updater';
import i18n from './i18n';

const { invokeMock, startAutomaticAppUpdateCheckMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  startAutomaticAppUpdateCheckMock: vi.fn(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('./updater', () => ({ startAutomaticAppUpdateCheck: startAutomaticAppUpdateCheckMock }));

const settings: AppSettings = {
  theme: 'system', autostart: false, startHidden: true, lowBatteryThreshold: 20,
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
  beforeEach(async () => {
    invokeMock.mockReset();
    startAutomaticAppUpdateCheckMock.mockReset();
    await i18n.changeLanguage('zh-CN');
    window.history.replaceState({}, '', '/');
  });

  it('describes automatic tray color as menu bar background matching on macOS', () => {
    window.history.replaceState({}, '', '/?platform=macos');
    render(<SettingsPage onNavigateAbout={vi.fn()} onThemeChange={vi.fn()} previewMode />);

    expect(screen.getByRole('option', { name: '跟随菜单栏背景' })).toBeInTheDocument();
    expect(screen.getByText('根据菜单栏的实际背景自动切换：深色背景用白色轮廓，浅色背景用黑色轮廓')).toBeInTheDocument();
  });

  it('enables the Windows battery icon toggle and saves only trayShowBatteryIcon', async () => {
    window.history.replaceState({}, '', '/?platform=windows');
    const windowsSettings = {
      ...settings,
      trayShowBatteryTitle: false,
      trayShowBatteryIcon: false,
    };
    invokeMock.mockImplementation((command: string, payload?: { settings?: AppSettings }) => {
      if (command === 'settings_get') return Promise.resolve(windowsSettings);
      if (command === 'settings_set') return Promise.resolve(payload?.settings);
      if (command === 'autostart_state') return Promise.resolve(false);
      if (command === 'about_info') return Promise.resolve({ bundledPlugins: [] });
      return Promise.resolve(undefined);
    });

    render(<SettingsPage onNavigateAbout={vi.fn()} onThemeChange={vi.fn()} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_get'));

    const toggle = screen.getByRole('switch', { name: '显示电量百分比' });
    expect(toggle).not.toBeDisabled();
    expect(toggle).not.toBeChecked();
    expect(screen.getByText('在鼠标状态图标旁显示独立的数字电量图标')).toBeInTheDocument();

    fireEvent.click(toggle);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_set', {
      settings: expect.objectContaining({
        trayShowBatteryIcon: true,
        trayShowBatteryTitle: false,
      }),
    }));
  });

  it.each(['macos', 'linux'])('keeps trayShowBatteryTitle on %s', async (platform) => {
    window.history.replaceState({}, '', `/?platform=${platform}`);
    const platformSettings = {
      ...settings,
      trayShowBatteryTitle: false,
      trayShowBatteryIcon: false,
    };
    invokeMock.mockImplementation((command: string, payload?: { settings?: AppSettings }) => {
      if (command === 'settings_get') return Promise.resolve(platformSettings);
      if (command === 'settings_set') return Promise.resolve(payload?.settings);
      if (command === 'autostart_state') return Promise.resolve(false);
      if (command === 'about_info') return Promise.resolve({ bundledPlugins: [] });
      return Promise.resolve(undefined);
    });

    render(<SettingsPage onNavigateAbout={vi.fn()} onThemeChange={vi.fn()} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_get'));
    fireEvent.click(screen.getByRole('switch', { name: '显示电量百分比' }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_set', {
      settings: expect.objectContaining({
        trayShowBatteryTitle: true,
        trayShowBatteryIcon: false,
      }),
    }));
  });

  it('shows the English Windows battery icon hint', async () => {
    await i18n.changeLanguage('en');
    window.history.replaceState({}, '', '/?platform=windows');
    render(<SettingsPage onNavigateAbout={vi.fn()} onThemeChange={vi.fn()} previewMode />);

    expect(screen.getByRole('switch', { name: 'Show battery percentage' })).not.toBeDisabled();
    expect(screen.getByText('Show a separate numeric battery icon next to the mouse status icon')).toBeInTheDocument();
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
    fireEvent.click(screen.getByRole('switch', { name: '标题显示接收器电量' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_set', expect.objectContaining({
      settings: expect.objectContaining({ trayIncludeReceiverBattery: true }),
    })));

    fireEvent.click(screen.getByRole('button', { name: '设备' }));
    expect(await screen.findByText('用于生成 24 小时和 10 天电量图表与本地用电摘要。')).toBeInTheDocument();
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
    await waitFor(() => expect(screen.getByRole('switch', { name: '禁用遥测' })).toBeDisabled());
    fireEvent.click(screen.getByRole('button', { name: '插件' }));
    const pluginAiToggle = await screen.findByRole('switch', { name: '本地 AI 分析' });
    expect(pluginAiToggle).toBeChecked();
    expect(screen.getByRole('button', { name: '检查更新' }).closest('.plugin-update-actions')).toHaveClass('align-end');
    expect(screen.getByRole('button', { name: '检查插件更新' }).closest('.plugin-update-actions')).toHaveClass('align-end');
    fireEvent.click(screen.getByRole('button', { name: '检查插件更新' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('plugin_updates_check'));
    fireEvent.click(pluginAiToggle);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_set', expect.objectContaining({
      settings: expect.objectContaining({ localAiAnalysisEnabled: false }),
    })));
    fireEvent.click(screen.getByRole('button', { name: '设备' }));
    await waitFor(() => expect(screen.getByRole('switch', { name: '开启 AI 分析' })).not.toBeChecked());
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
    expect(await screen.findByRole('switch', { name: '本地 AI 分析' })).toBeChecked();

    fireEvent.click(screen.getByRole('button', { name: '设备' }));
    await waitFor(() => {
      expect(screen.getByRole('switch', { name: '开启 AI 分析' })).not.toBeChecked();
      expect(screen.queryByText('本地 AI 分析')).toBeNull();
    });
  });

  it('updates and rolls back the local AI bundle as a single unit', async () => {
    let runtimeVersion = '0.5.0';
    invokeMock.mockImplementation((command: string, payload?: { component?: string }) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'autostart_state') return Promise.resolve(false);
      if (command === 'about_info') return Promise.resolve({ bundledPlugins: [] });
      if (command === 'local_ai_status') return Promise.resolve({
        ready: true,
        bundleVersion: runtimeVersion,
        runtimeVersion: runtimeVersion,
        modelPackId: 'mira.battery.default',
        modelPackVersion: '0.4.0',
        handlerId: 'mira.battery.handler',
        handlerVersion: '0.3.0',
        rollbackAvailable: true,
      });
      if (command === 'local_ai_updates_check') return Promise.resolve([
        { component: 'runtime', currentVersion: runtimeVersion, availableVersion: '0.6.0', updateAvailable: true },
        { component: 'model', currentVersion: '0.4.0', availableVersion: '0.4.0', updateAvailable: false },
        { component: 'handler', currentVersion: '0.3.0', availableVersion: '0.3.0', updateAvailable: false },
      ]);
      if (command === 'local_ai_update_install') {
        expect(payload?.component).toBe('bundle');
        runtimeVersion = '0.6.0';
        return Promise.resolve({ component: 'bundle', version: runtimeVersion, previousVersion: '0.5.0', ready: true });
      }
      if (command === 'local_ai_update_rollback') return Promise.resolve({
        ready: true,
        bundleVersion: '0.5.0',
        runtimeVersion: '0.5.0',
        modelPackId: 'mira.battery.default',
        modelPackVersion: '0.4.0',
        handlerId: 'mira.battery.handler',
        handlerVersion: '0.3.0',
        rollbackAvailable: false,
      });
      return Promise.resolve(undefined);
    });

    render(<SettingsPage onNavigateAbout={vi.fn()} onThemeChange={vi.fn()} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('local_ai_status'));
    fireEvent.click(screen.getByRole('button', { name: '插件' }));
    expect(await screen.findByText('本地 AI 引擎')).toBeInTheDocument();
    expect(screen.getByText('本地 AI 模型')).toBeInTheDocument();
    expect(screen.getByText('本地 AI 处理器')).toBeInTheDocument();
    expect(screen.getByText('引擎可用')).toBeInTheDocument();
    const runtimeItem = screen.getByText('本地 AI 引擎').closest('.plugin-item');
    expect(runtimeItem).not.toBeNull();
    expect(runtimeItem?.querySelector('.setting-hint')).toHaveTextContent('v0.5.0');
    expect(runtimeItem?.querySelector('.plugin-meta .badge')).toHaveTextContent('签名已验证');
    expect(runtimeItem?.querySelector('.plugin-meta .badge')).toHaveClass('badge-ok');

    // 更新成功后回退按钮不应显示（仅出错时显示）。
    expect(screen.queryByRole('button', { name: '回退' })).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: '检查更新' }));
    expect(await screen.findByText('可更新至 v0.6.0')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '更新' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('local_ai_update_install', { component: 'bundle' }));
    await waitFor(() => expect(screen.queryByText('可更新至 v0.6.0')).toBeNull());

    // 更新成功后回退按钮仍不应显示。
    expect(screen.queryByRole('button', { name: '回退' })).toBeNull();
  });

  it('shows the rollback button only when local AI update errors', async () => {
    const runtimeVersion = '0.5.0';
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'autostart_state') return Promise.resolve(false);
      if (command === 'about_info') return Promise.resolve({ bundledPlugins: [] });
      if (command === 'local_ai_status') return Promise.resolve({
        ready: true,
        bundleVersion: runtimeVersion,
        runtimeVersion: runtimeVersion,
        modelPackId: 'mira.battery.default',
        modelPackVersion: '0.4.0',
        handlerId: 'mira.battery.handler',
        handlerVersion: '0.3.1',
        rollbackAvailable: true,
        previousVersion: '0.3.0',
      });
      if (command === 'local_ai_updates_check') return Promise.resolve([
        { component: 'runtime', currentVersion: runtimeVersion, availableVersion: '0.6.0', updateAvailable: true },
        { component: 'model', currentVersion: '0.4.0', availableVersion: '0.4.0', updateAvailable: false },
        { component: 'handler', currentVersion: '0.3.1', availableVersion: '0.3.1', updateAvailable: false },
      ]);
      if (command === 'local_ai_update_install') {
        return Promise.reject(new Error('install failed'));
      }
      if (command === 'local_ai_update_rollback') return Promise.resolve({
        ready: true,
        bundleVersion: '0.5.0',
        runtimeVersion: '0.5.0',
        modelPackId: 'mira.battery.default',
        modelPackVersion: '0.4.0',
        handlerId: 'mira.battery.handler',
        handlerVersion: '0.3.1',
        rollbackAvailable: false,
      });
      return Promise.resolve(undefined);
    });

    render(<SettingsPage onNavigateAbout={vi.fn()} onThemeChange={vi.fn()} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('local_ai_status'));
    fireEvent.click(screen.getByRole('button', { name: '插件' }));
    expect(await screen.findByText('本地 AI 引擎')).toBeInTheDocument();

    // 出错前回退按钮不显示。
    expect(screen.queryByRole('button', { name: '回退' })).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: '检查更新' }));
    expect(await screen.findByText('可更新至 v0.6.0')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '更新' }));
    // 安装失败后回退按钮出现，且上一版本号显示在按钮旁。
    await waitFor(() => expect(screen.getByRole('button', { name: '回退' })).toBeInTheDocument());
    expect(screen.getByText('v0.3.0')).toBeInTheDocument();

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

  it('serializes rapid full-object settings writes without losing newer edits', async () => {
    const pendingSaves: Array<{
      settings: AppSettings;
      resolve: (value: AppSettings) => void;
    }> = [];
    invokeMock.mockImplementation((command: string, payload?: { settings?: AppSettings }) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'settings_set' && payload?.settings) {
        return new Promise<AppSettings>((resolve) => {
          pendingSaves.push({ settings: payload.settings as AppSettings, resolve });
        });
      }
      if (command === 'autostart_state') return Promise.resolve(false);
      if (command === 'about_info') return Promise.resolve({ bundledPlugins: [], updaterActive: true });
      return Promise.resolve(undefined);
    });

    render(<SettingsPage onNavigateAbout={vi.fn()} onThemeChange={vi.fn()} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_get'));

    fireEvent.click(screen.getByRole('switch', { name: '菜单显示连接状态' }));
    fireEvent.change(screen.getByRole('combobox', { name: '托盘图标颜色' }), { target: { value: 'black' } });

    await waitFor(() => expect(pendingSaves).toHaveLength(1));
    expect(invokeMock.mock.calls.filter(([command]) => command === 'settings_set')).toHaveLength(1);
    expect(pendingSaves[0].settings.trayShowConnection).toBe(false);

    await act(async () => pendingSaves[0].resolve(pendingSaves[0].settings));
    await waitFor(() => expect(pendingSaves).toHaveLength(2));
    expect(pendingSaves[1].settings).toEqual(expect.objectContaining({
      trayShowConnection: false,
      trayIconColor: 'black',
    }));

    await act(async () => pendingSaves[1].resolve(pendingSaves[1].settings));
    await waitFor(() => expect(screen.getByRole('combobox', { name: '托盘图标颜色' })).toHaveValue('black'));
    expect(screen.getByRole('switch', { name: '菜单显示连接状态' })).not.toBeChecked();
  });

  it('merges an edit made before settings hydration without overwriting persisted preferences', async () => {
    let resolveSettingsGet!: (value: AppSettings) => void;
    const settingsGet = new Promise<AppSettings>((resolve) => {
      resolveSettingsGet = resolve;
    });
    invokeMock.mockImplementation((command: string, payload?: { settings?: AppSettings }) => {
      if (command === 'settings_get') return settingsGet;
      if (command === 'settings_set') return Promise.resolve(payload?.settings);
      if (command === 'autostart_state') return Promise.resolve(false);
      if (command === 'about_info') return Promise.resolve({ bundledPlugins: [], updaterActive: true });
      return Promise.resolve(undefined);
    });

    render(<SettingsPage onNavigateAbout={vi.fn()} onThemeChange={vi.fn()} />);
    fireEvent.click(screen.getByRole('switch', { name: '菜单显示连接状态' }));
    expect(invokeMock.mock.calls.filter(([command]) => command === 'settings_set')).toHaveLength(0);

    const persisted: AppSettings = {
      ...settings,
      startHidden: false,
      trayIconColor: 'white',
      batteryHistoryRetentionDays: 90,
    };
    await act(async () => resolveSettingsGet(persisted));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_set', {
      settings: expect.objectContaining({
        startHidden: false,
        trayIconColor: 'white',
        batteryHistoryRetentionDays: 90,
        trayShowConnection: false,
      }),
    }));
  });

  it('ignores a stale initial autostart read after the user changes the setting', async () => {
    let resolveAutostartState!: (value: boolean) => void;
    const autostartState = new Promise<boolean>((resolve) => {
      resolveAutostartState = resolve;
    });
    invokeMock.mockImplementation((command: string, payload?: { settings?: AppSettings }) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'settings_set') return Promise.resolve(payload?.settings);
      if (command === 'autostart_state') return autostartState;
      if (command === 'set_autostart') return Promise.resolve(undefined);
      if (command === 'about_info') return Promise.resolve({ bundledPlugins: [], updaterActive: true });
      return Promise.resolve(undefined);
    });

    render(<SettingsPage onNavigateAbout={vi.fn()} onThemeChange={vi.fn()} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_get'));
    const autostartToggle = screen.getByRole('switch', { name: '开机自动启动' });
    fireEvent.click(autostartToggle);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('set_autostart', { enabled: true }));
    await waitFor(() => expect(autostartToggle).toBeChecked());

    await act(async () => resolveAutostartState(false));
    expect(autostartToggle).toBeChecked();
  });

  it('resyncs updater behavior when a failed save is replaced by a newer full-object save', async () => {
    const pendingSaves: Array<{
      settings: AppSettings;
      resolve: (value: AppSettings) => void;
      reject: (reason: Error) => void;
    }> = [];
    invokeMock.mockImplementation((command: string, payload?: { settings?: AppSettings }) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'settings_set' && payload?.settings) {
        return new Promise<AppSettings>((resolve, reject) => {
          pendingSaves.push({ settings: payload.settings as AppSettings, resolve, reject });
        });
      }
      if (command === 'autostart_state') return Promise.resolve(false);
      if (command === 'about_info') return Promise.resolve({ bundledPlugins: [], updaterActive: true });
      return Promise.resolve(undefined);
    });

    render(<SettingsPage onNavigateAbout={vi.fn()} onThemeChange={vi.fn()} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_get'));

    fireEvent.click(screen.getByRole('switch', { name: '自动检查 Mira 更新' }));
    fireEvent.click(screen.getByRole('switch', { name: '菜单显示连接状态' }));
    await waitFor(() => expect(pendingSaves).toHaveLength(1));

    await act(async () => pendingSaves[0].reject(new Error('transient write failure')));
    await waitFor(() => expect(pendingSaves).toHaveLength(2));
    expect(pendingSaves[1].settings).toEqual(expect.objectContaining({
      automaticUpdateChecks: false,
      trayShowConnection: false,
    }));

    await act(async () => pendingSaves[1].resolve(pendingSaves[1].settings));
    await waitFor(() => expect(startAutomaticAppUpdateCheckMock).toHaveBeenCalledWith(false));
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
