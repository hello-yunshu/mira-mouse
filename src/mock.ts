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
const MOCK_RECEIVER_LIGHT_COLOR = '#4BBFB1';

// 灯效选项：mouse 与 receiver 共用同一套声明式选项。
const LIGHTING_EFFECT_OPTIONS: PluginFieldOption[] = [
  { value: 0, labelKey: 'receiverLighting.effect.off' },
  { value: 1, labelKey: 'receiverLighting.effect.fixed' },
  { value: 2, labelKey: 'receiverLighting.effect.breathing' },
  { value: 3, labelKey: 'receiverLighting.effect.neon' },
  { value: 4, labelKey: 'receiverLighting.effect.wave' },
];
const RECEIVER_COLOR_MODE_OPTIONS: PluginFieldOption[] = [
  { value: 0, labelKey: 'receiverLighting.colorMode.red' },
  { value: 1, labelKey: 'receiverLighting.colorMode.orange' },
  { value: 2, labelKey: 'receiverLighting.colorMode.yellow' },
  { value: 3, labelKey: 'receiverLighting.colorMode.green' },
  { value: 4, labelKey: 'receiverLighting.colorMode.cyan' },
  { value: 5, labelKey: 'receiverLighting.colorMode.blue' },
  { value: 6, labelKey: 'receiverLighting.colorMode.purple' },
  { value: 7, labelKey: 'receiverLighting.colorMode.custom' },
  { value: 8, labelKey: 'receiverLighting.colorMode.colorful' },
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
    receiverLightColor: MOCK_RECEIVER_LIGHT_COLOR,
    // 休眠时间
    wirelessSleepValue: 300,
    bluetoothSleepValue: 600,
  },
  capabilities: {
    battery: { percentage: 82, charging: false, valid: true },
    dpi: { profile: 0, currentStage: 3, stageCount: 8, dpiX: [400, 800, 1000, 1600, 2400, 3200, 6400, 12800], dpiY: [400, 800, 1000, 1600, 2400, 3200, 6400, 12800], stageColors: ['#7ea7d8', '#9a8bd0', '#bf7fa8', '#d39378', '#7eb2a0', '#a8c46a', '#c9a86c', '#c77a9a'] },
    settings: { profile: 0, pollingRaw: 1, pollingRate: 1000, usbDebounce: 4, wirelessDebounce: 4, bluetoothDebounce: 4, rippleCorrection: true, angleSnap: false, motionSync: true, liftCutOff: 1, buttonChangeTime: 12, wheelToButton: 0, buttonToWheel: 0, bluetoothSleepValue: 600, wirelessSleepValue: 300, mouseLightStartColor: DEFAULT_THEME_ACCENT, mouseLightEndColor: DEFAULT_THEME_ACCENT, mouseLightEnabled: true },
    mouseLighting: { effect: 2, effectName: '呼吸', speed: 2, brightness: 70, color: DEFAULT_THEME_ACCENT, extraColor: DEFAULT_THEME_ACCENT, enabled: true },
    receiverLighting: { effect: 3, effectName: '霓虹', speed: 2, brightness: 70, option: 7, optionName: '模式 7', color: MOCK_RECEIVER_LIGHT_COLOR, enabled: true },
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
        summary: [
          { labelKey: 'mock.motionSync', source: 'capabilities.settings.motionSync' },
          { labelKey: 'mock.angleSnap', source: 'capabilities.settings.angleSnap' },
          { labelKey: 'mock.liftCutOff', source: 'capabilities.settings.liftCutOff' },
        ],
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
          editor: 'modal-number',
          format: 'sleep',
          range: { min: 0, max: 1800, step: 30 },
          labelKey: 'mock.wirelessSleep',
          editTitleKey: 'dashboard.setSleepTitle',
          editLabelKey: 'dashboard.timeoutSeconds',
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
        accentSource: 'state.mouseLightColor',
        zones: [
          {
            id: 'mouse',
            labelKey: 'lighting.mouse',
            fields: [
              { id: 'status', source: 'capabilities.settings.mouseLightEnabled', mutation: 'set-mouse-lighting', param: 'enabled', editor: 'inline-toggle', switch: { source: 'capabilities.settings.mouseLightEnabled', offValue: false }, labelKey: 'dashboard.status', paramSources: { color: 'state.mouseLightColor', enabled: 'capabilities.settings.mouseLightEnabled' } },
              { id: 'color', source: 'state.mouseLightColor', mutation: 'set-mouse-lighting', param: 'color', editor: 'modal-color', format: 'color', labelKey: 'receiverLighting.field.color', editTitleKey: 'dashboard.mouseLightColor', paramSources: { color: 'state.mouseLightColor', enabled: 'capabilities.settings.mouseLightEnabled' } },
            ],
          },
          {
            id: 'receiver',
            labelKey: 'lighting.receiver',
            visibleWhen: { path: 'capabilities.receiverLighting', ne: null },
            fields: [
              { id: 'effect', source: 'state.receiverLightEffect', mutation: 'set-receiver-lighting', param: 'effect', editor: 'modal-select', labelKey: 'receiverLighting.field.effect', labelSource: 'capabilities.receiverLighting.effectName', editTitleKey: 'dashboard.editReceiverLightingTitle', options: LIGHTING_EFFECT_OPTIONS, visibleWhen: { path: 'state.receiverLightEffect', ne: null } },
              { id: 'option', source: 'capabilities.receiverLighting.option', mutation: 'set-receiver-lighting', param: 'option', editor: 'modal-select', labelKey: 'receiverLighting.field.option', labelSource: 'capabilities.receiverLighting.optionName', editTitleKey: 'dashboard.editReceiverLightingTitle', options: RECEIVER_COLOR_MODE_OPTIONS },
              { id: 'speed', source: 'state.receiverLightSpeed', mutation: 'set-receiver-lighting', param: 'speed', editor: 'modal-range', labelKey: 'receiverLighting.field.speed', editTitleKey: 'dashboard.editReceiverLightingTitle', range: { min: 0, max: 10, step: 1 }, visibleWhen: { path: 'state.receiverLightEffect', ne: null } },
              { id: 'brightness', source: 'state.receiverLightBrightness', mutation: 'set-receiver-lighting', param: 'brightness', editor: 'modal-range', labelKey: 'receiverLighting.field.brightness', editTitleKey: 'dashboard.editReceiverLightingTitle', format: 'percent', range: { min: 0, max: 100, step: 1 }, visibleWhen: { path: 'state.receiverLightEffect', ne: null } },
              { id: 'color', source: 'state.receiverLightColor', mutation: 'set-receiver-lighting', param: 'color', editor: 'modal-color', labelKey: 'receiverLighting.field.color', editTitleKey: 'dashboard.editReceiverLightingTitle', visibleWhen: { path: 'state.receiverLightEffect', ne: null } },
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
        statusDisplay: {
          labelKey: 'plugin.label.capability.lighting',
          valueSource: 'state.mouseLightEffect',
          valueOptions: [
            { value: 0, labelKey: 'lighting.off' },
            { value: 1, labelKey: 'lighting.on' },
            { value: 2, labelKey: 'lighting.on' },
            { value: 3, labelKey: 'lighting.on' },
          ],
          onClickField: 'color',
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
          color: MOCK_RECEIVER_LIGHT_COLOR,
        },
      },
    }),
  },
];

