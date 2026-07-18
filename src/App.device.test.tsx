// SPDX-License-Identifier: AGPL-3.0-or-later
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';
import App from './App';
import { themeAccent } from './theme';
import type { AppSettings, DeviceSnapshot, PluginFieldOption } from './types';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

beforeAll(() => Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} }));
afterAll(() => Reflect.deleteProperty(window, '__TAURI_INTERNALS__'));

const settings: AppSettings = {
  theme: 'light', autostart: false, startHidden: false, lowBatteryThreshold: 20,
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

const LIGHTING_EFFECT_OPTIONS: PluginFieldOption[] = [
  { value: 0, labelKey: 'lighting.off' },
  { value: 1, labelKey: 'lighting.on' },
  { value: 3, labelKey: 'lighting.effect.neon' },
  { value: 4, labelKey: 'lighting.effect.rainbow' },
];

const snapshot: DeviceSnapshot = {
  displayName: 'AM INFINITY 8K MOUSE', connection: 'wireless', batteryPercent: 76,
  charging: false, dpi: 1600, pollingRateHz: 1000, profile: '1',
  supportedPollingRatesHz: [125, 250, 500, 1000, 2000, 4000, 8000],
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
    settings: { pollingRate: 1000, motionSync: true, angleSnap: false, liftCutOff: 1, wirelessSleepValue: 60, bluetoothSleepValue: 600, mouseLightStartColor: '#112233', mouseLightEndColor: '#112233', mouseLightEnabled: true },
    mouseLighting: { effect: 1, effectName: '常亮', speed: 3, brightness: 70, color: '#112233', extraColor: '#112233', enabled: true },
    receiverLighting: { effect: 3, effectName: '霓虹', speed: 3, brightness: 1, option: 7, optionName: '自定义', color: '#AABBCC', enabled: true },
    firmwareUsb: { versionRaw: 258 },
    buttonMappings: { '0x00': [1, 0, 0, 0] },
  },
  pluginCapabilities: [
    {
      id: 'dpi', control: 'DpiStages', labelKey: 'plugin.label.capability.dpi', readOnly: false,
      placements: [{ region: 'control', group: 'performance', order: 10, span: 1, icon: 'gauge' }],
      metadata: {
        stageLayout: {
          dotsSource: 'state.dpiStages', selectMutation: 'set-dpi-stage', setMutation: 'set-dpi-value',
          valueSource: 'state.dpiStages', colorSource: 'state.dpiStages', range: { min: 100, max: 32000, step: 50 },
        },
        stateMapping: { dpiStages: 'dpiStages' },
      },
    },
    {
      id: 'polling-rate', control: 'Select', labelKey: 'plugin.label.capability.polling-rate', readOnly: false,
      placements: [{ region: 'control', group: 'polling', order: 20, span: 1, icon: 'wave' }],
      metadata: {
        fields: [{
          id: 'value', source: 'state.pollingRate', mutation: 'set-polling-rate', param: 'value',
          editor: 'modal-select', optionSource: 'state.supportedPollingRates', format: 'hertz',
          labelKey: 'plugin.label.capability.polling-rate',
        }],
        stateMapping: { pollingRate: 'pollingRateHz', supportedPollingRates: 'supportedPollingRatesHz' },
        summary: [
          { labelKey: 'mock.motionSync', source: 'capabilities.settings.motionSync' },
          { labelKey: 'mock.angleSnap', source: 'capabilities.settings.angleSnap' },
          { labelKey: 'mock.liftCutOff', source: 'capabilities.settings.liftCutOff' },
        ],
      },
    },
    {
      id: 'sleep-time', control: 'Number', labelKey: 'plugin.label.capability.sleep-time', readOnly: false,
      placements: [{ region: 'status', order: 30, span: 1, icon: 'timer' }],
      metadata: {
        fields: [{
          id: 'value', source: 'state.wirelessSleepValue', mutation: 'set-wireless-sleep-time', param: 'value',
          editor: 'modal-range', format: 'sleep', range: { min: 0, max: 1800, step: 30 },
          labelKey: 'plugin.label.capability.sleep-time',
          visibleWhen: { path: 'connection', eq: 'wireless' },
        }],
        statusDisplay: { valueSource: 'state.wirelessSleepValue', valueFormat: 'sleep', onClickField: 'value' },
        stateMapping: { wirelessSleepValue: 'capabilities.settings.wirelessSleepValue' },
      },
    },
    {
      id: 'profile', control: 'ReadOnlyValue', labelKey: 'plugin.label.capability.profile', readOnly: true,
      placements: [{ region: 'status', order: 20, span: 1, icon: 'profile' }],
      metadata: {
        fields: [{ id: 'value', source: 'state.profile', editor: 'static-readonly', labelKey: 'plugin.label.capability.profile' }],
        statusDisplay: { valueSource: 'state.profile' },
        stateMapping: { profile: 'profile' },
      },
    },
    {
      id: 'lighting', control: 'LightingZone', labelKey: 'plugin.label.capability.lighting', readOnly: false,
      placements: [
        { region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' },
        { region: 'status', order: 40, span: 1, icon: 'lightbulb' },
      ],
      metadata: {
        accentSource: 'state.mouseLightColor',
        zones: [
          {
            id: 'mouse', labelKey: 'dashboard.mouseLighting',
            fields: [
              { id: 'status', source: 'state.mouseLightEffect', mutation: 'set-mouse-lighting', param: 'effect', editor: 'inline-toggle', switch: { source: 'state.mouseLightEffect', offValue: 0, restoreField: 'effect' }, labelKey: 'dashboard.status' },
              { id: 'effect', source: 'state.mouseLightEffect', mutation: 'set-mouse-lighting', param: 'effect', editor: 'modal-select', labelKey: 'receiverLighting.field.effect', labelSource: 'capabilities.mouseLighting.effectName', options: LIGHTING_EFFECT_OPTIONS, visibleWhen: { path: 'state.mouseLightEffect', ne: 0 } },
              { id: 'color', source: 'state.mouseLightColor', mutation: 'set-mouse-lighting', param: 'color', editor: 'modal-color', labelKey: 'dashboard.mouseLightColor', visibleWhen: { path: 'state.mouseLightEffect', ne: 0 } },
            ],
          },
          {
            id: 'receiver', labelKey: 'dashboard.receiverLighting',
            visibleWhen: { path: 'capabilities.receiverLighting', ne: null },
            fields: [
              { id: 'status', source: 'state.receiverLightEffect', mutation: 'set-receiver-lighting', param: 'effect', editor: 'inline-toggle', switch: { source: 'state.receiverLightEffect', offValue: 0, restoreField: 'effect' }, labelKey: 'dashboard.status' },
              { id: 'effect', source: 'state.receiverLightEffect', mutation: 'set-receiver-lighting', param: 'effect', editor: 'modal-select', labelKey: 'receiverLighting.field.effect', labelSource: 'capabilities.receiverLighting.effectName', options: LIGHTING_EFFECT_OPTIONS, visibleWhen: { path: 'state.receiverLightEffect', ne: 0 } },
              { id: 'color', source: 'state.receiverLightColor', mutation: 'set-receiver-lighting', param: 'color', editor: 'modal-color', labelKey: 'receiverLighting.field.color', visibleWhen: { path: 'state.receiverLightEffect', ne: 0 } },
            ],
          },
        ],
        statusDisplay: { valueSource: 'state.mouseLightColor', valueFormat: 'color', onClickField: 'status' },
        stateMapping: {
          mouseLightColor: 'confirmedLightColor',
          mouseLightEffect: 'capabilities.mouseLighting.effect',
          receiverLightEffect: 'capabilities.receiverLighting.effect',
          receiverLightColor: 'capabilities.receiverLighting.color',
        },
      },
    },
  ],
  writableMutations: ['set-dpi-stage', 'set-dpi-value', 'set-wireless-sleep-time', 'set-mouse-lighting', 'set-receiver-lighting'],
};

function entries(...snapshots: DeviceSnapshot[]) {
  return snapshots.map((item, index) => ({
    deviceKey: `device-${index}`,
    snapshot: item,
    selected: index === 0,
  }));
}

describe('real device snapshot mapping', () => {
  it('renders the signed plugin layout during the provisional presence snapshot', async () => {
    const provisionalSnapshot: DeviceSnapshot = {
      ...snapshot,
      batteryPercent: undefined,
      batteries: [],
      dpi: undefined,
      dpiStages: undefined,
      pollingRateHz: undefined,
      supportedPollingRatesHz: undefined,
      profile: undefined,
      confirmedLightColor: undefined,
      capabilities: {},
      writableMutations: [],
      pluginCapabilities: snapshot.pluginCapabilities?.map((capability) => ({
        ...capability,
        metadata: { ...capability.metadata, _miraRuntimePending: true },
      })),
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(provisionalSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'AM INFINITY 8K MOUSE' });

    const dashboard = document.querySelector('.dashboard');
    expect(dashboard).toHaveAttribute('aria-busy', 'true');
    expect(screen.getAllByRole('tab').map((tab) => tab.textContent)).toEqual(['DPI', '回报率', '灯光']);
    expect(screen.getByRole('region', { name: '设备状态' })).toBeInTheDocument();
    expect(document.querySelectorAll('.dpi-stage-placeholder')).toHaveLength(5);
    expect(screen.queryByText('这台鼠标还没带回 DPI 档位呢')).not.toBeInTheDocument();
  });

  it('hides capabilities that runtime probing marks unavailable', async () => {
    const probedSnapshot: DeviceSnapshot = {
      ...snapshot,
      pluginCapabilities: snapshot.pluginCapabilities?.map((capability) => (
        capability.id === 'polling-rate' || capability.id === 'sleep-time'
          ? { ...capability, available: false }
          : capability
      )),
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(probedSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'AM INFINITY 8K MOUSE' });
    expect(screen.queryByRole('tab', { name: '回报率' })).not.toBeInTheDocument();
    expect(screen.getAllByRole('tab').map((tab) => tab.textContent)).toEqual(['DPI', '灯光']);
    expect(screen.getByRole('region', { name: '设备状态' })).not.toHaveTextContent('休眠时间');
  });

  it('renders plugin-declared readback summary inside the polling-rate block', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(snapshot));
      if (command === 'device_refresh_quick') return Promise.resolve();
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'AM INFINITY 8K MOUSE' });
    expect(screen.queryByLabelText('设备摘要')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('tab', { name: '回报率' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('device_refresh_quick'));
    const summary = screen.getByLabelText('设备摘要');
    expect(summary).toHaveTextContent('运动同步开启');
    expect(summary).toHaveTextContent('角度吸附关闭');
    expect(summary).toHaveTextContent('抬升高度1');
    expect(summary.closest('.metric-reading')).toBeInTheDocument();
  });

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

  it('crossfades color codes without recoloring the outgoing value', async () => {
    const firstSnapshot: DeviceSnapshot = {
      ...snapshot,
      displayName: 'First Color Mouse',
      confirmedLightColor: '#112233',
      capabilities: {
        ...snapshot.capabilities,
        mouseLighting: { ...snapshot.capabilities!.mouseLighting, color: '#112233' },
      },
    };
    const secondSnapshot: DeviceSnapshot = {
      ...firstSnapshot,
      displayName: 'Second Color Mouse',
      confirmedLightColor: '#8fc7b8',
      capabilities: {
        ...firstSnapshot.capabilities,
        mouseLighting: { ...firstSnapshot.capabilities!.mouseLighting, color: '#8fc7b8' },
      },
    };
    invokeMock.mockImplementation((command: string, args?: { deviceKey?: string }) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(firstSnapshot, secondSnapshot));
      if (command === 'device_select' && args?.deviceKey === 'device-1') return Promise.resolve(secondSnapshot);
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'First Color Mouse' });
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    const colorValue = document.querySelector<HTMLElement>('.lighting-group-mouse .color-value')!;
    expect(colorValue.querySelector<HTMLElement>('.live-value-current')?.style.getPropertyValue('--value-color')).toBe('#112233');

    fireEvent.click(screen.getByRole('button', { name: '切换鼠标' }));
    fireEvent.click(screen.getByText('Second Color Mouse').closest('button')!);

    await waitFor(() => expect(colorValue.querySelector('.live-value-next')).toHaveTextContent('#8fc7b8'));
    expect(colorValue.querySelector<HTMLElement>('.live-value-current')?.style.getPropertyValue('--value-color')).toBe('#112233');
    expect(colorValue.querySelector<HTMLElement>('.live-value-next')?.style.getPropertyValue('--value-color')).toBe('#8fc7b8');
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
      metadata: {
        fields: [{ id: 'value', source: 'battery', editor: 'static-readonly' as const, format: 'percent' as const }],
        statusDisplay: { valueSource: 'battery', valueFormat: 'percent' as const },
      },
    }));
    const gridSnapshot: DeviceSnapshot = {
      displayName: 'Grid Mouse', connection: 'virtual', batteryPercent: 80, charging: false, batteries: [],
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

  it('uses the shared continuous battery icon for plugin-declared dashboard battery status', async () => {
    const batteryStatusSnapshot: DeviceSnapshot = {
      displayName: 'Battery Status Mouse', connection: 'wireless', batteryPercent: 67,
      charging: false, batteries: [{ id: 'mouse', label: '鼠标', percentage: 67, charging: false }],
      capabilities: {}, writableMutations: [], evidence: 'fixture-verified',
      pluginCapabilities: [
        {
          id: 'battery-status',
          control: 'ReadOnlyValue',
          labelKey: 'plugin.label.capability.battery',
          readOnly: true,
          placements: [{ region: 'status', order: 10, span: 1, icon: 'battery' }],
          metadata: {
            fields: [{ id: 'value', source: 'battery', editor: 'static-readonly', format: 'percent', labelKey: 'plugin.label.capability.battery' }],
            statusDisplay: { valueSource: 'battery', valueFormat: 'percent' },
            stateMapping: { battery: 'batteryPercent', charging: 'charging' },
          },
        },
      ],
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(batteryStatusSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    const status = await screen.findByRole('region', { name: '设备状态' });
    expect(status.querySelector('.battery-level-icon')).toBeInTheDocument();
    expect(status.querySelector('.battery-level-fill')).toHaveAttribute('width', String((16 * 67) / 100));
    expect(status.querySelector('svg:not(.battery-level-svg)')).not.toBeInTheDocument();
  });

  it('executes an inline-toggle status field without opening an unsupported modal', async () => {
    const toggleStatusSnapshot: DeviceSnapshot = {
      displayName: 'Contract Toggle Mouse', connection: 'wireless', batteryPercent: 80,
      charging: false, batteries: [],
      capabilities: {
        settings: { mouseLightEnabled: true },
        mouseLighting: { color: '#112233' },
      },
      pluginCapabilities: [
        {
          id: 'lighting', control: 'LightingZone', labelKey: 'plugin.label.capability.lighting', readOnly: false,
          placements: [
            { region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' },
            { region: 'status', order: 30, span: 1, icon: 'lightbulb' },
          ],
          metadata: {
            zones: [{
              id: 'mouse', labelKey: 'dashboard.mouseLighting', fields: [{
                id: 'enabled', source: 'capabilities.settings.mouseLightEnabled',
                mutation: 'set-mouse-lighting', param: 'enabled', editor: 'inline-toggle',
                switch: { source: 'capabilities.settings.mouseLightEnabled', offValue: false },
                labelKey: 'dashboard.status',
                paramSources: {
                  color: 'capabilities.mouseLighting.color',
                  enabled: 'capabilities.settings.mouseLightEnabled',
                },
              }],
            }],
            statusDisplay: {
              labelKey: 'plugin.label.capability.lighting',
              valueSource: 'capabilities.settings.mouseLightEnabled',
              valueOptions: [
                { value: false, labelKey: 'lighting.off' },
                { value: true, labelKey: 'lighting.on' },
              ],
              onClickField: 'enabled',
            },
          },
        },
      ],
      writableMutations: ['set-mouse-lighting'], evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string, args?: { mutation?: string }) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(toggleStatusSnapshot));
      if (command === 'device_mutate' && args?.mutation === 'set-mouse-lighting') return Promise.resolve(toggleStatusSnapshot);
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    const status = await screen.findByRole('region', { name: '设备状态' });
    const lightingStatus = status.querySelector('button');
    expect(lightingStatus).toHaveTextContent('灯光已开启');
    fireEvent.click(lightingStatus as HTMLButtonElement);

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('device_mutate', {
      mutation: 'set-mouse-lighting',
      params: { color: '#112233', enabled: false },
    }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.queryByText('未报告')).not.toBeInTheDocument();
  });

  it('renders plugin-declared controls and status without a brand-specific branch', async () => {
    const pluginSnapshot: DeviceSnapshot = {
      displayName: 'Declarative Mouse', connection: 'wireless', batteryPercent: 80,
      charging: false, batteries: [{ id: 'mouse', label: '鼠标', percentage: 80, charging: false }],
      dpi: 1600, dpiStages: [{ value: 1600, color: '#9a8bd0', active: true, enabled: true }],
      capabilities: { controlMode: { mode: 1, modeName: 'onboard' } },
      pluginCapabilities: [
        {
          id: 'control-mode', control: 'Segmented', labelKey: 'plugin.label.capability.lighting', readOnly: false,
          placements: [
            { region: 'control', group: 'configuration', order: 20, span: 1, icon: 'profile' },
            { region: 'status', order: 10, span: 2, icon: 'profile' },
          ],
          metadata: {
            fields: [{
              id: 'mode', source: 'state.controlMode', mutation: 'set-control-mode', param: 'mode',
              editor: 'inline-segmented',
              options: [{ value: 1, labelKey: '板载' }, { value: 2, labelKey: '软件' }],
              labelKey: 'plugin.label.capability.lighting',
            }],
            statusDisplay: { valueSource: 'state.controlMode', valueOptions: [{ value: 1, labelKey: '板载' }, { value: 2, labelKey: '软件' }], onClickField: 'mode' },
            stateMapping: { controlMode: 'capabilities.controlMode.mode' },
          },
        },
        {
          id: 'dpi', control: 'DpiStages', labelKey: 'plugin.label.capability.dpi', readOnly: false,
          placements: [{ region: 'control', group: 'performance', order: 10, span: 1, icon: 'gauge' }],
          metadata: {
            stageLayout: { dotsSource: 'state.dpiStages', selectMutation: 'set-dpi-stage', setMutation: 'set-dpi-value', valueSource: 'state.dpiStages', colorSource: 'state.dpiStages', range: { min: 100, max: 32000, step: 50 } },
            stateMapping: { dpiStages: 'dpiStages' },
          },
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
    expect((await screen.findAllByRole('tab')).map((tab) => tab.textContent)).toEqual(['DPI', '灯光']);
    expect(screen.getByRole('tab', { name: '灯光' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'DPI' })).toBeInTheDocument();
    expect(screen.getByRole('region', { name: '设备状态' })).toHaveTextContent('灯光板载');
    expect(screen.getByRole('region', { name: '设备状态' })).toHaveAttribute('data-status-count', '1');
    fireEvent.click(screen.getByRole('region', { name: '设备状态' }).querySelector('button') as HTMLButtonElement);
    expect(screen.getByRole('tab', { name: '灯光' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();

    const stage = document.querySelector('.control-stage')!;
    expect(stage).toHaveAttribute('data-control-transition', 'dpi-to-segmented');
    expect(stage.querySelector('.control-stage-page.is-leaving')).toHaveAttribute('data-page-kind', 'dpi');
    let enteringPage = stage.querySelector('.control-stage-page.is-entering')!;
    expect(enteringPage).toHaveAttribute('data-page-kind', 'segmented');
    expect(within(enteringPage as HTMLElement).getByRole('group', { name: '灯光' })).toBeInTheDocument();
    fireEvent.animationEnd(enteringPage, { animationName: 'mira-control-page-enter' });
    await waitFor(() => expect(stage.querySelector('.control-stage-page.is-leaving')).not.toBeInTheDocument());

    fireEvent.click(screen.getByRole('tab', { name: 'DPI' }));
    expect(stage).toHaveAttribute('data-control-transition', 'segmented-to-dpi');
    expect(stage.querySelector('.control-stage-page.is-leaving')).toHaveAttribute('data-page-kind', 'segmented');
    enteringPage = stage.querySelector('.control-stage-page.is-entering')!;
    expect(enteringPage).toHaveAttribute('data-page-kind', 'dpi');
    expect(enteringPage.querySelector('.dpi-scale')).toBeInTheDocument();
    fireEvent.animationEnd(enteringPage, { animationName: 'mira-control-page-enter' });
    await waitFor(() => expect(stage.querySelector('.control-stage-page.is-leaving')).not.toBeInTheDocument());

    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    enteringPage = stage.querySelector('.control-stage-page.is-entering')!;
    fireEvent.animationEnd(enteringPage, { animationName: 'mira-control-page-enter' });
    fireEvent.click(screen.getByRole('button', { name: '软件' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('device_mutate', {
      mutation: 'set-control-mode', params: { mode: 2 },
    }));
    await waitFor(() => {
      expect(screen.getByRole('region', { name: '设备状态' })).toHaveTextContent('灯光软件');
    });
  });

  it('keeps the global mouse accent while the lighting tabs follow the active zone color', async () => {
    const receiverOnlySnapshot: DeviceSnapshot = {
      displayName: 'Receiver-lit Mouse',
      connection: 'wireless',
      batteryPercent: 80,
      charging: false,
      batteries: [{ id: 'mouse', label: '鼠标', percentage: 80, charging: false }],
      dpi: 1600,
      dpiStages: [{ value: 1600, color: '#9a8bd0', active: true, enabled: true }],
      confirmedLightColor: '#CC2244',
      capabilities: {
        mouseLighting: { effect: 1, color: '#CC2244', enabled: true },
        receiverLighting: { effect: 1, effectName: '常亮', speed: 2, brightness: 3, option: 7, color: '#00FF00', enabled: true },
      },
      pluginCapabilities: [
        {
          id: 'dpi', control: 'DpiStages', labelKey: 'plugin.label.capability.dpi', readOnly: false,
          metadata: {
            stageLayout: {
              dotsSource: 'state.dpiStages', selectMutation: 'set-dpi-stage', setMutation: 'set-dpi-value',
              valueSource: 'state.dpiStages', colorSource: 'state.dpiStages', range: { min: 100, max: 32000, step: 50 },
            },
            stateMapping: { dpiStages: 'dpiStages' },
          },
        },
        {
          id: 'lighting', control: 'LightingZone', labelKey: 'plugin.label.capability.lighting', readOnly: false,
          placements: [
            { region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' },
            { region: 'status', order: 30, span: 1, icon: 'lightbulb' },
          ],
          metadata: {
            accentSource: 'state.mouseLightColor',
            zones: [
              {
                id: 'mouse', labelKey: 'dashboard.mouseLighting',
                fields: [
                  { id: 'status', source: 'state.mouseLightEffect', mutation: 'set-mouse-lighting', param: 'effect', editor: 'inline-toggle', switch: { source: 'state.mouseLightEffect', offValue: 0, restoreField: 'effect' }, labelKey: 'dashboard.status' },
                  { id: 'color', source: 'state.mouseLightColor', mutation: 'set-mouse-lighting', param: 'color', editor: 'modal-color', format: 'color', labelKey: 'dashboard.mouseLightColor' },
                ],
              },
              {
                id: 'receiver', labelKey: 'dashboard.receiverLighting',
                visibleWhen: { path: 'capabilities.receiverLighting', ne: null },
                fields: [
                  { id: 'status', source: 'state.receiverLightEffect', mutation: 'set-receiver-lighting', param: 'effect', editor: 'inline-toggle', switch: { source: 'state.receiverLightEffect', offValue: 0, restoreField: 'effect' }, labelKey: 'dashboard.status' },
                  { id: 'color', source: 'state.receiverLightColor', mutation: 'set-receiver-lighting', param: 'color', editor: 'modal-color', labelKey: 'receiverLighting.field.color', visibleWhen: { path: 'state.receiverLightEffect', ne: 0 } },
                ],
              },
            ],
            statusDisplay: { valueSource: 'state.mouseLightColor', valueFormat: 'color', onClickField: 'status' },
            stateMapping: {
              mouseLightColor: 'confirmedLightColor',
              mouseLightEffect: 'capabilities.mouseLighting.effect',
              receiverLightEffect: 'capabilities.receiverLighting.effect',
              receiverLightColor: 'capabilities.receiverLighting.color',
            },
          },
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
    await screen.findByRole('heading', { name: 'Receiver-lit Mouse' });
    const accent = document.documentElement.style.getPropertyValue('--accent');
    expect(accent).toBe(themeAccent('#CC2244'));
    expect(accent).not.toBe(themeAccent('#00FF00'));
    expect(accent).not.toBe(themeAccent('#9a8bd0'));

    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    const lightingTabs = screen.getByRole('tablist', { name: '灯光对象' });
    expect(lightingTabs.style.getPropertyValue('--segmented-indicator-accent')).toBe('var(--accent)');

    fireEvent.click(screen.getByRole('tab', { name: '接收器灯光' }));
    expect(lightingTabs.style.getPropertyValue('--segmented-indicator-accent')).toBe('#00FF00');
    expect(document.documentElement.style.getPropertyValue('--accent')).toBe(themeAccent('#CC2244'));
  });

  it('uses receiver lighting options to label the off effect', async () => {
    const offReceiverSnapshot: DeviceSnapshot = {
      displayName: 'Off Receiver Mouse', connection: 'wireless', batteryPercent: 80,
      charging: false, batteries: [], dpi: 1600,
      dpiStages: [{ value: 1600, color: '#9a8bd0', active: true, enabled: true }],
      capabilities: {
        receiverLighting: { effect: 0, effectName: '关闭', speed: 0, brightness: 0, color: '#000000', enabled: false },
      },
      pluginCapabilities: [
        {
          id: 'lighting', control: 'LightingZone', labelKey: 'plugin.label.capability.lighting', readOnly: false,
          placements: [{ region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' }],
          metadata: {
            zones: [
              {
                id: 'receiver', labelKey: 'dashboard.receiverLighting',
                visibleWhen: { path: 'capabilities.receiverLighting', ne: null },
                fields: [
                  { id: 'status', source: 'state.receiverLightEffect', mutation: 'set-receiver-lighting', param: 'effect', editor: 'inline-toggle', switch: { source: 'state.receiverLightEffect', offValue: 0, restoreField: 'effect' }, labelKey: 'dashboard.status' },
                  { id: 'effect', source: 'state.receiverLightEffect', mutation: 'set-receiver-lighting', param: 'effect', editor: 'modal-select', labelKey: 'receiverLighting.field.effect', labelSource: 'capabilities.receiverLighting.effectName', options: LIGHTING_EFFECT_OPTIONS, visibleWhen: { path: 'state.receiverLightEffect', ne: 0 } },
                ],
              },
            ],
            stateMapping: { receiverLightEffect: 'capabilities.receiverLighting.effect' },
          },
        },
      ],
      writableMutations: ['set-receiver-lighting'], evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(offReceiverSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'Off Receiver Mouse' });
    // 灯效为 0（off）时，开关显示"关闭"
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    expect(screen.getByText('关闭')).toBeInTheDocument();
  });

  it('renders Logitech HID++ pointer speed as a modal-range field', async () => {
    const pointerSnapshot: DeviceSnapshot = {
      displayName: 'Pointer Mouse', connection: 'usb', batteryPercent: 90,
      charging: false, batteries: [], dpi: 800,
      dpiStages: [{ value: 800, color: '#7ea7d8', active: true, enabled: true }],
      capabilities: { settings: { pointerSpeed: 512 } },
      pluginCapabilities: [
        {
          id: 'pointer-speed', control: 'Slider', labelKey: 'plugin.label.capability.firmware', readOnly: false,
          placements: [{ region: 'control', group: 'sensor', order: 40, span: 1, icon: 'gauge' }],
          metadata: {
            fields: [{
              id: 'value', source: 'state.pointerSpeed', mutation: 'set-pointer-speed', param: 'value',
              editor: 'modal-range', range: { min: 0, max: 1000, step: 1 }, labelKey: 'capability.field.sensorIndex',
            }],
            stateMapping: { pointerSpeed: 'capabilities.settings.pointerSpeed' },
          },
        },
      ],
      writableMutations: ['set-pointer-speed'], evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(pointerSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'Pointer Mouse' });
    fireEvent.click(screen.getByRole('tab', { name: '固件' }));
    // 普通设置沿用旧版的大号可编辑读数样式，而不是灯光卡片。
    const editButton = screen.getByRole('button', { name: /传感器索引/ });
    expect(editButton).toHaveClass('plugin-value-button');
    fireEvent.click(editButton);
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(screen.getByRole('slider')).toBeInTheDocument();
  });

  it('keeps Logitech generic number and select controls collapsed', async () => {
    const logitechSnapshot: DeviceSnapshot = {
      displayName: 'Logitech Mouse', connection: 'usb', batteryPercent: 90,
      charging: false, batteries: [], dpi: 800,
      dpiStages: [{ value: 800, color: '#7ea7d8', active: true, enabled: true }],
      capabilities: { settings: { angleSnap: false, liftCutOff: 2 } },
      pluginCapabilities: [
        {
          id: 'angle-snap', control: 'Toggle', labelKey: 'plugin.label.capability.firmware', readOnly: false,
          placements: [{ region: 'control', group: 'sensor', order: 10, span: 1, icon: 'gauge' }],
          metadata: {
            fields: [{ id: 'value', source: 'state.angleSnap', mutation: 'set-angle-snap', param: 'value', editor: 'inline-toggle', labelKey: 'capability.field.sensorIndex' }],
            stateMapping: { angleSnap: 'capabilities.settings.angleSnap' },
          },
        },
        {
          id: 'lift-cutoff', control: 'Select', labelKey: 'plugin.label.capability.profile', readOnly: false,
          placements: [{ region: 'control', group: 'sensor', order: 20, span: 1, icon: 'settings' }],
          metadata: {
            fields: [{ id: 'value', source: 'state.liftCutOff', mutation: 'set-lift-cutoff', param: 'value', editor: 'modal-select', labelKey: 'capability.field.sensorIndex', options: [{ value: 1, labelKey: '1mm' }, { value: 2, labelKey: '2mm' }] }],
            stateMapping: { liftCutOff: 'capabilities.settings.liftCutOff' },
          },
        },
      ],
      writableMutations: ['set-angle-snap', 'set-lift-cutoff'], evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(logitechSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'Logitech Mouse' });
    // 两个 capability 同属 sensor group，合并为一个标签页
    expect(screen.getAllByRole('tab')).toHaveLength(1);
  });

  it('hides Logitech numeric controls when value not reported', async () => {
    const missingValueSnapshot: DeviceSnapshot = {
      displayName: 'Missing Value Mouse', connection: 'usb', batteryPercent: 90,
      charging: false, batteries: [], dpi: 800,
      dpiStages: [{ value: 800, color: '#7ea7d8', active: true, enabled: true }],
      capabilities: { settings: {} },
      pluginCapabilities: [
        {
          id: 'pointer-speed', control: 'Slider', labelKey: 'plugin.label.capability.firmware', readOnly: false,
          placements: [{ region: 'control', group: 'sensor', order: 10, span: 1, icon: 'gauge' }],
          metadata: {
            fields: [{
              id: 'value', source: 'state.pointerSpeed', mutation: 'set-pointer-speed', param: 'value',
              editor: 'modal-range', range: { min: 0, max: 1000, step: 1 }, labelKey: 'capability.field.sensorIndex',
            }],
            stateMapping: { pointerSpeed: 'capabilities.settings.pointerSpeed' },
          },
        },
      ],
      writableMutations: ['set-pointer-speed'], evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(missingValueSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'Missing Value Mouse' });
    // 即使插件漏写 visibleWhen，未上报的普通数值能力也不能产生空标签页或占位控件。
    expect(screen.queryByRole('tab')).not.toBeInTheDocument();
    expect(screen.queryByText('capability.field.sensorIndex')).not.toBeInTheDocument();
    expect(screen.queryByText('未报告')).not.toBeInTheDocument();
  });

  it('removes unreported sibling controls and follows plugin icon declarations', async () => {
    const mixedSnapshot: DeviceSnapshot = {
      displayName: 'Mixed Capability Mouse', connection: 'wireless', batteryPercent: 80,
      charging: false, batteries: [], dpi: 1600,
      dpiStages: [{ value: 1600, color: '#9a8bd0', active: true, enabled: true }],
      capabilities: { controlMode: { mode: 1 } },
      pluginCapabilities: [
        {
          id: 'control-mode', control: 'Segmented', labelKey: '配置控制', readOnly: false,
          placements: [{ region: 'control', group: 'configuration', order: 5, span: 1, icon: 'settings' }],
          metadata: { fields: [{
            id: 'value', source: 'capabilities.controlMode.mode', mutation: 'set-control-mode', param: 'mode',
            editor: 'inline-segmented', labelKey: '配置控制',
            options: [{ value: 1, labelKey: '板载' }, { value: 2, labelKey: '软件' }],
          }] },
        },
        {
          id: 'profile-mgmt-current', control: 'Number', labelKey: '当前配置文件', readOnly: false,
          placements: [{ region: 'control', group: 'configuration', order: 6, span: 1, icon: 'profile' }],
          metadata: { fields: [{
            id: 'value', source: 'capabilities.profileMgmtCurrent.profileIndex',
            mutation: 'set-profile-mgmt-current', param: 'profileIndex', editor: 'modal-number',
            labelKey: '当前配置文件', range: { min: 0, max: 15, step: 1 },
          }] },
        },
        {
          id: 'dpi', control: 'DpiStages', labelKey: 'DPI', readOnly: false,
          placements: [{ region: 'control', group: 'performance', order: 10, span: 1, icon: 'gauge' }],
          metadata: {
            stageLayout: {
              dotsSource: 'state.dpiStages', selectMutation: 'set-dpi-stage',
              setMutation: 'set-dpi-value', valueSource: 'state.dpiStages',
              range: { min: 100, max: 32000, step: 50 },
            },
            stateMapping: { dpiStages: 'dpiStages' },
          },
        },
        {
          id: 'pointer-speed', control: 'Number', labelKey: '指针速度', readOnly: false,
          placements: [{ region: 'control', group: 'performance', order: 15, span: 1, icon: 'gauge' }],
          metadata: { fields: [{
            id: 'value', source: 'capabilities.pointerSpeed.speedRaw', mutation: 'set-pointer-speed',
            param: 'speed', editor: 'modal-number', labelKey: '指针速度',
            range: { min: 46, max: 511, step: 1 },
          }] },
        },
      ],
      writableMutations: ['set-control-mode', 'set-profile-mgmt-current', 'set-dpi-stage', 'set-dpi-value', 'set-pointer-speed'],
      evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(mixedSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'Mixed Capability Mouse' });

    expect(screen.getAllByRole('tab').map((tab) => tab.textContent)).toEqual(['配置控制', 'DPI']);
    expect(document.querySelector('.plugin-control-reading svg')).toHaveAttribute('data-plugin-icon', 'settings');
    expect(screen.queryByText('当前配置文件')).not.toBeInTheDocument();
    expect(screen.queryByText('指针速度')).not.toBeInTheDocument();
    expect(screen.queryByText('未报告')).not.toBeInTheDocument();
  });

  it('keeps all plugin capabilities available from the main snapshot', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(snapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'AM INFINITY 8K MOUSE' });
    const tabs = screen.getAllByRole('tab').map((t) => t.textContent);
    expect(tabs).toEqual(expect.arrayContaining(['DPI', '回报率', '灯光']));
    expect(screen.getByRole('region', { name: '设备状态' })).toHaveAttribute('data-status-count', '3');
  });

  it('renders a read-only HID++ snapshot', async () => {
    const readonlySnapshot: DeviceSnapshot = {
      displayName: 'Readonly Mouse', connection: 'usb', batteryPercent: 90,
      charging: false, batteries: [], dpi: 800,
      dpiStages: [{ value: 800, color: '#7ea7d8', active: true, enabled: true }],
      capabilities: { settings: { pointerSpeed: 512 } },
      readonly: true,
      pluginCapabilities: [
        {
          id: 'pointer-speed', control: 'Slider', labelKey: 'plugin.label.capability.firmware', readOnly: true,
          placements: [{ region: 'control', group: 'sensor', order: 10, span: 1, icon: 'gauge' }],
          metadata: {
            fields: [{ id: 'value', source: 'state.pointerSpeed', editor: 'static-readonly', labelKey: 'capability.field.sensorIndex' }],
            stateMapping: { pointerSpeed: 'capabilities.settings.pointerSpeed' },
          },
        },
      ],
      writableMutations: [], evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(readonlySnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'Readonly Mouse' });
    fireEvent.click(screen.getByRole('tab', { name: '固件' }));
    // 只读快照中 static-readonly 渲染值，无编辑按钮
    expect(screen.getByText('512')).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('uses extended HID++ mutations', async () => {
    const extendedSnapshot: DeviceSnapshot = {
      displayName: 'Extended Mouse', connection: 'usb', batteryPercent: 90,
      charging: false, batteries: [], dpi: 800,
      dpiStages: [{ value: 800, color: '#7ea7d8', active: true, enabled: true }],
      capabilities: { settings: { pointerSpeed: 512 } },
      pluginCapabilities: [
        {
          id: 'pointer-speed', control: 'Slider', labelKey: 'plugin.label.capability.firmware', readOnly: false,
          placements: [{ region: 'control', group: 'sensor', order: 10, span: 1, icon: 'gauge' }],
          metadata: {
            fields: [{
              id: 'value', source: 'state.pointerSpeed', mutation: 'set-pointer-speed', param: 'value',
              editor: 'modal-range', range: { min: 0, max: 1000, step: 1 }, labelKey: 'capability.field.sensorIndex',
            }],
            stateMapping: { pointerSpeed: 'capabilities.settings.pointerSpeed' },
          },
        },
      ],
      writableMutations: ['set-pointer-speed'], evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string, args?: { mutation?: string; params?: Record<string, unknown> }) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(extendedSnapshot));
      if (command === 'device_mutate' && args?.mutation === 'set-pointer-speed') {
        return Promise.resolve({ ...extendedSnapshot, capabilities: { settings: { pointerSpeed: args.params?.value as number } } });
      }
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'Extended Mouse' });
    fireEvent.click(screen.getByRole('tab', { name: '固件' }));
    const editButton = screen.getByRole('button', { name: /传感器索引/ });
    fireEvent.click(editButton);
    const slider = screen.getByRole('slider');
    fireEvent.change(slider, { target: { value: 800 } });
    fireEvent.click(screen.getByRole('button', { name: '应用' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('device_mutate', {
      mutation: 'set-pointer-speed', params: { value: 800 },
    }));
  });

  it('renders a minimal device without crashing', async () => {
    const minimalSnapshot: DeviceSnapshot = {
      displayName: 'Minimal Mouse', connection: 'usb', charging: false, batteries: [],
      capabilities: {}, pluginCapabilities: [], writableMutations: [], evidence: 'unknown',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(minimalSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    expect(await screen.findByRole('heading', { name: 'Minimal Mouse' })).toBeInTheDocument();
    // tablist 容器总是渲染，但无 capability 时不应渲染任何 tab
    expect(screen.queryByRole('tab')).not.toBeInTheDocument();
    expect(screen.queryByRole('region', { name: '设备状态' })).not.toBeInTheDocument();
  });

  it('renders partial lighting without receiver tab', async () => {
    const partialLightingSnapshot: DeviceSnapshot = {
      displayName: 'Partial Lighting Mouse', connection: 'wireless', batteryPercent: 80,
      charging: false, batteries: [], dpi: 1600,
      dpiStages: [{ value: 1600, color: '#9a8bd0', active: true, enabled: true }],
      confirmedLightColor: '#112233',
      capabilities: { mouseLighting: { effect: 1, effectName: '常亮', speed: 3, brightness: 70, color: '#112233', enabled: true } },
      pluginCapabilities: [
        {
          id: 'lighting', control: 'LightingZone', labelKey: 'plugin.label.capability.lighting', readOnly: false,
          placements: [{ region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' }],
          metadata: {
            zones: [
              {
                id: 'mouse', labelKey: 'dashboard.mouseLighting',
                fields: [
                  { id: 'status', source: 'state.mouseLightEffect', mutation: 'set-mouse-lighting', param: 'effect', editor: 'inline-toggle', switch: { source: 'state.mouseLightEffect', offValue: 0, restoreField: 'effect' }, labelKey: 'dashboard.status' },
                  { id: 'effect', source: 'state.mouseLightEffect', mutation: 'set-mouse-lighting', param: 'effect', editor: 'modal-select', labelKey: 'receiverLighting.field.effect', labelSource: 'capabilities.mouseLighting.effectName', options: LIGHTING_EFFECT_OPTIONS, visibleWhen: { path: 'state.mouseLightEffect', ne: 0 } },
                ],
              },
              {
                id: 'receiver', labelKey: 'dashboard.receiverLighting',
                // 仅当 capabilities.receiverLighting 存在时才渲染此 zone
                // （{ path } 形式使用 value != null 判断，undefined 时为 false）
                visibleWhen: { path: 'capabilities.receiverLighting' },
                fields: [
                  { id: 'status', source: 'state.receiverLightEffect', mutation: 'set-receiver-lighting', param: 'effect', editor: 'inline-toggle', switch: { source: 'state.receiverLightEffect', offValue: 0, restoreField: 'effect' }, labelKey: 'dashboard.status' },
                ],
              },
            ],
            stateMapping: { mouseLightColor: 'confirmedLightColor', mouseLightEffect: 'capabilities.mouseLighting.effect' },
          },
        },
      ],
      writableMutations: ['set-mouse-lighting'], evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(partialLightingSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'Partial Lighting Mouse' });
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    // 无 receiverLighting capability 数据，receiver 区域不渲染
    expect(screen.queryByRole('tablist', { name: '灯光对象' })).not.toBeInTheDocument();
    expect(screen.getByText('状态')).toBeInTheDocument();
    expect(document.querySelector('.lighting-group-title')).toHaveTextContent('鼠标灯光');
  });

  it('keeps a five-field secondary lighting zone in the legacy compact grid', async () => {
    const compactLightingSnapshot: DeviceSnapshot = {
      ...snapshot,
      displayName: 'Compact Receiver Mouse',
      pluginCapabilities: (snapshot.pluginCapabilities ?? []).map((capability) => capability.id === 'lighting'
        ? {
            ...capability,
            metadata: {
              ...capability.metadata,
              zones: [
                {
                  id: 'mouse', labelKey: 'dashboard.mouseLighting',
                  fields: [{
                    id: 'status', source: 'capabilities.mouseLighting.enabled', mutation: 'set-mouse-lighting', param: 'enabled',
                    editor: 'inline-toggle', switch: { source: 'capabilities.mouseLighting.enabled', offValue: false }, labelKey: 'dashboard.status',
                  }],
                },
                {
                  id: 'receiver', labelKey: 'dashboard.receiverLighting', visibleWhen: { path: 'capabilities.receiverLighting' },
                  fields: [
                    { id: 'effect', source: 'capabilities.receiverLighting.effect', mutation: 'set-receiver-lighting', param: 'effect', editor: 'modal-select', labelKey: 'receiverLighting.field.effect', options: LIGHTING_EFFECT_OPTIONS },
                    { id: 'option', source: 'capabilities.receiverLighting.option', mutation: 'set-receiver-lighting', param: 'option', editor: 'modal-select', labelKey: 'receiverLighting.field.option', options: LIGHTING_EFFECT_OPTIONS },
                    { id: 'speed', source: 'capabilities.receiverLighting.speed', mutation: 'set-receiver-lighting', param: 'speed', editor: 'modal-select', labelKey: 'receiverLighting.field.speed', options: LIGHTING_EFFECT_OPTIONS },
                    { id: 'brightness', source: 'capabilities.receiverLighting.brightness', mutation: 'set-receiver-lighting', param: 'brightness', editor: 'modal-select', labelKey: 'receiverLighting.field.brightness', options: LIGHTING_EFFECT_OPTIONS },
                    { id: 'color', source: 'capabilities.receiverLighting.color', mutation: 'set-receiver-lighting', param: 'color', editor: 'modal-color', format: 'color', labelKey: 'receiverLighting.field.color' },
                  ],
                },
              ],
            },
          }
        : capability),
      writableMutations: [...(snapshot.writableMutations ?? []), 'set-receiver-lighting'],
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(compactLightingSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'Compact Receiver Mouse' });
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    fireEvent.click(screen.getByRole('tab', { name: '接收器灯光' }));
    const receiverGroup = document.querySelector('.lighting-group-receiver');
    expect(receiverGroup).toHaveClass('is-compact');
    expect(receiverGroup?.querySelectorAll('.lighting-row')).toHaveLength(5);
  });

  it('hides polling control when rate not reported', async () => {
    const noPollingSnapshot: DeviceSnapshot = {
      displayName: 'No Polling Mouse', connection: 'wireless', batteryPercent: 80,
      charging: false, batteries: [], dpi: 1600,
      dpiStages: [{ value: 1600, color: '#9a8bd0', active: true, enabled: true }],
      capabilities: {},
      pluginCapabilities: [
        {
          id: 'polling-rate', control: 'Select', labelKey: 'plugin.label.capability.polling-rate', readOnly: false,
          placements: [{ region: 'control', group: 'polling', order: 20, span: 1, icon: 'wave' }],
          metadata: {
            fields: [{ id: 'value', source: 'state.pollingRate', mutation: 'set-polling-rate', param: 'value', editor: 'modal-select', optionSource: 'state.supportedPollingRates', format: 'hertz', labelKey: 'plugin.label.capability.polling-rate' }],
            stateMapping: { pollingRate: 'pollingRateHz', supportedPollingRates: 'supportedPollingRatesHz' },
          },
        },
      ],
      writableMutations: ['set-polling-rate'], evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(noPollingSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'No Polling Mouse' });
    expect(screen.queryByRole('tab', { name: '回报率' })).not.toBeInTheDocument();
    expect(screen.queryByText('未报告')).not.toBeInTheDocument();
    expect(document.querySelector('.metric-reading')).not.toBeInTheDocument();
  });

  it('uses plugin locale labels when available', async () => {
    const localeSnapshot: DeviceSnapshot = {
      displayName: 'Locale Mouse', connection: 'wireless', batteryPercent: 80,
      charging: false, batteries: [], dpi: 1600,
      dpiStages: [{ value: 1600, color: '#9a8bd0', active: true, enabled: true }],
      confirmedLightColor: '#112233',
      capabilities: { mouseLighting: { effect: 2, effectName: '呼吸', speed: 3, brightness: 70, color: '#112233', enabled: true } },
      pluginId: 'test-plugin',
      pluginCapabilities: [
        {
          id: 'lighting', control: 'LightingZone', labelKey: 'plugin.label.capability.lighting', readOnly: false,
          placements: [{ region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' }],
          metadata: {
            zones: [
              {
                id: 'mouse', labelKey: 'dashboard.mouseLighting',
                fields: [
                  { id: 'status', source: 'state.mouseLightEffect', mutation: 'set-mouse-lighting', param: 'effect', editor: 'inline-toggle', switch: { source: 'state.mouseLightEffect', offValue: 0, restoreField: 'effect' }, labelKey: 'dashboard.status' },
                  { id: 'effect', source: 'state.mouseLightEffect', mutation: 'set-mouse-lighting', param: 'effect', editor: 'modal-select', labelKey: 'receiverLighting.field.effect', labelSource: 'capabilities.mouseLighting.effectName', options: LIGHTING_EFFECT_OPTIONS, visibleWhen: { path: 'state.mouseLightEffect', ne: 0 } },
                ],
              },
            ],
            stateMapping: { mouseLightColor: 'confirmedLightColor', mouseLightEffect: 'capabilities.mouseLighting.effect' },
          },
        },
      ],
      writableMutations: ['set-mouse-lighting'], evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(localeSnapshot));
      if (command === 'plugin_locales') return Promise.resolve({});
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'Locale Mouse' });
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    // labelSource 从 capabilities.mouseLighting.effectName 读取运行时标签
    expect(screen.getByText('呼吸')).toBeInTheDocument();
  });

  it('renders HID++ mouse lighting with multi-field editor', async () => {
    const multiFieldLightingSnapshot: DeviceSnapshot = {
      displayName: 'Multi-Field Lighting Mouse', connection: 'wireless', batteryPercent: 80,
      charging: false, batteries: [], dpi: 1600,
      dpiStages: [{ value: 1600, color: '#9a8bd0', active: true, enabled: true }],
      confirmedLightColor: '#112233',
      capabilities: {
        mouseLighting: { effect: 3, effectName: '霓虹', speed: 5, brightness: 80, color: '#112233', extraColor: '#445566', enabled: true },
      },
      pluginCapabilities: [
        {
          id: 'lighting', control: 'LightingZone', labelKey: 'plugin.label.capability.lighting', readOnly: false,
          placements: [{ region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' }],
          metadata: {
            zones: [
              {
                id: 'mouse', labelKey: 'dashboard.mouseLighting',
                fields: [
                  { id: 'status', source: 'state.mouseLightEffect', mutation: 'set-mouse-lighting', param: 'effect', editor: 'inline-toggle', switch: { source: 'state.mouseLightEffect', offValue: 0, restoreField: 'effect' }, labelKey: 'dashboard.status' },
                  { id: 'effect', source: 'state.mouseLightEffect', mutation: 'set-mouse-lighting', param: 'effect', editor: 'modal-select', labelKey: 'receiverLighting.field.effect', labelSource: 'capabilities.mouseLighting.effectName', options: LIGHTING_EFFECT_OPTIONS, visibleWhen: { path: 'state.mouseLightEffect', ne: 0 } },
                  { id: 'speed', source: 'state.mouseLightSpeed', mutation: 'set-mouse-lighting', param: 'speed', editor: 'modal-range', labelKey: 'receiverLighting.field.speed', range: { min: 0, max: 10, step: 1 }, visibleWhen: { path: 'state.mouseLightEffect', ne: 0 } },
                  { id: 'brightness', source: 'state.mouseLightBrightness', mutation: 'set-mouse-lighting', param: 'brightness', editor: 'modal-range', labelKey: 'receiverLighting.field.brightness', range: { min: 0, max: 100, step: 1 }, visibleWhen: { path: 'state.mouseLightEffect', ne: 0 } },
                  { id: 'color', source: 'state.mouseLightColor', mutation: 'set-mouse-lighting', param: 'color', editor: 'modal-color', labelKey: 'dashboard.mouseLightColor', visibleWhen: { path: 'state.mouseLightEffect', ne: 0 } },
                ],
              },
            ],
            stateMapping: {
              mouseLightColor: 'confirmedLightColor',
              mouseLightEffect: 'capabilities.mouseLighting.effect',
              mouseLightSpeed: 'capabilities.mouseLighting.speed',
              mouseLightBrightness: 'capabilities.mouseLighting.brightness',
            },
          },
        },
      ],
      writableMutations: ['set-mouse-lighting'], evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(multiFieldLightingSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'Multi-Field Lighting Mouse' });
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    // 灯效非 0 时，所有字段可见
    // effect 字段 labelSource 返回 capabilities.mouseLighting.effectName = "霓虹"
    expect(screen.getByText('霓虹')).toBeInTheDocument();
    expect(screen.getByText('速度')).toBeInTheDocument();
    expect(screen.getByText('亮度')).toBeInTheDocument();
    // 点击灯效字段打开编辑弹窗（按钮 accessible name = label + value = "霓虹 3"）
    fireEvent.click(screen.getByRole('button', { name: /霓虹/ }));
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(screen.getByRole('combobox')).toBeInTheDocument();
  });

  it('sizes HID++ mouse lighting rows from rendered field count', async () => {
    const rowSizingSnapshot: DeviceSnapshot = {
      displayName: 'Row Sizing Mouse', connection: 'wireless', batteryPercent: 80,
      charging: false, batteries: [], dpi: 1600,
      dpiStages: [{ value: 1600, color: '#9a8bd0', active: true, enabled: true }],
      confirmedLightColor: '#112233',
      capabilities: { mouseLighting: { effect: 3, effectName: '霓虹', speed: 5, brightness: 80, color: '#112233', enabled: true } },
      pluginCapabilities: [
        {
          id: 'lighting', control: 'LightingZone', labelKey: 'plugin.label.capability.lighting', readOnly: false,
          placements: [{ region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' }],
          metadata: {
            zones: [
              {
                id: 'mouse', labelKey: 'dashboard.mouseLighting',
                fields: [
                  { id: 'status', source: 'state.mouseLightEffect', mutation: 'set-mouse-lighting', param: 'effect', editor: 'inline-toggle', switch: { source: 'state.mouseLightEffect', offValue: 0, restoreField: 'effect' }, labelKey: 'dashboard.status' },
                  { id: 'effect', source: 'state.mouseLightEffect', mutation: 'set-mouse-lighting', param: 'effect', editor: 'modal-select', labelKey: 'receiverLighting.field.effect', labelSource: 'capabilities.mouseLighting.effectName', options: LIGHTING_EFFECT_OPTIONS, visibleWhen: { path: 'state.mouseLightEffect', ne: 0 } },
                  { id: 'color', source: 'state.mouseLightColor', mutation: 'set-mouse-lighting', param: 'color', editor: 'modal-color', labelKey: 'dashboard.mouseLightColor', visibleWhen: { path: 'state.mouseLightEffect', ne: 0 } },
                ],
              },
            ],
            stateMapping: { mouseLightColor: 'confirmedLightColor', mouseLightEffect: 'capabilities.mouseLighting.effect' },
          },
        },
      ],
      writableMutations: ['set-mouse-lighting'], evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(rowSizingSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'Row Sizing Mouse' });
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    // 3 个可见字段（status + effect + color），lighting-rows grid 为 3 列
    const rows = screen.getByLabelText('灯光分组').querySelector('.lighting-rows');
    expect(rows).toHaveStyle({ gridTemplateColumns: 'repeat(3, minmax(0, 1fr))' });
  });

  it('restores mouse lighting with supported non-off effect', async () => {
    const restoreSnapshot: DeviceSnapshot = {
      displayName: 'Restore Mouse', connection: 'wireless', batteryPercent: 80,
      charging: false, batteries: [], dpi: 1600,
      dpiStages: [{ value: 1600, color: '#9a8bd0', active: true, enabled: true }],
      confirmedLightColor: '#112233',
      capabilities: { mouseLighting: { effect: 1, effectName: '常亮', speed: 3, brightness: 70, color: '#112233', enabled: true } },
      pluginCapabilities: [
        {
          id: 'lighting', control: 'LightingZone', labelKey: 'plugin.label.capability.lighting', readOnly: false,
          placements: [{ region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' }],
          metadata: {
            zones: [
              {
                id: 'mouse', labelKey: 'dashboard.mouseLighting',
                fields: [
                  { id: 'status', source: 'state.mouseLightEffect', mutation: 'set-mouse-lighting', param: 'effect', editor: 'inline-toggle', switch: { source: 'state.mouseLightEffect', offValue: 0, restoreField: 'effect' }, labelKey: 'dashboard.status' },
                  { id: 'effect', source: 'state.mouseLightEffect', mutation: 'set-mouse-lighting', param: 'effect', editor: 'modal-select', labelKey: 'receiverLighting.field.effect', labelSource: 'capabilities.mouseLighting.effectName', options: LIGHTING_EFFECT_OPTIONS, visibleWhen: { path: 'state.mouseLightEffect', ne: 0 } },
                ],
              },
            ],
            stateMapping: { mouseLightColor: 'confirmedLightColor', mouseLightEffect: 'capabilities.mouseLighting.effect' },
          },
        },
      ],
      writableMutations: ['set-mouse-lighting'], evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string, args?: { mutation?: string; params?: Record<string, unknown> }) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(restoreSnapshot));
      if (command === 'device_mutate' && args?.mutation === 'set-mouse-lighting') {
        const newEffect = args.params?.effect as number;
        return Promise.resolve({
          ...restoreSnapshot,
          capabilities: { ...restoreSnapshot.capabilities!, mouseLighting: { ...restoreSnapshot.capabilities!.mouseLighting, effect: newEffect, effectName: newEffect === 0 ? '关闭' : '常亮' } },
        });
      }
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'Restore Mouse' });
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    // 初始灯效为 1（常亮），开关显示"开启"
    expect(screen.getByText('开启')).toBeInTheDocument();
    // 点击开关关闭灯光（发送 offValue=0）
    fireEvent.click(screen.getByRole('button', { name: /状态/ }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('device_mutate', {
      mutation: 'set-mouse-lighting', params: { effect: 0 },
    }));
    // 再次点击开关恢复灯光（发送上次非 off 值 1）
    fireEvent.click(screen.getByRole('button', { name: /状态/ }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('device_mutate', {
      mutation: 'set-mouse-lighting', params: { effect: 1 },
    }));
  });

  it('shows HID++ mouse lighting as off when RGB disabled', async () => {
    const offLightingSnapshot: DeviceSnapshot = {
      displayName: 'Off Lighting Mouse', connection: 'wireless', batteryPercent: 80,
      charging: false, batteries: [], dpi: 1600,
      dpiStages: [{ value: 1600, color: '#9a8bd0', active: true, enabled: true }],
      confirmedLightColor: '#112233',
      capabilities: { mouseLighting: { effect: 0, effectName: '关闭', speed: 0, brightness: 0, color: '#112233', enabled: false } },
      pluginCapabilities: [
        {
          id: 'lighting', control: 'LightingZone', labelKey: 'plugin.label.capability.lighting', readOnly: false,
          placements: [{ region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' }],
          metadata: {
            zones: [
              {
                id: 'mouse', labelKey: 'dashboard.mouseLighting',
                fields: [
                  { id: 'status', source: 'state.mouseLightEffect', mutation: 'set-mouse-lighting', param: 'effect', editor: 'inline-toggle', switch: { source: 'state.mouseLightEffect', offValue: 0, restoreField: 'effect' }, labelKey: 'dashboard.status' },
                  { id: 'effect', source: 'state.mouseLightEffect', mutation: 'set-mouse-lighting', param: 'effect', editor: 'modal-select', labelKey: 'receiverLighting.field.effect', labelSource: 'capabilities.mouseLighting.effectName', options: LIGHTING_EFFECT_OPTIONS, visibleWhen: { path: 'state.mouseLightEffect', ne: 0 } },
                  { id: 'color', source: 'state.mouseLightColor', mutation: 'set-mouse-lighting', param: 'color', editor: 'modal-color', labelKey: 'dashboard.mouseLightColor', visibleWhen: { path: 'state.mouseLightEffect', ne: 0 } },
                ],
              },
            ],
            stateMapping: { mouseLightColor: 'confirmedLightColor', mouseLightEffect: 'capabilities.mouseLighting.effect' },
          },
        },
      ],
      writableMutations: ['set-mouse-lighting'], evidence: 'hardware-verified',
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settings);
      if (command === 'device_snapshots') return Promise.resolve(entries(offLightingSnapshot));
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<App />);
    await screen.findByRole('heading', { name: 'Off Lighting Mouse' });
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    // 灯效为 0（off）时，开关显示"关闭"，effect/color 字段不可见
    expect(screen.getByText('关闭')).toHaveClass('lighting-status-value');
    expect(screen.queryByText('灯效')).not.toBeInTheDocument();
    expect(screen.queryByText('鼠标灯光颜色')).not.toBeInTheDocument();
  });
});
