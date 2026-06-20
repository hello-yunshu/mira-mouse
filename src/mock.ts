// SPDX-License-Identifier: AGPL-3.0-or-later
// Explicit test/development boundary. Production must obtain snapshots from Tauri commands.
import type { DeviceState } from './types';
export const MOCK_DEVICE: DeviceState = {
  name: 'Mira Example Wireless Mouse', connection: '虚拟', battery: 82, charging: false,
  batteries: [
    { id: 'mouse', label: '鼠标', percentage: 82, charging: false },
    { id: 'receiver', label: '接收器', percentage: 100, charging: false },
  ],
  pollingRate: 1000, profile: 'Profile 1', evidence: 'fixture-verified', updatedAt: '刚刚',
  dpiStages: [
    { value: 400, color: '#7ea7d8', enabled: true, active: false },
    { value: 800, color: '#9a8bd0', enabled: true, active: false },
    { value: 1000, color: '#bf7fa8', enabled: true, active: true },
    { value: 1600, color: '#d39378', enabled: true, active: false },
    { value: 3200, color: '#7eb2a0', enabled: true, active: false },
  ],
  lighting: {
    enabled: true, mode: '呼吸', color: '#D8B0B7', supportsSpeed: true, supportsBrightness: true, receiverLinked: true,
    mouseLightEnabled: true, mouseLightColor: '#D8B0B7', mouseLightEndColor: '#D8B0B7',
    receiverLightEnabled: true, receiverLightMode: '霓虹', receiverLightColor: '#D8B0B7',
  },
  capabilities: {
    battery: { percentage: 82, charging: false, valid: true },
    dpi: { profile: 0, currentStage: 3, stageCount: 5, dpiX: [400, 800, 1000, 1600, 3200], dpiY: [400, 800, 1000, 1600, 3200], stageColors: ['#7ea7d8', '#9a8bd0', '#bf7fa8', '#d39378', '#7eb2a0'] },
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
};