// ─── 电量使用情况 mock 数据 ─────────────────────────────────────────────────

function mockBatteryHistoryResponse(range: BatteryHistoryRange): BatteryHistoryResponse {
  const now = new Date();
  const bucketCount = range === '24h' ? 48 : 30;

  // 鼠标：24h 从 90% 降到 82%，9-7 小时前有充电段；10d 从 100% 降到 82%，第 5 天有充电段。
  const mousePoints = Array.from({ length: bucketCount }, (_, i) => {
    if (range === '24h') {
      // 48 个 30 分钟 bucket，halfHourAgo 表示该 bucket 距今的半小时数
      const halfHourAgo = bucketCount - 1 - i;
      const hourAgo = halfHourAgo * 0.5;
      // 分三段：24h→9h 线性下降 90→78；9-7h 充电 78→92；7h→now 下降 92→82
      let pct: number;
      const charging = hourAgo >= 7 && hourAgo <= 9;
      if (charging) {
        pct = 78 + ((9 - hourAgo) / 2) * 14;
      } else if (hourAgo > 9) {
        pct = 90 - (24 - hourAgo) * (12 / 15);
      } else {
        pct = 92 - (7 - hourAgo) * (10 / 7);
      }
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
    // 10d：每天 3 个 8 小时时段，共 30 个 bucket。
    const slotAgo = bucketCount - 1 - i;
    const dayAgo = Math.floor(slotAgo / 3);
    const slotInDay = slotAgo % 3; // 2=当天最早, 1=中间, 0=当天最晚
    // 分三段：day9→5前 100→86；day5中间充电 86→97；day5后→now 97→82
    const isChargingSlot = dayAgo === 5 && slotInDay === 1;
    let pct: number;
    if (isChargingSlot) {
      pct = 97;
    } else if (dayAgo > 5 || (dayAgo === 5 && slotInDay === 2)) {
      pct = 100 - (9 - dayAgo) * 3.5;
    } else {
      pct = 97 - (5 - dayAgo) * 3;
    }
    const charging = isChargingSlot;
    const lowBattery = !charging && pct < 20;
    const day = new Date(now.getTime() - slotAgo * 8 * 3600_000);
    const startHour = Math.floor(day.getHours() / 8) * 8;
    const label = `${String(day.getMonth() + 1).padStart(2, '0')}-${String(day.getDate()).padStart(2, '0')} ${String(startHour).padStart(2, '0')}:00–${String(startHour + 8).padStart(2, '0')}:00`;
    return {
      bucketStart: day.toISOString(),
      bucketLabel: label,
      percentage: Math.round(pct),
      minPercentage: Math.max(0, Math.round(pct - 5)),
      maxPercentage: Math.min(100, Math.round(pct + 3)),
      charging,
      lowBattery,
      sampleCount: 8 + (i % 5),
    };
  });

  // 接收器：电量稳定在 96-100%。
  const receiverPoints = Array.from({ length: bucketCount }, (_, i) => {
    const ago = bucketCount - 1 - i;
    const interval = range === '24h' ? 1800_000 : 8 * 3600_000;
    const dt = new Date(now.getTime() - ago * interval);
    const hourAgo = range === '24h' ? ago * 0.5 : ago;
    // 接收器从满电缓慢下降至 96%
    const pct = range === '24h'
      ? 96 + hourAgo * (4 / 24)
      : 96 + Math.floor(ago / 3) * (4 / 9);
    return {
      bucketStart: dt.toISOString(),
      bucketLabel: range === '24h'
        ? `${String(dt.getHours()).padStart(2, '0')}:${String(dt.getMinutes()).padStart(2, '0')}`
        : `${String(dt.getMonth() + 1).padStart(2, '0')}-${String(dt.getDate()).padStart(2, '0')} ${String(Math.floor(dt.getHours() / 8) * 8).padStart(2, '0')}:00–${String(Math.floor(dt.getHours() / 8) * 8 + 8).padStart(2, '0')}:00`,
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
