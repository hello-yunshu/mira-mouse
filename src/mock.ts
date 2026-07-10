// SPDX-License-Identifier: AGPL-3.0-or-later
// Explicit test/development boundary. Production must obtain snapshots from Tauri commands.
import type { BatteryHistoryResponse, BatteryHistoryRange, DeviceSnapshot, DeviceSnapshotEntry, DeviceState, DpiStage, PluginFieldOption } from './types';
import { DEFAULT_THEME_ACCENT } from './theme';

// ─── 共享常量 ────────────────────────────────────────────────────────────────
// 把原硬编码在 DeviceState 顶层的字段抽成常量，便于 mockSnapshot 与 state 共用。
const MOCK_DPI_STAGES: DpiStage[] = [
  { value: 400, color: '#7ea7d8', enabled: true, active: false },
  { value: 800, color: '#9a8bd0', enabled: true, active: false },
  { value: 1000, color: '#bf7fa8', enabled: true, active: true },
  { value: 1600, color: '#d39378', enabled: true, active: false },
  { value: 2400, color: '#7eb2a0', enabled: true, active: false },
  { value: 3200, color: '#a8c46a', enabled: true, active: false },
  { value: 6400, color: '#c9a86c', enabled: true, active: false },
  { value: 12800, color: '#c77a9a', enabled: true, active: false },
];
const MOCK_SUPPORTED_POLLING_RATES: number[] = [125, 250, 500, 1000, 2000, 4000, 8000];
const MOCK_MOUSE_LIGHT_COLOR = DEFAULT_THEME_ACCENT;

// 灯效选项：mouse 与 receiver 共用同一套声明式选项。
const LIGHTING_EFFECT_OPTIONS: PluginFieldOption[] = [
  { value: 0, labelKey: 'lighting.effect.off' },
  { value: 1, labelKey: 'lighting.effect.fixed' },
  { value: 2, labelKey: 'lighting.effect.breathing' },
  { value: 3, labelKey: 'lighting.effect.neon' },
  { value: 4, labelKey: 'lighting.effect.rainbow' },
];

