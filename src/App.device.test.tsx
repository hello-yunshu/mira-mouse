// SPDX-License-Identifier: AGPL-3.0-or-later
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';
import App from './App';
import { themeAccent } from './theme';
import type { AppSettings, DeviceSnapshot } from './types';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

beforeAll(() => Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} }));
afterAll(() => Reflect.deleteProperty(window, '__TAURI_INTERNALS__'));

const settings: AppSettings = {
  theme: 'light', autostart: false, startHidden: false, lowBatteryThreshold: 20,
  trayShowBatteryTitle: true, trayIncludeReceiverBattery: false, trayShowConnection: true,
  trayIconColor: 'white',
  nightModeEnabled: false, nightModeStart: '22:00', nightModeEnd: '07:00',
  nightModeTriggerTime: true, nightModeTriggerTheme: false, nightModeThemeDark: true,
  nightModeTriggerCharging: false, nightModeTriggerLowBattery: false,
  nightModeTargetMouse: true, nightModeTargetReceiver: false,
  refreshIntervalSeconds: 5, telemetryDisabled: true,
  automaticUpdateChecks: true, automaticUpdateInstall: false, automaticPluginUpdateChecks: true,
  language: 'auto',
};

const snapshot: DeviceSnapshot = {
  displayName: 'AM INFINITY 8K MOUSE', connection: 'wireless', batteryPercent: 76,
  charging: false, dpi: 1600, pollingRateHz: 1000, profile: '1',
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
    settings: { pollingRate: 1000, motionSync: true, angleSnap: false, liftCutOff: 2, wirelessSleepValue: 60, bluetoothSleepValue: 600, mouseLightStartColor: '#112233', mouseLightEndColor: '#112233', mouseLightEnabled: true },
    receiverLighting: { effect: 3, effectName: '霓虹', speed: 3, speedLabel: '慢', brightness: 1, brightnessLabel: '暗', option: 7, optionName: '自定义', color: '#AABBCC' },
    firmwareUsb: { versionRaw: 258 },
    buttonMappings: { '0x00': [1, 0, 0, 0] },
  },
  pluginCapabilities: [
    { id: 'dpi', control: 'DpiStages', labelKey: 'capability.dpi', readOnly: false, metadata: { label: 'DPI', section: 'control', mutations: { select: 'set-dpi-stage', value: 'set-dpi-value' } } },
    { id: 'polling-rate', control: 'Select', labelKey: 'capability.polling-rate', readOnly: false, metadata: { label: '回报率', section: 'control', source: 'pollingRate', mutation: 'set-polling-rate', options: [{ value: 1000, label: '1000 Hz' }], summary: [{ label: '运动同步', source: 'capabilities.settings.motionSync' }, { label: '抬升高度', source: 'capabilities.settings.liftCutOff' }] } },
    {
      id: 'sleep-time', control: 'Number', labelKey: 'capability.sleep-time', readOnly: false,
      metadata: {
        label: '休眠时间', section: 'status', status: true,
        bindings: [
          { when: { path: 'connection', eq: 'wireless' }, label: '插件声明的无线休眠', source: 'capabilities.settings.wirelessSleepValue', mutation: 'set-wireless-sleep-time', param: 'seconds' },
        ],
      },
    },
    { id: 'profile', control: 'ReadOnlyValue', labelKey: 'capability.profile', readOnly: true, metadata: { label: '配置文件', section: 'status', status: true, source: 'profile' } },
    { id: 'lighting', control: 'LightingZone', labelKey: 'capability.lighting', readOnly: false, metadata: { label: '灯光', section: 'control', status: true, lightingRole: { mouse: 'set-mouse-lighting', receiver: 'set-receiver-lighting' } } },
  ],
  writableMutations: ['set-dpi-stage', 'set-wireless-sleep-time', 'set-mouse-lighting'],
};

function entries(...snapshots: DeviceSnapshot[]) {
  return snapshots.map((item, index) => ({
    deviceKey: `device-${index}`,
    snapshot: item,
    selected: index === 0,
  }));
}

