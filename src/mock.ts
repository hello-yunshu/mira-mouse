// SPDX-License-Identifier: AGPL-3.0-or-later
// Explicit test/development boundary. Production must obtain snapshots from Tauri commands.
import type { BatteryHistoryResponse, BatteryHistoryRange, DeviceSnapshot, DeviceSnapshotEntry, DeviceState } from './types';
import { DEFAULT_THEME_ACCENT } from './theme';

export const MOCK_DEVICE: DeviceState = {
  name: 'Mira Example Wireless Mouse', connection: 'virtual', battery: 82, charging: false,
  batteries: [
    { id: 'mouse', label: 'mock.mouseLabel', percentage: 82, charging: false },
    { id: 'receiver', label: 'mock.receiverLabel', percentage: 100, charging: false },
  ],
  pollingRate: 1000, supportedPollingRates: [125, 250, 500, 1000, 2000, 4000, 8000], profile: 'Profile 1', evidence: 'fixture-verified', updatedAt: '00:00',
  dpiStages: [
    { value: 400, color: '#7ea7d8', enabled: true, active: false },
    { value: 800, color: '#9a8bd0', enabled: true, active: false },
    { value: 1000, color: '#bf7fa8', enabled: true, active: true },
    { value: 1600, color: '#d39378', enabled: true, active: false },
    { value: 2400, color: '#7eb2a0', enabled: true, active: false },
    { value: 3200, color: '#a8c46a', enabled: true, active: false },
    { value: 6400, color: '#c9a86c', enabled: true, active: false },
    { value: 12800, color: '#c77a9a', enabled: true, active: false },
  ],
  lighting: {
    enabled: true, mode: 'mock.breathing', color: DEFAULT_THEME_ACCENT, supportsSpeed: true, supportsBrightness: true, receiverLinked: true,
    mouseLightEnabled: true, mouseLightColor: DEFAULT_THEME_ACCENT, mouseLightEndColor: DEFAULT_THEME_ACCENT,
    receiverLightEnabled: true, receiverLightMode: 'mock.neon', receiverLightColor: DEFAULT_THEME_ACCENT,
  },
  capabilities: {
    battery: { percentage: 82, charging: false, valid: true },
    dpi: { profile: 0, currentStage: 3, stageCount: 8, dpiX: [400, 800, 1000, 1600, 2400, 3200, 6400, 12800], dpiY: [400, 800, 1000, 1600, 2400, 3200, 6400, 12800], stageColors: ['#7ea7d8', '#9a8bd0', '#bf7fa8', '#d39378', '#7eb2a0', '#a8c46a', '#c9a86c', '#c77a9a'] },
    settings: { profile: 0, pollingRaw: 1, pollingRate: 1000, usbDebounce: 4, wirelessDebounce: 4, bluetoothDebounce: 4, rippleCorrection: true, angleSnap: false, motionSync: true, liftCutOff: 1, buttonChangeTime: 12, wheelToButton: 0, buttonToWheel: 0, bluetoothSleepValue: 600, wirelessSleepValue: 300, mouseLightStartColor: DEFAULT_THEME_ACCENT, mouseLightEndColor: DEFAULT_THEME_ACCENT, mouseLightEnabled: true },
    receiverLighting: { effect: 3, effectName: '霓虹', speed: 2, brightness: 70, option: 7, color: DEFAULT_THEME_ACCENT },
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
    { id: 'battery', control: 'ReadOnlyValue', labelKey: 'capability.battery', readOnly: true, placements: [{ region: 'hero', order: 10, span: 1, icon: 'battery' }], metadata: {} },
    { id: 'dpi', control: 'DpiStages', labelKey: 'capability.dpi', readOnly: false, placements: [{ region: 'control', group: 'performance', order: 10, span: 1, icon: 'gauge' }], metadata: { label: 'DPI', section: 'control', source: 'dpiStages', mutations: { select: 'set-dpi-stage', value: 'set-dpi-value' }, range: { min: 50, max: 30000, step: 50 } } },
    { id: 'polling-rate', control: 'Select', labelKey: 'capability.polling-rate', readOnly: false, placements: [{ region: 'control', group: 'polling', order: 20, span: 1, icon: 'wave' }], metadata: { section: 'control', source: 'pollingRate', mutation: 'set-polling-rate', param: 'rate', unit: 'Hz', options: [125, 250, 500, 1000, 2000, 4000, 8000].map((value) => ({ value, label: `${value} Hz` })), summary: [{ label: 'mock.motionSync', source: 'capabilities.settings.motionSync' }, { label: 'mock.angleSnap', source: 'capabilities.settings.angleSnap' }, { label: 'mock.liftCutOff', source: 'capabilities.settings.liftCutOff' }] } },
    {
      id: 'sleep-time', control: 'Number', labelKey: 'capability.sleep-time', readOnly: false,
      placements: [{ region: 'status', order: 10, span: 1, icon: 'timer' }],
      metadata: {
        section: 'status', status: true, format: 'sleep', range: { min: 10, max: 65535, step: 10 },
        bindings: [
          { when: { path: 'connection', eq: 'bluetooth' }, label: 'mock.bluetoothSleep', source: 'capabilities.settings.bluetoothSleepValue', mutation: 'set-bluetooth-sleep-time', param: 'seconds' },
          { when: { path: 'connection', eq: 'wireless' }, label: 'mock.wirelessSleep', source: 'capabilities.settings.wirelessSleepValue', mutation: 'set-wireless-sleep-time', param: 'seconds' },
          { when: { path: 'connection', eq: 'virtual' }, label: 'mock.wirelessSleep', source: 'capabilities.settings.wirelessSleepValue', mutation: 'set-wireless-sleep-time', param: 'seconds' },
        ],
      },
    },
    { id: 'profile', control: 'ReadOnlyValue', labelKey: 'capability.profile', readOnly: true, placements: [{ region: 'status', order: 20, span: 1, icon: 'profile' }], metadata: { section: 'status', status: true, source: 'profile' } },
    { id: 'lighting', control: 'LightingZone', labelKey: 'capability.lighting', readOnly: false, placements: [{ region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' }, { region: 'status', order: 30, span: 1, icon: 'lightbulb' }], metadata: { section: 'control', status: true, source: 'capabilities.mouseLighting.color', lightingRole: { mouse: 'set-mouse-lighting', receiver: 'set-receiver-lighting' } } },
    { id: 'firmware', control: 'ReadOnlyValue', labelKey: 'capability.firmware', readOnly: true, placements: [{ region: 'details', order: 10, span: 1, icon: 'info' }], metadata: {} },
  ],
  writableMutations: ['set-dpi-stage', 'set-dpi-value', 'set-polling-rate', 'set-mouse-lighting', 'set-receiver-lighting', 'set-wireless-sleep-time', 'set-bluetooth-sleep-time'],
  readonly: false,
};

function mockSnapshot(overrides: Partial<DeviceSnapshot> = {}): DeviceSnapshot {
  return {
    displayName: MOCK_DEVICE.name,
    connection: MOCK_DEVICE.connection,
    batteryPercent: MOCK_DEVICE.battery,
    charging: MOCK_DEVICE.charging,
    batteries: MOCK_DEVICE.batteries,
    dpi: MOCK_DEVICE.dpiStages.find((stage) => stage.active)?.value,
    dpiStages: MOCK_DEVICE.dpiStages,
    pollingRateHz: MOCK_DEVICE.pollingRate,
    supportedPollingRatesHz: MOCK_DEVICE.supportedPollingRates,
    profile: MOCK_DEVICE.profile,
    confirmedLightColor: MOCK_DEVICE.lighting?.mouseLightColor,
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
      dpiStages: MOCK_DEVICE.dpiStages.map((stage) => ({
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