export const MOCK_DEVICE: DeviceState = {
  name: 'Mira Example Wireless Mouse', connection: 'virtual', battery: 82, charging: false,
  batteries: [
    { id: 'mouse', label: 'mock.mouseLabel', percentage: 82, charging: false },
    { id: 'receiver', label: 'mock.receiverLabel', percentage: 100, charging: false },
  ],
  // 把原 pollingRate/supportedPollingRates/profile/dpiStages/lighting 等硬编码字段
  // 统一收拢进 state，capability 通过 source: 'state.*' 读取。
  state: {
    pollingRate: 1000,
    supportedPollingRates: MOCK_SUPPORTED_POLLING_RATES,
    profile: 'Profile 1',
    dpiStages: MOCK_DPI_STAGES,
    // 鼠标灯光状态
    mouseLightEnabled: true,
    mouseLightColor: MOCK_MOUSE_LIGHT_COLOR,
    mouseLightEndColor: MOCK_MOUSE_LIGHT_COLOR,
    mouseLightEffect: 2,
    mouseLightSpeed: 2,
    mouseLightBrightness: 70,
    mouseLightExtraColor: MOCK_MOUSE_LIGHT_COLOR,
    // 接收器灯光状态
    receiverLightEnabled: true,
    receiverLightEffect: 3,
    receiverLightSpeed: 2,
    receiverLightBrightness: 70,
    receiverLightColor: MOCK_MOUSE_LIGHT_COLOR,
    // 休眠时间
    wirelessSleepValue: 300,
    bluetoothSleepValue: 600,
  },
  capabilities: {
    battery: { percentage: 82, charging: false, valid: true },
    dpi: { profile: 0, currentStage: 3, stageCount: 8, dpiX: [400, 800, 1000, 1600, 2400, 3200, 6400, 12800], dpiY: [400, 800, 1000, 1600, 2400, 3200, 6400, 12800], stageColors: ['#7ea7d8', '#9a8bd0', '#bf7fa8', '#d39378', '#7eb2a0', '#a8c46a', '#c9a86c', '#c77a9a'] },
    settings: { profile: 0, pollingRaw: 1, pollingRate: 1000, usbDebounce: 4, wirelessDebounce: 4, bluetoothDebounce: 4, rippleCorrection: true, angleSnap: false, motionSync: true, liftCutOff: 1, buttonChangeTime: 12, wheelToButton: 0, buttonToWheel: 0, bluetoothSleepValue: 600, wirelessSleepValue: 300, mouseLightStartColor: DEFAULT_THEME_ACCENT, mouseLightEndColor: DEFAULT_THEME_ACCENT, mouseLightEnabled: true },
    mouseLighting: { effect: 2, effectName: '呼吸', speed: 2, brightness: 70, color: DEFAULT_THEME_ACCENT, extraColor: DEFAULT_THEME_ACCENT, enabled: true },
    receiverLighting: { effect: 3, effectName: '霓虹', speed: 2, brightness: 70, option: 7, color: DEFAULT_THEME_ACCENT, enabled: true },
    fps: { enabled: true },
    dpiButton: { enabled: true },
    firmwareUsb: { versionRaw: 258 },
    firmwareSoc: { versionRaw: 515 },
    receiverFirmwareUsb: { versionRaw: 257 },
    receiverFirmwareSoc: { versionRaw: 3 },
    receiverFirmwareLed: { versionRaw: 260 },
    buttonMappings: { '0x00': [1, 0, 0, 0], '0x01': [2, 0, 0, 0], '0x02': [3, 0, 0, 0], '0x03': [4, 0, 0, 0], '0x04': [5, 0, 0, 0], '0x0e': [14, 0, 0, 0], '0x0f': [15, 0, 0, 0] },
  },
  pluginCapabilities: [
    // 电量：只读静态展示，从 device.battery 顶层读取。
    {
      id: 'battery', control: 'ReadOnlyValue', labelKey: 'plugin.label.capability.battery', readOnly: true,
      placements: [{ region: 'hero', order: 10, span: 1, icon: 'battery' }],
      metadata: {
        fields: [{ id: 'value', source: 'battery', editor: 'static-readonly', format: 'percent', labelKey: 'plugin.label.capability.battery' }],
        stateMapping: { battery: 'batteryPercent', charging: 'charging' },
      },
    },
    // DPI 分档：使用 stageLayout 声明档位布局与 mutation。
    {
      id: 'dpi', control: 'DpiStages', labelKey: 'plugin.label.capability.dpi', readOnly: false,
      placements: [{ region: 'control', group: 'performance', order: 10, span: 1, icon: 'gauge' }],
      metadata: {
        stageLayout: {
          dotsSource: 'state.dpiStages',
          selectMutation: 'set-active-dpi-stage',
          setMutation: 'set-dpi-stage-value',
          valueSource: 'state.dpiStages',
          colorSource: 'state.dpiStages',
          range: { min: 100, max: 32000, step: 50 },
        },
        stateMapping: { dpiStages: 'dpiStages' },
      },
    },
    // 回报率：modal-select，选项由 optionSource 动态读取。
    {
      id: 'polling-rate', control: 'Select', labelKey: 'plugin.label.capability.polling-rate', readOnly: false,
      placements: [{ region: 'control', group: 'polling', order: 20, span: 1, icon: 'wave' }],
      metadata: {
        fields: [{
          id: 'value',
          source: 'state.pollingRate',
          mutation: 'set-polling-rate',
          param: 'value',
          editor: 'modal-select',
          optionSource: 'state.supportedPollingRates',
          format: 'hertz',
          labelKey: 'plugin.label.capability.polling-rate',
        }],
        stateMapping: { pollingRate: 'pollingRateHz', supportedPollingRates: 'supportedPollingRatesHz' },
      },
    },
    // 休眠时间：modal-range，状态栏点击进入编辑。
    {
      id: 'sleep-time', control: 'Number', labelKey: 'plugin.label.capability.sleep-time', readOnly: false,
      placements: [{ region: 'status', order: 10, span: 1, icon: 'timer' }],
      metadata: {
        fields: [{
          id: 'value',
          source: 'state.wirelessSleepValue',
          mutation: 'set-sleep',
          param: 'value',
          editor: 'modal-range',
          format: 'sleep',
          range: { min: 0, max: 1800, step: 30 },
          labelKey: 'plugin.label.capability.sleep-time',
        }],
        statusDisplay: { valueSource: 'state.wirelessSleepValue', valueFormat: 'sleep', onClickField: 'value' },
        stateMapping: { wirelessSleepValue: 'capabilities.settings.wirelessSleepValue' },
      },
    },
    // 配置文件：只读展示。
    {
      id: 'profile', control: 'ReadOnlyValue', labelKey: 'plugin.label.capability.profile', readOnly: true,
      placements: [{ region: 'status', order: 20, span: 1, icon: 'profile' }],
      metadata: {
        fields: [{ id: 'value', source: 'state.profile', editor: 'static-readonly', labelKey: 'plugin.label.capability.profile' }],
        statusDisplay: { valueSource: 'state.profile', onClickField: 'value' },
        stateMapping: { profile: 'profile' },
      },
    },
    // 灯光：LightingZone 声明 mouse 与 receiver 两个区域，每个区域含一组字段。
    {
      id: 'lighting', control: 'LightingZone', labelKey: 'plugin.label.capability.lighting', readOnly: false,
      placements: [{ region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' }, { region: 'status', order: 30, span: 1, icon: 'lightbulb' }],
      metadata: {
        zones: [
          {
            id: 'mouse',
            labelKey: 'lighting.mouse',
            fields: [
              { id: 'status', source: 'state.mouseLightEffect', mutation: 'set-mouse-lighting', param: 'effect', editor: 'inline-toggle', switch: { source: 'state.mouseLightEffect', offValue: 0, restoreField: 'effect' }, labelKey: 'dashboard.status' },
              { id: 'effect', source: 'state.mouseLightEffect', mutation: 'set-mouse-lighting', param: 'effect', editor: 'modal-select', labelKey: 'receiverLighting.field.effect', labelSource: 'capabilities.mouseLighting.effectName', options: LIGHTING_EFFECT_OPTIONS, visibleWhen: { path: 'state.mouseLightEffect', ne: null } },
              { id: 'speed', source: 'state.mouseLightSpeed', mutation: 'set-mouse-lighting', param: 'speed', editor: 'modal-range', labelKey: 'capability.field.speed', range: { min: 0, max: 10, step: 1 }, visibleWhen: { path: 'state.mouseLightEffect', ne: null } },
              { id: 'brightness', source: 'state.mouseLightBrightness', mutation: 'set-mouse-lighting', param: 'brightness', editor: 'modal-range', labelKey: 'capability.field.brightness', range: { min: 0, max: 100, step: 1 }, visibleWhen: { path: 'state.mouseLightEffect', ne: null } },
              { id: 'color', source: 'state.mouseLightColor', mutation: 'set-mouse-lighting', param: 'color', editor: 'modal-color', labelKey: 'capability.field.color', visibleWhen: { path: 'state.mouseLightEffect', ne: null } },
              { id: 'extraColor', source: 'state.mouseLightExtraColor', mutation: 'set-mouse-lighting', param: 'extraColor', editor: 'modal-color', labelKey: 'lighting.extraColor', visibleWhen: { path: 'state.mouseLightEffect', ne: null } },
            ],
          },
          {
            id: 'receiver',
            labelKey: 'lighting.receiver',
            visibleWhen: { path: 'capabilities.receiverLighting', ne: null },
            fields: [
              { id: 'status', source: 'state.receiverLightEffect', mutation: 'set-receiver-lighting', param: 'effect', editor: 'inline-toggle', switch: { source: 'state.receiverLightEffect', offValue: 0, restoreField: 'effect' }, labelKey: 'dashboard.status' },
              { id: 'effect', source: 'state.receiverLightEffect', mutation: 'set-receiver-lighting', param: 'effect', editor: 'modal-select', labelKey: 'receiverLighting.field.effect', labelSource: 'capabilities.receiverLighting.effectName', options: LIGHTING_EFFECT_OPTIONS, visibleWhen: { path: 'state.receiverLightEffect', ne: null } },
              { id: 'speed', source: 'state.receiverLightSpeed', mutation: 'set-receiver-lighting', param: 'speed', editor: 'modal-range', labelKey: 'receiverLighting.field.speed', range: { min: 0, max: 10, step: 1 }, visibleWhen: { path: 'state.receiverLightEffect', ne: null } },
              { id: 'brightness', source: 'state.receiverLightBrightness', mutation: 'set-receiver-lighting', param: 'brightness', editor: 'modal-range', labelKey: 'receiverLighting.field.brightness', range: { min: 0, max: 100, step: 1 }, visibleWhen: { path: 'state.receiverLightEffect', ne: null } },
              { id: 'color', source: 'state.receiverLightColor', mutation: 'set-receiver-lighting', param: 'color', editor: 'modal-color', labelKey: 'receiverLighting.field.color', visibleWhen: { path: 'state.receiverLightEffect', ne: null } },
            ],
          },
        ],
        stateMapping: {
          mouseLightEnabled: 'capabilities.settings.mouseLightEnabled',
          mouseLightColor: 'capabilities.mouseLighting.color',
          mouseLightEndColor: 'capabilities.settings.mouseLightEndColor',
          mouseLightEffect: 'capabilities.mouseLighting.effect',
          mouseLightSpeed: 'capabilities.mouseLighting.speed',
          mouseLightBrightness: 'capabilities.mouseLighting.brightness',
          mouseLightExtraColor: 'capabilities.mouseLighting.extraColor',
          receiverLightEnabled: 'capabilities.receiverLighting.enabled',
          receiverLightEffect: 'capabilities.receiverLighting.effect',
          receiverLightSpeed: 'capabilities.receiverLighting.speed',
          receiverLightBrightness: 'capabilities.receiverLighting.brightness',
          receiverLightColor: 'capabilities.receiverLighting.color',
        },
      },
    },
    // 固件：只读展示（多值聚合，无旧 metadata 字段）。
    { id: 'firmware', control: 'ReadOnlyValue', labelKey: 'plugin.label.capability.firmware', readOnly: true, placements: [{ region: 'details', order: 10, span: 1, icon: 'info' }], metadata: {} },
  ],
  writableMutations: ['set-active-dpi-stage', 'set-dpi-stage-value', 'set-polling-rate', 'set-mouse-lighting', 'set-receiver-lighting', 'set-sleep'],
  evidence: 'fixture-verified', updatedAt: '00:00',
  readonly: false,
};

function mockSnapshot(overrides: Partial<DeviceSnapshot> = {}): DeviceSnapshot {
  return {
    displayName: MOCK_DEVICE.name,
    connection: MOCK_DEVICE.connection,
    batteryPercent: MOCK_DEVICE.battery,
    charging: MOCK_DEVICE.charging,
    batteries: MOCK_DEVICE.batteries,
    dpi: MOCK_DPI_STAGES.find((stage) => stage.active)?.value,
    dpiStages: MOCK_DPI_STAGES,
    pollingRateHz: MOCK_DEVICE.state.pollingRate as number,
    supportedPollingRatesHz: MOCK_SUPPORTED_POLLING_RATES,
    profile: MOCK_DEVICE.state.profile as string,
    confirmedLightColor: MOCK_DEVICE.state.mouseLightColor as string,
    capabilities: MOCK_DEVICE.capabilities,
    pluginCapabilities: MOCK_DEVICE.pluginCapabilities,
    writableMutations: MOCK_DEVICE.writableMutations,
    evidence: MOCK_DEVICE.evidence,
    readonly: MOCK_DEVICE.readonly,
    pluginId: MOCK_DEVICE.pluginId,
    ...overrides,
  };
}

export const MOCK_DEVICE_ENTRIES: DeviceSnapshotEntry[] = [
  {
    deviceKey: 'demo-wireless',
    selected: true,
    snapshot: mockSnapshot(),
  },
  {
    deviceKey: 'demo-usb',
    selected: false,
    snapshot: mockSnapshot({
      displayName: 'Mira Example USB Mouse',
      connection: 'usb',
      batteryPercent: 96,
      charging: true,
      batteries: [{ id: 'mouse', label: 'mock.mouseLabel', percentage: 96, charging: true }],
      dpi: 1600,
      dpiStages: MOCK_DPI_STAGES.map((stage) => ({
        ...stage,
        active: stage.value === 1600,
      })),
      pollingRateHz: 8000,
      profile: 'Profile 2',
      confirmedLightColor: '#8fc7b8',
      capabilities: {
        ...MOCK_DEVICE.capabilities,
        battery: { percentage: 96, charging: true, valid: true },
        dpi: {
          profile: 1,
          currentStage: 4,
          stageCount: 8,
          dpiX: [400, 800, 1000, 1600, 2400, 3200, 6400, 12800],
          dpiY: [400, 800, 1000, 1600, 2400, 3200, 6400, 12800],
          stageColors: ['#7ea7d8', '#9a8bd0', '#bf7fa8', '#d39378', '#7eb2a0', '#a8c46a', '#c9a86c', '#c77a9a'],
        },
        settings: {
          ...MOCK_DEVICE.capabilities.settings,
          profile: 1,
          pollingRate: 8000,
          mouseLightStartColor: '#8fc7b8',
          mouseLightEndColor: '#8fc7b8',
        },
        mouseLighting: {
          ...MOCK_DEVICE.capabilities.mouseLighting,
          color: '#8fc7b8',
          extraColor: '#8fc7b8',
        },
        receiverLighting: {
          ...MOCK_DEVICE.capabilities.receiverLighting,
          color: '#8fc7b8',
        },
      },
    }),
  },
];

// ─── 电量使用情况 mock 数据 ─────────────────────────────────────────────────

function mockBatteryHistoryResponse(range: BatteryHistoryRange): BatteryHistoryResponse {
  const now = new Date();
  const bucketCount = range === '24h' ? 48 : 10;

  // 鼠标：24h 从 90% 降到 82%，中间有充电段；10d 从 100% 降到 82%。
  const mousePoints = Array.from({ length: bucketCount }, (_, i) => {
    if (range === '24h') {
      // 48 个 30 分钟 bucket，halfHourAgo 表示该 bucket 距今的半小时数
      const halfHourAgo = bucketCount - 1 - i;
      const hourAgo = halfHourAgo * 0.5;
      const startPct = 90 - (hourAgo < 12 ? hourAgo * 1.0 : 12 + (hourAgo - 12) * 0.5);
      const charging = hourAgo >= 4 && hourAgo <= 5; // 2 小时充电段
      const pct = charging ? Math.min(100, startPct + 15) : Math.max(15, startPct);
      const lowBattery = !charging && pct < 20;
      const dt = new Date(now.getTime() - halfHourAgo * 1800_000);
      return {
        bucketStart: dt.toISOString(),
        bucketLabel: `${String(dt.getHours()).padStart(2, '0')}:${String(dt.getMinutes()).padStart(2, '0')}`,
        percentage: Math.round(pct),
        minPercentage: Math.round(pct - 2),
        maxPercentage: Math.round(pct + 2),
        charging,
        lowBattery,
        sampleCount: 3 + (i % 4),
      };
    }
    // 10d
    const dayAgo = bucketCount - 1 - i;
    const pct = Math.max(20, 100 - dayAgo * 2 - 5);
    const charging = dayAgo === 5;
    const lowBattery = !charging && pct < 20;
    const day = new Date(now.getTime() - dayAgo * 86400_000);
    return {
      bucketStart: day.toISOString(),
      bucketLabel: `${String(day.getMonth() + 1).padStart(2, '0')}-${String(day.getDate()).padStart(2, '0')}`,
      percentage: pct,
      minPercentage: Math.max(0, pct - 5),
      maxPercentage: Math.min(100, pct + 3),
      charging,
      lowBattery,
      sampleCount: 8 + (i % 5),
    };
  });

  // 接收器：电量稳定在 95-100%。
  const receiverPoints = Array.from({ length: bucketCount }, (_, i) => {
    const ago = range === '24h' ? bucketCount - 1 - i : bucketCount - 1 - i;
    const interval = range === '24h' ? 1800_000 : 86400_000;
    const dt = new Date(now.getTime() - ago * interval);
    const hourAgo = range === '24h' ? ago * 0.5 : ago;
    const pct = 100 - (range === '24h' ? hourAgo * 0.1 : ago * 0.5);
    return {
      bucketStart: dt.toISOString(),
      bucketLabel: range === '24h'
        ? `${String(dt.getHours()).padStart(2, '0')}:${String(dt.getMinutes()).padStart(2, '0')}`
        : `${String(dt.getMonth() + 1).padStart(2, '0')}-${String(dt.getDate()).padStart(2, '0')}`,
      percentage: Math.round(pct),
      minPercentage: Math.round(pct - 1),
      maxPercentage: 100,
      charging: false,
      lowBattery: false,
      sampleCount: 2 + (i % 3),
    };
  });

  return {
    range,
    devices: [
      {
        key: 'mouse:abc123:mouse',
        deviceId: 'abc123',
        deviceName: 'Mira Example Wireless Mouse',
        connection: 'wireless',
        componentId: 'mouse',
        componentLabel: 'mock.mouseLabel',
        latestPercentage: 82,
        latestCharging: false,
        latestAt: now.toISOString(),
        lowBattery: false,
      },
      {
        key: 'mouse:abc123:receiver',
        deviceId: 'abc123',
        deviceName: 'Mira Example Wireless Mouse',
        connection: 'wireless',
        componentId: 'receiver',
        componentLabel: 'mock.receiverLabel',
        latestPercentage: 96,
        latestCharging: false,
        latestAt: now.toISOString(),
        lowBattery: false,
      },
    ],
    series: [
      { key: 'mouse:abc123:mouse', points: mousePoints },
      { key: 'mouse:abc123:receiver', points: receiverPoints },
    ],
    insights: [
      {
        type: 'estimatedRemaining',
        severity: 'info',
        title: 'estimatedRemaining',
        message: range === '24h' ? 'remainingDaysHours|3|6' : 'remainingDaysHours|4|2',
        deviceKey: 'mouse:abc123:mouse',
      },
      {
        type: 'estimatedRunout',
        severity: 'info',
        title: 'estimatedRunout',
        message: '07-08 14:00',
        deviceKey: 'mouse:abc123:mouse',
      },
      {
        type: 'chargingHabit',
        severity: 'info',
        title: 'chargingHabit',
        message: 'chargingHabitStartEnd|18|92|3',
        deviceKey: 'mouse:abc123:mouse',
      },
      {
        type: 'batteryConsistency',
        severity: 'info',
        title: 'batteryConsistency',
        message: 'consistencyStable',
        deviceKey: 'mouse:abc123:mouse',
      },
      {
        type: 'averageDailyDrain',
        severity: 'info',
        title: 'averageDailyDrain',
        message: `averageDailyDrain|${range === '24h' ? '2.3' : '1.8'}`,
        deviceKey: 'mouse:abc123:mouse',
      },
      {
        type: 'chargingCount',
        severity: 'info',
        title: 'chargingCount',
        message: `chargingCount|${range === '24h' ? '1' : '6'}`,
        deviceKey: 'mouse:abc123:mouse',
      },
    ],
    generatedAt: now.toISOString(),
  };
}

export const MOCK_BATTERY_HISTORY_24H: BatteryHistoryResponse = mockBatteryHistoryResponse('24h');
export const MOCK_BATTERY_HISTORY_10D: BatteryHistoryResponse = mockBatteryHistoryResponse('10d');
