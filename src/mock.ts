// SPDX-License-Identifier: AGPL-3.0-or-later
// Explicit test/development boundary. Production must obtain snapshots from Tauri commands.
import type { DeviceState } from './types';
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
    enabled: true, mode: 'mock.breathing', color: '#D8B0B7', supportsSpeed: true, supportsBrightness: true, receiverLinked: true,
    mouseLightEnabled: true, mouseLightColor: '#D8B0B7', mouseLightEndColor: '#D8B0B7',
    receiverLightEnabled: true, receiverLightMode: 'mock.neon', receiverLightColor: '#D8B0B7',
  },
  capabilities: {
    battery: { percentage: 82, charging: false, valid: true },
    dpi: { profile: 0, currentStage: 3, stageCount: 8, dpiX: [400, 800, 1000, 1600, 2400, 3200, 6400, 12800], dpiY: [400, 800, 1000, 1600, 2400, 3200, 6400, 12800], stageColors: ['#7ea7d8', '#9a8bd0', '#bf7fa8', '#d39378', '#7eb2a0', '#a8c46a', '#c9a86c', '#c77a9a'] },
    settings: { profile: 0, pollingRaw: 1, pollingRate: 1000, usbDebounce: 4, wirelessDebounce: 4, bluetoothDebounce: 4, rippleCorrection: true, angleSnap: false, motionSync: true, liftCutOff: 1, buttonChangeTime: 12, wheelToButton: 0, buttonToWheel: 0, bluetoothSleepValue: 600, wirelessSleepValue: 300, mouseLightStartColor: '#D8B0B7', mouseLightEndColor: '#D8B0B7', mouseLightEnabled: true },
    receiverLighting: { effect: 3, effectName: '霓虹', speed: 2, brightness: 70, option: 7, color: '#D8B0B7' },
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
    { id: 'dpi', control: 'DpiStages', labelKey: 'capability.dpi', readOnly: false, placements: [{ region: 'control', group: 'performance', order: 10, span: 1, icon: 'gauge' }], metadata: { label: 'DPI', section: 'control', source: 'dpiStages', mutations: { select: 'set-dpi-stage', value: 'set-dpi-value' } } },
    { id: 'polling-rate', control: 'Select', labelKey: 'capability.polling-rate', readOnly: false, placements: [{ region: 'control', group: 'polling', order: 20, span: 1, icon: 'wave' }], metadata: { section: 'control', source: 'pollingRate', mutation: 'set-polling-rate', param: 'rate', unit: 'Hz', options: [125, 250, 500, 1000, 2000, 4000, 8000].map((value) => ({ value, label: `${value} Hz` })), summary: [{ label: 'mock.motionSync', source: 'capabilities.settings.motionSync' }, { label: 'mock.angleSnap', source: 'capabilities.settings.angleSnap' }, { label: 'mock.liftCutOff', source: 'capabilities.settings.liftCutOff' }] } },
    {
      id: 'sleep-time', control: 'Number', labelKey: 'capability.sleep-time', readOnly: false,
      placements: [{ region: 'status', order: 10, span: 1, icon: 'timer' }],
      metadata: {
        section: 'status', status: true, format: 'sleep',
        bindings: [
          { when: { path: 'connection', eq: 'bluetooth' }, label: 'mock.bluetoothSleep', source: 'capabilities.settings.bluetoothSleepValue', mutation: 'set-bluetooth-sleep-time', param: 'seconds' },
          { when: { path: 'connection', eq: 'wireless' }, label: 'mock.wirelessSleep', source: 'capabilities.settings.wirelessSleepValue', mutation: 'set-wireless-sleep-time', param: 'seconds' },
          { when: { path: 'connection', eq: 'virtual' }, label: 'mock.wirelessSleep', source: 'capabilities.settings.wirelessSleepValue', mutation: 'set-wireless-sleep-time', param: 'seconds' },
        ],
      },
    },
    { id: 'profile', control: 'ReadOnlyValue', labelKey: 'capability.profile', readOnly: true, placements: [{ region: 'status', order: 20, span: 1, icon: 'profile' }], metadata: { section: 'status', status: true, source: 'profile' } },
    { id: 'lighting', control: 'LightingZone', labelKey: 'capability.lighting', readOnly: false, placements: [{ region: 'control', group: 'lighting', order: 30, span: 1, icon: 'lightbulb' }, { region: 'status', order: 30, span: 1, icon: 'lightbulb' }], metadata: { section: 'control', status: true, source: 'lighting.mouseLightColor', mutations: { mouse: 'set-mouse-lighting', receiver: 'set-receiver-lighting' } } },
    { id: 'firmware', control: 'ReadOnlyValue', labelKey: 'capability.firmware', readOnly: true, placements: [{ region: 'details', order: 10, span: 1, icon: 'info' }], metadata: {} },
  ],
  writableMutations: ['set-dpi-stage', 'set-dpi-value', 'set-polling-rate', 'set-mouse-lighting', 'set-receiver-lighting', 'set-wireless-sleep-time', 'set-bluetooth-sleep-time'],
  readonly: false,
};