describe('real device snapshot mapping', () => {
  it('switches between multiple connected mouse snapshots from the dashboard title', async () => {
    const firstSnapshot: DeviceSnapshot = {
      displayName: 'First Mouse', connection: 'wireless', charging: false, batteries: [],
      capabilities: {}, pluginCapabilities: [], writableMutations: [], evidence: 'hardware-verified',
    };
    const secondSnapshot: DeviceSnapshot = {
      displayName: 'Second Mouse', connection: 'usb', charging: false, batteries: [],
      capabilities: {}, pluginCapabilities: [], writableMutations: [], evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string, args?: { deviceKey?: string }) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(firstSnapshot, secondSnapshot));
      if (command === 'device_select' && args?.deviceKey === 'device-1') return Promise.resolve(secondSnapshot);
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect(await screen.findByRole('heading', { name: 'First Mouse' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '切换鼠标' }));
    fireEvent.click(screen.getByText('Second Mouse').closest('button')!);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('device_select', { deviceKey: 'device-1' }));
    expect(await screen.findByRole('heading', { name: 'Second Mouse' })).toBeInTheDocument();
  });

  it('keeps plugin-declared dashboard rows within the host layout limit', async () => {
    const capabilities = Array.from({ length: 7 }, (_, index) => ({
      id: `control-${index}`,
      control: 'ReadOnlyValue' as const,
      labelKey: `Control ${index + 1}`,
      readOnly: true,
      placements: [
        { region: 'control' as const, group: `group-${index}`, order: index, span: 1, icon: 'info' },
        { region: 'status' as const, order: index, span: 1, icon: 'info' },
      ],
      metadata: { source: 'name' },
    }));
    const gridSnapshot: DeviceSnapshot = {
      displayName: 'Grid Mouse', connection: 'virtual', charging: false, batteries: [],
      capabilities: {}, pluginCapabilities: capabilities, writableMutations: [], evidence: 'fixture-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(gridSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect(await screen.findByRole('heading', { name: 'Grid Mouse' })).toBeInTheDocument();
    expect(screen.getByRole('tablist', { name: '设备控制' })).toHaveStyle({ gridTemplateColumns: 'repeat(6, minmax(0, 1fr))' });
    expect(screen.getByRole('region', { name: '设备状态' })).toHaveStyle({ gridTemplateColumns: 'repeat(6, minmax(0, 1fr))' });
    expect(screen.queryByRole('tab', { name: 'Control 7' })).not.toBeInTheDocument();
  });

  it('renders plugin-declared controls and status without a brand-specific branch', async () => {
    const pluginSnapshot: DeviceSnapshot = {
      displayName: 'Declarative Mouse', connection: 'wireless', batteryPercent: 80,
      charging: false, batteries: [{ id: 'mouse', label: '鼠标', percentage: 80, charging: false }],
      dpi: 1600, dpiStages: [{ value: 1600, color: '#9a8bd0', active: true, enabled: true }],
      capabilities: { controlMode: { mode: 1, modeName: 'onboard' }, dpi: { dpiValue: 1600 } },
      pluginCapabilities: [
        {
          id: 'control-mode', control: 'Segmented', labelKey: 'capability.control-mode', readOnly: false,
          placements: [
            { region: 'control', group: 'configuration', order: 20, span: 1, icon: 'profile' },
            { region: 'status', order: 10, span: 2, icon: 'profile' },
          ],
          metadata: {
            label: '配置控制', section: 'control', status: true,
            source: 'capabilities.controlMode.mode', mutation: 'set-control-mode', param: 'mode',
            options: [{ value: 1, label: '板载' }, { value: 2, label: '软件' }],
            summary: [
              { label: '连接', source: 'connection' },
              { label: '电量', source: 'battery', unit: '%' },
            ],
          },
        },
        {
          id: 'dpi', control: 'DpiStages', labelKey: 'capability.dpi', readOnly: false,
          placements: [{ region: 'control', group: 'performance', order: 10, span: 1, icon: 'gauge' }],
          metadata: { mutations: { value: 'set-dpi-value' } },
        },
      ],
      writableMutations: ['set-control-mode', 'set-dpi-value'], evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string, args?: { mutation?: string }) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(pluginSnapshot));
      if (command === 'device_mutate' && args?.mutation === 'set-control-mode') return Promise.resolve({
        ...pluginSnapshot,
        capabilities: { ...pluginSnapshot.capabilities, controlMode: { mode: 2, modeName: 'host' } },
      });
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect((await screen.findAllByRole('tab')).map((tab) => tab.textContent)).toEqual(['DPI', '配置控制']);
    expect(screen.getByRole('tab', { name: '配置控制' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'DPI' })).toBeInTheDocument();
    expect(screen.getByRole('region', { name: '设备状态' })).toHaveTextContent('配置控制板载');
    expect(screen.getByRole('region', { name: '设备状态' })).toHaveStyle({ gridTemplateColumns: 'repeat(1, minmax(0, 1fr))' });
    expect(screen.getByRole('region', { name: '设备状态' })).toHaveAttribute('data-status-count', '1');
    expect(screen.getByRole('region', { name: '设备状态' }).firstElementChild).not.toHaveStyle({ gridColumn: 'span 2' });
    fireEvent.click(screen.getByRole('tab', { name: '配置控制' }));
    expect(screen.getByLabelText('设备摘要')).toHaveTextContent('连接无线电量80 %');
    expect(screen.getByLabelText('设备摘要').children).toHaveLength(2);
    fireEvent.click(screen.getByRole('button', { name: '软件' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('device_mutate', {
      mutation: 'set-control-mode', params: { mode: 2 },
    }));
    expect(screen.getByRole('region', { name: '设备状态' })).toHaveTextContent('配置控制软件');
  });

  it('does not use receiver lighting as the app accent or mouse lighting color', async () => {
    const receiverOnlySnapshot: DeviceSnapshot = {
      displayName: 'Receiver-lit Mouse',
      connection: 'wireless',
      batteryPercent: 80,
      charging: false,
      batteries: [{ id: 'mouse', label: '鼠标', percentage: 80, charging: false }],
      dpi: 1600,
      dpiStages: [{ value: 1600, color: '#9a8bd0', active: true, enabled: true }],
      confirmedLightColor: '#00FF00',
      capabilities: {
        receiverLighting: { effect: 1, effectName: '常亮', speed: 2, brightness: 3, option: 7, color: '#00FF00' },
        receiverLightSwitch: { enabled: false },
      },
      pluginCapabilities: [
        {
          id: 'lighting',
          control: 'LightingZone',
          labelKey: 'capability.lighting',
          readOnly: false,
          placements: [
            { region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' },
            { region: 'status', order: 30, span: 1, icon: 'lightbulb' },
          ],
          metadata: { label: '灯光', section: 'control', status: true, mutations: { mouse: 'set-mouse-lighting', receiver: 'set-receiver-lighting' } },
        },
      ],
      writableMutations: ['set-mouse-lighting', 'set-receiver-lighting'],
      evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(receiverOnlySnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect(await screen.findByText('Receiver-lit Mouse')).toBeInTheDocument();
    await waitFor(() => expect(document.documentElement.style.getPropertyValue('--accent')).toBe(themeAccent('#9a8bd0')));
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    expect(screen.getByRole('button', { name: '颜色未报告' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('tab', { name: '接收器灯光' }));
    const receiverColor = screen.getByText('#00FF00');
    expect(screen.getByRole('button', { name: '颜色#00FF00' })).toBeInTheDocument();
    expect(receiverColor).toHaveClass('color-value');
    expect(receiverColor).toHaveStyle('--value-color: #00FF00');
  });

  it('uses receiver lighting options to label the off effect', async () => {
    const receiverOffSnapshot: DeviceSnapshot = {
      ...snapshot,
      capabilities: {
        ...snapshot.capabilities,
        receiverLighting: {
          effect: 0,
          effectName: null,
          enabled: false,
          speed: 3,
          brightness: 1,
          option: 7,
          optionName: '自定义',
          color: '#AABBCC',
        },
      },
      pluginCapabilities: (snapshot.pluginCapabilities ?? []).map((capability) => capability.id === 'lighting'
        ? {
            ...capability,
            metadata: {
              ...capability.metadata,
              receiverLightingOptions: {
                effect: [
                  { value: 0, labelKey: 'receiverLighting.effect.off' },
                  { value: 1, labelKey: 'receiverLighting.effect.fixed' },
                ],
              },
            },
          }
        : capability),
      writableMutations: [...(snapshot.writableMutations ?? []), 'set-receiver-lighting'],
      pluginId: 'mira.amaster',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(receiverOffSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect(await screen.findByText('AM INFINITY 8K MOUSE')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    fireEvent.click(screen.getByRole('tab', { name: '接收器灯光' }));
    expect(screen.getByRole('button', { name: /灯效已关闭/ })).toBeInTheDocument();
    expect(screen.queryByText('灯效 0')).not.toBeInTheDocument();
  });

  it('renders Logitech HID++ pointer speed from plugin metadata', async () => {
    const logitechSnapshot: DeviceSnapshot = {
      displayName: 'HID++ Mouse', connection: 'wireless', batteryPercent: 82,
      charging: false, batteries: [{ id: 'mouse', label: '鼠标', percentage: 82, charging: false }],
      dpi: 1600, dpiStages: [{ value: 1600, color: '#9a8bd0', active: true, enabled: true }],
      capabilities: {
        pointerSpeed: { speedRaw: 256 },
      },
      pluginCapabilities: [
        {
          id: 'pointer-speed', control: 'Number', labelKey: 'capability.pointer-speed', readOnly: false,
          placements: [{ region: 'control', group: 'performance', order: 15, span: 1, icon: 'gauge' }],
          metadata: {
            label: '指针速度',
            source: 'capabilities.pointerSpeed.speedRaw',
            mutation: 'set-pointer-speed',
            param: 'speed',
            min: 46,
            max: 511,
            step: 1,
            description: '使用 HID++ 0x2205 Pointer Speed 写入设备指针速度。',
          },
        },
      ],
      writableMutations: ['set-pointer-speed'], evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string, args?: { mutation?: string; params?: Record<string, unknown> }) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(logitechSnapshot));
      if (command === 'device_mutate' && args?.mutation === 'set-pointer-speed') return Promise.resolve({
        ...logitechSnapshot,
        capabilities: { ...logitechSnapshot.capabilities, pointerSpeed: { speedRaw: args.params?.speed } },
      });
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect(await screen.findByText('HID++ Mouse')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('tab', { name: '指针速度' }));
    expect(screen.queryByText(/使用 HID\+\+ 0x2205/)).not.toBeInTheDocument();
    expect(screen.queryByRole('spinbutton', { name: '指针速度' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '应用' })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '指针速度：256，点击编辑' }));
    fireEvent.change(screen.getByRole('spinbutton', { name: '指针速度' }), { target: { value: '300' } });
    fireEvent.click(screen.getByRole('button', { name: '应用' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('device_mutate', {
      mutation: 'set-pointer-speed', params: { speed: 300 },
    }));
    expect(screen.queryByRole('tab', { name: /RGB/ })).not.toBeInTheDocument();
  });

  it('keeps Logitech generic number and select controls collapsed until editing', async () => {
    const logitechSnapshot: DeviceSnapshot = {
      displayName: 'Logitech Config Mouse',
      connection: 'wireless',
      charging: false,
      batteries: [],
      pollingRateHz: 1000,
      capabilities: {
        controlMode: { mode: 2 },
        profileMgmtCurrent: { profileIndex: 1 },
        pointerSpeed: { speedRaw: 256 },
      },
      pluginCapabilities: [
        {
          id: 'control-mode', control: 'Segmented', labelKey: 'capability.control-mode', readOnly: false,
          placements: [{ region: 'control', group: 'configuration', order: 5, span: 1, icon: 'settings' }],
          metadata: {
            label: '配置控制', source: 'capabilities.controlMode.mode', mutation: 'set-control-mode', param: 'mode',
            options: [{ value: 1, label: '板载' }, { value: 2, label: '软件' }],
          },
        },
        {
          id: 'profile-mgmt-current', control: 'Number', labelKey: 'capability.profile-mgmt-current', readOnly: false,
          placements: [{ region: 'control', group: 'configuration', order: 6, span: 1, icon: 'profile' }],
          metadata: {
            label: '当前配置文件', source: 'capabilities.profileMgmtCurrent.profileIndex', mutation: 'set-profile-mgmt-current', param: 'profileIndex',
            min: 0, max: 15, step: 1,
          },
        },
        {
          id: 'pointer-speed', control: 'Number', labelKey: 'capability.pointer-speed', readOnly: false,
          placements: [{ region: 'control', group: 'performance', order: 15, span: 1, icon: 'gauge' }],
          metadata: {
            label: '指针速度', source: 'capabilities.pointerSpeed.speedRaw', mutation: 'set-pointer-speed', param: 'speed',
            min: 46, max: 511, step: 1,
          },
        },
        {
          id: 'polling-rate', control: 'Select', labelKey: 'capability.polling-rate', readOnly: false,
          placements: [{ region: 'control', group: 'polling', order: 20, span: 1, icon: 'wave' }],
          metadata: {
            label: '回报率', source: 'pollingRate', mutation: 'set-polling-rate', param: 'rate',
            options: [{ value: 500, label: '500 Hz' }, { value: 1000, label: '1000 Hz' }],
          },
        },
      ],
      writableMutations: ['set-control-mode', 'set-profile-mgmt-current', 'set-pointer-speed', 'set-polling-rate'],
      evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(logitechSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect(await screen.findByText('Logitech Config Mouse')).toBeInTheDocument();
    expect(screen.queryByRole('spinbutton', { name: '当前配置文件' })).not.toBeInTheDocument();
    expect(screen.queryByRole('combobox', { name: '回报率' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '应用' })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '当前配置文件：1，点击编辑' }));
    expect(screen.getByRole('spinbutton', { name: '当前配置文件' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '取消' }));

    fireEvent.click(screen.getByRole('tab', { name: '指针速度' }));
    expect(screen.queryByRole('spinbutton', { name: '指针速度' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '应用' })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '指针速度：256，点击编辑' }));
    expect(screen.getByRole('spinbutton', { name: '指针速度' })).toHaveAttribute('min', '46');
    expect(screen.getByRole('spinbutton', { name: '指针速度' })).toHaveAttribute('max', '511');
    expect(screen.getByRole('spinbutton', { name: '指针速度' })).toHaveAttribute('step', '1');
    fireEvent.click(screen.getByRole('button', { name: '取消' }));

    fireEvent.click(screen.getByRole('tab', { name: '回报率' }));
    expect(screen.queryByRole('combobox', { name: '回报率' })).not.toBeInTheDocument();
    fireEvent.click(screen.getByLabelText('当前回报率：1000 Hz，点击编辑'));
    expect(screen.getByRole('combobox', { name: '回报率' })).toBeInTheDocument();
  });

  it('hides Logitech numeric controls when the device did not report their value', async () => {
    const logitechSnapshot: DeviceSnapshot = {
      displayName: 'Logitech Partial Mouse',
      connection: 'wireless',
      charging: false,
      batteries: [],
      capabilities: {},
      pluginCapabilities: [
        {
          id: 'pointer-speed', control: 'Number', labelKey: 'capability.pointer-speed', readOnly: false,
          placements: [{ region: 'control', group: 'performance', order: 15, span: 1, icon: 'gauge' }],
          metadata: {
            label: '指针速度', source: 'capabilities.pointerSpeed.speedRaw', mutation: 'set-pointer-speed', param: 'speed',
            min: 46, max: 511, step: 1,
          },
        },
      ],
      writableMutations: ['set-pointer-speed'],
      evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(logitechSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect(await screen.findByText('Logitech Partial Mouse')).toBeInTheDocument();
    expect(screen.queryByRole('tab', { name: '指针速度' })).not.toBeInTheDocument();
    expect(screen.queryByText('未报告')).not.toBeInTheDocument();
  });

  it('keeps all plugin capabilities available to the UI', async () => {
    invokeMock.mockImplementation((command: string, args?: { mutation?: string; params?: Record<string, unknown> }) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(snapshot));
      if (command === 'device_mutate') {
        if (args?.mutation === 'set-dpi-stage') return Promise.resolve({
          ...snapshot,
          dpi: 800,
          dpiStages: snapshot.dpiStages?.map((stage, index) => ({ ...stage, active: index === 0 })),
        });
        if (args?.mutation === 'set-wireless-sleep-time') return Promise.resolve({
          ...snapshot,
          capabilities: { ...snapshot.capabilities, settings: { ...snapshot.capabilities?.settings, wirelessSleepValue: args.params?.seconds } },
        });
        if (args?.mutation === 'set-mouse-lighting') return Promise.resolve({
          ...snapshot,
          confirmedLightColor: String(args.params?.color),
          capabilities: { ...snapshot.capabilities, settings: { ...snapshot.capabilities?.settings, mouseLightStartColor: args.params?.color, mouseLightEndColor: args.params?.color } },
        });
      }
      return Promise.reject(new Error(`unexpected command ${command}`));
    });
    render(<App />);
    expect(await screen.findByText('AM INFINITY 8K MOUSE')).toBeInTheDocument();
    expect(screen.getByLabelText('当前 DPI：1600，点击编辑')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '切换到第 1 档' }));
    await waitFor(() => expect(screen.getByLabelText('当前 DPI：800，点击编辑')).toBeInTheDocument());
    expect(invokeMock).toHaveBeenCalledWith('device_mutate', { mutation: 'set-dpi-stage', params: { stage: 1 } });
    expect(screen.getByText(/搞定啦/)).toBeInTheDocument();
    expect(document.documentElement.style.getPropertyValue('--accent')).toContain('210');
    fireEvent.click(screen.getByRole('button', { name: /76%/ }));
    expect(screen.getByRole('region', { name: '设备电量' })).toHaveTextContent('鼠标76%');
    expect(screen.getByRole('region', { name: '设备电量' })).toHaveTextContent('接收器100%');
    expect(screen.getByRole('button', { name: /76%/ })).toHaveAttribute('aria-expanded', 'true');
    fireEvent.click(screen.getByRole('button', { name: /76%/ }));
    expect(screen.getByRole('button', { name: /76%/ })).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(screen.getByRole('button', { name: /76%/ }));
    fireEvent.click(document.body);
    expect(screen.getByRole('button', { name: /76%/ })).toHaveAttribute('aria-expanded', 'false');

    fireEvent.click(screen.getByRole('tab', { name: '回报率' }));
    const summary = screen.getByLabelText('设备摘要');
    expect(summary.children).toHaveLength(2);
    expect(summary).toHaveStyle({ gridTemplateColumns: 'repeat(2, minmax(0, 1fr))' });
    expect(summary).toHaveTextContent('运动同步开启抬升高度2');

    fireEvent.click(screen.getByRole('button', { name: /插件声明的无线休眠.*1 分钟/ }));
    expect(screen.getByRole('dialog', { name: '设置插件声明的无线休眠' })).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('超时时间（秒）'), { target: { value: '120' } });
    fireEvent.click(screen.getByRole('button', { name: '应用' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('device_mutate', {
      mutation: 'set-wireless-sleep-time', params: { seconds: 120 },
    }));

    fireEvent.click(screen.getByRole('button', { name: /灯光.*已开启/ }));
    expect(screen.getByRole('dialog', { name: '鼠标灯光颜色' })).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('颜色'), { target: { value: '#445566' } });
    fireEvent.click(screen.getByRole('button', { name: '应用' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('device_mutate', {
      mutation: 'set-mouse-lighting',
      params: { color: '#445566', enabled: true, effect: 1, speed: 0, brightness: 100, extraColor: '#000000' },
    }));

    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    expect(screen.queryByRole('tab', { name: '字符灯' })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('tab', { name: '接收器灯光' }));
    expect(screen.getByText('霓虹')).toBeInTheDocument();
    expect(screen.getByText('暗')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '全部读数' }));
    await waitFor(() => expect(screen.getByText('鼠标 USB 固件')).toBeInTheDocument());
    expect(screen.getByText('按键映射')).toBeInTheDocument();
  });

  it('renders a read-only HID++ snapshot without unsupported control tabs', async () => {
    const logitechSnapshot: DeviceSnapshot = {
      displayName: 'G705 Mouse',
      connection: 'wireless',
      batteryPercent: 66,
      charging: false,
      batteries: [{ id: 'mouse', label: '鼠标', percentage: 66, charging: false }],
      dpi: 1800,
      dpiStages: [{ value: 1800, color: '#9a8bd0', active: true, enabled: true }],
      capabilities: {
        device: { deviceIndex: 1, connection: 'wireless', featureIndex: 2 },
        deviceName: { name: 'G705 Mouse' },
        battery: { percentage: 66, charging: false },
        dpi: { sensorIndex: 0, dpiValue: 1800, defaultDpi: 800 },
      },
      pluginCapabilities: [
        {
          id: 'dpi', control: 'DpiStages', labelKey: 'capability.dpi', readOnly: false,
          placements: [{ region: 'control', group: 'performance', order: 10, span: 1, icon: 'gauge' }],
          metadata: { label: 'DPI', source: 'dpiStages', mutations: { value: 'set-dpi-value' } },
        },
      ],
      writableMutations: [],
      evidence: 'source-confirmed',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(logitechSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect(await screen.findByText('G705 Mouse')).toBeInTheDocument();
    expect(screen.getByText('无线 · 已连接')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /66%/ })).toBeInTheDocument();
    expect(screen.getByLabelText('当前 DPI：1800，点击编辑')).toBeDisabled();
    expect(screen.getByRole('tab', { name: 'DPI' })).toBeInTheDocument();
    expect(screen.queryByRole('tab', { name: '回报率' })).not.toBeInTheDocument();
    expect(screen.queryByRole('tab', { name: '灯光' })).not.toBeInTheDocument();
    expect(screen.queryByRole('region', { name: '设备状态' })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '全部读数' }));
    expect(screen.getByText('设备连接')).toBeInTheDocument();
    expect(screen.getByText('当前 DPI')).toBeInTheDocument();
  });

  it('uses extended HID++ mutations and device-reported polling options when available', async () => {
    const extendedSnapshot: DeviceSnapshot = {
      displayName: 'Extended HID++ Mouse',
      connection: 'usb',
      batteryPercent: 88,
      charging: false,
      batteries: [{ id: 'mouse', label: '鼠标', percentage: 88, charging: false }],
      dpi: 2400,
      dpiStages: [{ value: 2400, color: '#9a8bd0', active: true, enabled: true }],
      pollingRateHz: 1000,
      supportedPollingRatesHz: [1000, 2000, 4000, 8000],
      capabilities: {
        dpiExtended: { sensorIndex: 0, dpiValue: 2400 },
        settingsExtended: { pollingRate: 1000 },
      },
      pluginCapabilities: [
        {
          id: 'dpi', control: 'DpiStages', labelKey: 'capability.dpi', readOnly: false,
          placements: [{ region: 'control', group: 'performance', order: 10, span: 1, icon: 'gauge' }],
          metadata: { label: 'DPI', source: 'dpiStages', min: 50, max: 30000, step: 50, mutations: { value: ['set-dpi-value', 'set-dpi-value-extended'] } },
        },
        {
          id: 'polling-rate', control: 'Select', labelKey: 'capability.polling-rate', readOnly: false,
          placements: [{ region: 'control', group: 'polling', order: 20, span: 1, icon: 'wave' }],
          metadata: {
            label: '回报率', source: 'pollingRate', mutation: ['set-polling-rate', 'set-polling-rate-extended'], param: 'rate',
            options: [125, 250, 500, 1000, 2000, 4000, 8000].map((value) => ({ value, label: `${value} Hz` })),
          },
        },
      ],
      writableMutations: ['set-dpi-value-extended', 'set-polling-rate-extended'],
      evidence: 'source-confirmed',
    };
    invokeMock.mockImplementation((command: string, args?: { mutation?: string; params?: Record<string, unknown> }) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(extendedSnapshot));
      if (command === 'device_mutate' && args?.mutation === 'set-dpi-value-extended') {
        return Promise.resolve({ ...extendedSnapshot, dpi: args.params?.dpi, dpiStages: [{ value: Number(args.params?.dpi), color: '#9a8bd0', active: true, enabled: true }] });
      }
      if (command === 'device_mutate' && args?.mutation === 'set-polling-rate-extended') {
        return Promise.resolve({ ...extendedSnapshot, pollingRateHz: args.params?.rate });
      }
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect(await screen.findByText('Extended HID++ Mouse')).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText('当前 DPI：2400，点击编辑'));
    expect(screen.getByLabelText('DPI 数值')).toHaveAttribute('step', '50');
    fireEvent.change(screen.getByLabelText('DPI 数值'), { target: { value: '3201' } });
    expect(screen.getByRole('button', { name: '应用' })).toBeDisabled();
    fireEvent.change(screen.getByLabelText('DPI 数值'), { target: { value: '3200' } });
    fireEvent.click(screen.getByRole('button', { name: '应用' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('device_mutate', {
      mutation: 'set-dpi-value-extended', params: { stage: 1, dpi: 3200 },
    }));

    fireEvent.click(screen.getByRole('tab', { name: '回报率' }));
    expect(screen.queryByRole('combobox', { name: '回报率' })).not.toBeInTheDocument();
    fireEvent.click(screen.getByLabelText('当前回报率：1000 Hz，点击编辑'));
    const pollingSelect = screen.getByRole('combobox', { name: '回报率' });
    expect(screen.queryByRole('option', { name: '125 Hz' })).not.toBeInTheDocument();
    expect(screen.getByRole('option', { name: '8000 Hz' })).toBeInTheDocument();
    fireEvent.change(pollingSelect, { target: { value: '8000' } });
    fireEvent.click(screen.getByRole('button', { name: '应用' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('device_mutate', {
      mutation: 'set-polling-rate-extended', params: { rate: 8000 },
    }));
  });

  it('renders a minimal device without crashing when most capabilities are absent', async () => {
    const minimalSnapshot: DeviceSnapshot = {
      displayName: undefined as unknown as string,
      connection: 'unknown-connection' as unknown as DeviceSnapshot['connection'],
      batteryPercent: undefined,
      charging: undefined,
      batteries: [],
      dpi: undefined,
      dpiStages: undefined,
      pollingRateHz: undefined,
      supportedPollingRatesHz: undefined,
      profile: undefined,
      confirmedLightColor: undefined,
      capabilities: {},
      writableMutations: [],
      evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(minimalSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect(await screen.findByText('未知设备')).toBeInTheDocument();
    expect(screen.getByText('未知连接 · 已连接')).toBeInTheDocument();
    // No control tabs should appear when no capabilities are reported.
    expect(screen.queryByRole('tab', { name: 'DPI' })).not.toBeInTheDocument();
    expect(screen.queryByRole('tab', { name: '回报率' })).not.toBeInTheDocument();
    expect(screen.queryByRole('tab', { name: '灯光' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /电量/ })).not.toBeInTheDocument();
    expect(screen.queryByRole('region', { name: '设备状态' })).not.toBeInTheDocument();
  });

  it('renders partial lighting without receiver tab and shows placeholders', async () => {
    const partialSnapshot: DeviceSnapshot = {
      displayName: 'Simple Mouse',
      connection: 'usb',
      batteryPercent: 80,
      charging: false,
      batteries: [{ id: 'mouse', label: '鼠标', percentage: 80, charging: false }],
      dpi: 1600,
      dpiStages: [{ value: 1600, color: '#9a8bd0', active: true, enabled: true }],
      pollingRateHz: 1000,
      supportedPollingRatesHz: [125, 250, 500, 1000],
      profile: undefined,
      confirmedLightColor: undefined,
      capabilities: {
        mouseEffect: { enabled: true },
      },
      pluginCapabilities: [
        {
          id: 'lighting', control: 'LightingZone', labelKey: 'capability.lighting', readOnly: false,
          placements: [
            { region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' },
            { region: 'status', order: 30, span: 1, icon: 'lightbulb' },
          ],
          metadata: { label: '灯光', source: 'capabilities.mouseEffect', lightingRole: { mouse: 'set-mouse-lighting' } },
        },
      ],
      writableMutations: ['set-mouse-lighting'],
      evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(partialSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect(await screen.findByText('Simple Mouse')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    expect(screen.getByRole('tab', { name: '鼠标灯光' })).toBeInTheDocument();
    expect(screen.getByRole('tablist', { name: '灯光对象' })).toHaveStyle({ gridTemplateColumns: 'repeat(1, minmax(0, 1fr))' });
    expect(screen.queryByRole('tab', { name: '接收器灯光' })).not.toBeInTheDocument();
    // The color row should show a placeholder because mouseEffect lacks a color field.
    const colorRow = screen.getByRole('button', { name: '颜色未报告' });
    expect(colorRow).toBeInTheDocument();
    expect(colorRow.parentElement).toHaveStyle({ gridTemplateColumns: 'repeat(2, minmax(0, 1fr))' });
  });

  it('shows polling placeholder when rate is not reported but control is supported', async () => {
    const noRateSnapshot: DeviceSnapshot = {
      displayName: 'Polling-Only Mouse',
      connection: 'wireless',
      batteryPercent: 90,
      charging: false,
      batteries: [{ id: 'mouse', label: '鼠标', percentage: 90, charging: false }],
      dpi: undefined,
      dpiStages: undefined,
      pollingRateHz: undefined,
      supportedPollingRatesHz: [125, 250, 500, 1000],
      profile: undefined,
      confirmedLightColor: undefined,
      capabilities: {
        settings: { motionSync: false },
      },
      pluginCapabilities: [
        {
          id: 'polling-rate', control: 'Select', labelKey: 'capability.polling-rate', readOnly: false,
          placements: [{ region: 'control', group: 'polling', order: 20, span: 1, icon: 'wave' }],
          metadata: {
            label: '回报率', source: 'pollingRate', mutation: 'set-polling-rate', param: 'rate',
            options: [125, 250, 500, 1000].map((value) => ({ value, label: `${value} Hz` })),
          },
        },
      ],
      writableMutations: ['set-polling-rate'],
      evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(noRateSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect(await screen.findByText('Polling-Only Mouse')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('tab', { name: '回报率' }));
    expect(screen.getByLabelText('回报率未报告，点击设置')).toBeInTheDocument();
    expect(screen.queryByRole('combobox', { name: '回报率' })).not.toBeInTheDocument();
    fireEvent.click(screen.getByLabelText('回报率未报告，点击设置'));
    const pollingSelect = screen.getByRole('combobox', { name: '回报率' });
    expect(pollingSelect).toHaveDisplayValue('125 Hz');
    fireEvent.change(pollingSelect, { target: { value: '125' } });
    fireEvent.click(screen.getByRole('button', { name: '应用' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('device_mutate', {
      mutation: 'set-polling-rate', params: { rate: 125 },
    }));
  });

  it('uses plugin locale labels before manifest fallback labels', async () => {
    const localeSnapshot: DeviceSnapshot = {
      displayName: 'Localized Plugin Mouse',
      connection: 'wireless',
      charging: false,
      batteries: [],
      capabilities: { mouseLighting: { color: '#FF00AA', enabled: true } },
      pluginCapabilities: [{
        id: 'mouse-lighting',
        control: 'LightingZone',
        labelKey: 'capability.mouse-lighting',
        readOnly: false,
        placements: [{ region: 'control', group: 'lighting', order: 10, span: 1, icon: 'lightbulb' }],
        metadata: { label: '灯光', source: 'lighting.mouseLightColor', lightingRole: { mouse: 'set-mouse-lighting' } },
      }],
      writableMutations: ['set-mouse-lighting'],
      evidence: 'fixture-verified',
      pluginId: 'mira.logitech-hidpp',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(localeSnapshot));
      if (command === 'plugin_locales') return Promise.resolve({
        'mira.logitech-hidpp': {
          'zh-CN': { 'plugin.label.capability.mouse-lighting': '插件鼠标灯光' },
          en: { 'plugin.label.capability.mouse-lighting': '插件鼠标灯光' },
        },
      });
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect(await screen.findByText('Localized Plugin Mouse')).toBeInTheDocument();
    expect(await screen.findByRole('tab', { name: '插件鼠标灯光' })).toBeInTheDocument();
    expect(screen.queryByRole('tab', { name: '灯光' })).not.toBeInTheDocument();
  });

  it('renders HID++ mouse lighting with multi-field editor and submits full params', async () => {
    const hidppSnapshot: DeviceSnapshot = {
      displayName: 'HID++ Light Mouse',
      connection: 'wireless',
      batteryPercent: 80,
      charging: false,
      batteries: [{ id: 'mouse', label: '鼠标', percentage: 80, charging: false }],
      dpi: 1600,
      dpiStages: [{ value: 1600, color: '#9a8bd0', active: true, enabled: true }],
      capabilities: {
        mouseLighting: { effect: 1, speed: 128, brightness: 50, color: '#b87ab0', enabled: true, effectName: '常亮', supportedEffects: [0, 1, 3, 4, 5, 10, 11, 12] },
        colorLedInfo: { supportsFixed: true, supportsCycle: true, supportsWave: true, supportsStarlight: true, supportsBreathing: true, supportsRipple: true, supportsCustom: true },
      },
      pluginCapabilities: [
        {
          id: 'mouse-lighting', control: 'LightingZone', labelKey: 'capability.mouse-lighting', readOnly: false,
          placements: [
            { region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' },
            { region: 'status', order: 30, span: 1, icon: 'lightbulb' },
          ],
          metadata: {
            label: '灯光', section: 'control', status: true,
            mutations: { mouse: 'set-mouse-lighting', receiver: 'set-receiver-lighting' },
            lightingRole: { mouse: 'set-mouse-lighting', receiver: 'set-receiver-lighting' },
            effectOptions: {
              offValue: 0,
              effect: [
                { value: 0, labelKey: 'lighting.off' },
                { value: 1, labelKey: 'lighting.fixed' },
                { value: 3, labelKey: 'lighting.cycle' },
                { value: 4, labelKey: 'lighting.wave' },
                { value: 5, labelKey: 'lighting.starlight', requiresExtraColor: true },
                { value: 10, labelKey: 'lighting.breathing' },
                { value: 11, labelKey: 'lighting.ripple' },
                { value: 12, labelKey: 'lighting.custom' },
              ],
              speed: { min: 0, max: 255, step: 1 },
              brightness: { min: 0, max: 100, step: 1 },
            },
          },
        },
      ],
      writableMutations: ['set-mouse-lighting-onboard', 'set-mouse-lighting'],
      evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string, args?: { mutation?: string; params?: Record<string, unknown> }) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(hidppSnapshot));
      if (command === 'device_mutate' && args?.mutation === 'set-mouse-lighting') {
        const caps = hidppSnapshot.capabilities ?? {};
        return Promise.resolve({
          ...hidppSnapshot,
          capabilities: {
            ...caps,
            mouseLighting: { ...caps.mouseLighting, effect: args.params?.effect },
          },
        });
      }
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect(await screen.findByText('HID++ Light Mouse')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));

    // HID++ multi-field UI: effect/speed/brightness/color buttons visible
    expect(screen.getByText('常亮')).toBeInTheDocument();
    expect(screen.getByText('128')).toBeInTheDocument();
    expect(screen.getByText('50%')).toBeInTheDocument();
    const mouseColor = screen.getByText('#b87ab0');
    expect(mouseColor).toHaveClass('color-value');
    expect(mouseColor).toHaveStyle('--value-color: #b87ab0');
    expect(screen.getByRole('button', { name: /状态/ }).parentElement).toHaveStyle({ gridTemplateColumns: 'repeat(5, minmax(0, 1fr))' });
    expect(screen.getByRole('region', { name: '设备状态' })).toHaveTextContent('已开启');

    fireEvent.click(screen.getByRole('button', { name: /状态/ }));
    await waitFor(() => expect(screen.getByRole('region', { name: '设备状态' })).toHaveTextContent('已关闭'));

    fireEvent.click(screen.getByRole('button', { name: /速度/ }));
    expect(screen.getByRole('dialog', { name: '速度' })).toBeInTheDocument();
    expect(screen.getByRole('slider')).toHaveAttribute('min', '0');
    expect(screen.getByRole('slider')).toHaveAttribute('max', '255');
    expect(screen.getByRole('slider').closest('label')).toHaveClass('range-field');
    fireEvent.click(screen.getByRole('button', { name: '取消' }));

    fireEvent.click(screen.getByRole('button', { name: /亮度/ }));
    expect(screen.getByRole('dialog', { name: '亮度' })).toBeInTheDocument();
    expect(screen.getByRole('slider')).toHaveAttribute('min', '0');
    expect(screen.getByRole('slider')).toHaveAttribute('max', '100');
    expect(screen.getByRole('slider').closest('label')).toHaveClass('range-field');
    fireEvent.click(screen.getByRole('button', { name: '取消' }));

    // Click effect button to open edit modal
    fireEvent.click(screen.getByRole('button', { name: /灯效/ }));
    expect(screen.getByRole('dialog', { name: '灯效' })).toBeInTheDocument();

    // Change effect to 'wave' (value 4) and submit
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 4 } });
    fireEvent.click(screen.getByRole('button', { name: '应用' }));

    // Verify full params (effect/speed/brightness/color/extraColor) are submitted
    // extraColor defaults to #000000 when device hasn't reported it (non-starlight effect)
    // lightingRole.mouse uses the unified direct/memory mutation. Even if an older
    // snapshot still lists the narrow onboard mutation, the UI should not pick it.
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('device_mutate', {
      mutation: 'set-mouse-lighting',
      params: { color: '#b87ab0', enabled: true, effect: 4, speed: 128, brightness: 50, extraColor: '#000000' },
    }));
  });

  it('sizes HID++ mouse lighting rows from the rendered field count', async () => {
    const compactLightingSnapshot: DeviceSnapshot = {
      displayName: 'Compact HID++ Light Mouse',
      connection: 'wireless',
      charging: false,
      batteries: [],
      capabilities: {
        mouseLighting: { effect: 1, color: '#FF0000', enabled: true, effectName: '常亮', supportedEffects: [0, 1] },
      },
      pluginCapabilities: [
        {
          id: 'mouse-lighting', control: 'LightingZone', labelKey: 'capability.mouse-lighting', readOnly: false,
          placements: [{ region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' }],
          metadata: {
            label: '灯光', section: 'control',
            lightingRole: { mouse: 'set-mouse-lighting' },
            effectOptions: {
              offValue: 0,
              effect: [
                { value: 0, labelKey: 'lighting.off' },
                { value: 1, labelKey: 'lighting.fixed' },
              ],
            },
          },
        },
      ],
      writableMutations: ['set-mouse-lighting'],
      evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(compactLightingSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect(await screen.findByText('Compact HID++ Light Mouse')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    expect(screen.getByRole('button', { name: /状态/ }).parentElement).toHaveStyle({ gridTemplateColumns: 'repeat(3, minmax(0, 1fr))' });
  });

  it('restores mouse lighting with a currently supported non-off effect', async () => {
    const supportedOnlySnapshot: DeviceSnapshot = {
      displayName: 'Supported Effect Mouse',
      connection: 'wireless',
      charging: false,
      batteries: [],
      capabilities: {
        mouseLighting: { effect: 0, color: '#336699', enabled: false, supportedEffects: [0, 3] },
      },
      pluginCapabilities: [
        {
          id: 'mouse-lighting', control: 'LightingZone', labelKey: 'capability.mouse-lighting', readOnly: false,
          placements: [{ region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' }],
          metadata: {
            label: '灯光', section: 'control',
            lightingRole: { mouse: 'set-mouse-lighting' },
            effectOptions: {
              offValue: 0,
              effect: [
                { value: 0, labelKey: 'lighting.off' },
                { value: 1, labelKey: 'lighting.fixed' },
                { value: 3, labelKey: 'lighting.cycle' },
              ],
            },
          },
        },
      ],
      writableMutations: ['set-mouse-lighting'],
      evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string, args?: { mutation?: string; params?: Record<string, unknown> }) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(supportedOnlySnapshot));
      if (command === 'device_mutate' && args?.mutation === 'set-mouse-lighting') {
        return Promise.resolve({
          ...supportedOnlySnapshot,
          capabilities: {
            ...(supportedOnlySnapshot.capabilities ?? {}),
            mouseLighting: {
              ...(supportedOnlySnapshot.capabilities?.mouseLighting ?? {}),
              effect: args.params?.effect as number,
              enabled: args.params?.enabled as boolean,
            },
          },
        });
      }
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    const view = render(<App />);
    const current = within(view.container);
    expect(await current.findByText('Supported Effect Mouse')).toBeInTheDocument();
    fireEvent.click(current.getByRole('tab', { name: '灯光' }));
    invokeMock.mockClear();
    const statusButtons = current.getAllByRole('button', { name: /状态/ });
    fireEvent.click(statusButtons[statusButtons.length - 1]);

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('device_mutate', {
      mutation: 'set-mouse-lighting',
      params: { color: '#336699', enabled: true, effect: 3, speed: 0, brightness: 100, extraColor: '#000000' },
    }));
  });

  it('shows HID++ mouse lighting as off when RGB control is explicitly disabled', async () => {
    // 当 RGB 控制显式关闭时，effect 字段也应为 offValue(0)，二者一致表示关闭。
    // 若 enabled=false 但 effect!=offValue（板载模式 host 控制关闭但 profile 灯效仍存在），
    // mouseLightingOnState 会优先基于 effect 判定为开启，以匹配设备真实灯效。
    const disabledLightingSnapshot: DeviceSnapshot = {
      displayName: 'Disabled HID++ Light Mouse',
      connection: 'wireless',
      charging: false,
      batteries: [],
      capabilities: {
        mouseLighting: { effect: 0, color: '#004d65', enabled: false },
      },
      pluginCapabilities: [
        {
          id: 'mouse-lighting', control: 'LightingZone', labelKey: 'capability.mouse-lighting', readOnly: false,
          placements: [
            { region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' },
            { region: 'status', order: 20, span: 1, icon: 'lightbulb' },
          ],
          metadata: {
            label: '灯光', section: 'control', status: true,
            lightingRole: { mouse: 'set-mouse-lighting' },
            effectOptions: {
              offValue: 0,
              effect: [
                { value: 0, labelKey: 'lighting.off' },
                { value: 1, labelKey: 'lighting.fixed' },
              ],
            },
          },
        },
      ],
      writableMutations: ['set-mouse-lighting'],
      evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(disabledLightingSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect(await screen.findByText('Disabled HID++ Light Mouse')).toBeInTheDocument();
    expect(screen.getByRole('region', { name: '设备状态' })).toHaveTextContent('已关闭');
  });
});
