// SPDX-License-Identifier: AGPL-3.0-or-later
// Explicit test/development boundary. Production must obtain snapshots from Tauri commands.
import type { DeviceState } from './types';
export const MOCK_DEVICE: DeviceState = {
  name: 'Mira Example Wireless Mouse', connection: '虚拟', battery: 82, charging: false,
  pollingRate: 1000, profile: 'Profile 1', evidence: 'fixture-verified', updatedAt: '刚刚',
  dpiStages: [
    { value: 400, color: '#7ea7d8', enabled: true, active: false },
    { value: 800, color: '#9a8bd0', enabled: true, active: false },
    { value: 1000, color: '#bf7fa8', enabled: true, active: true },
    { value: 1600, color: '#d39378', enabled: true, active: false },
    { value: 3200, color: '#7eb2a0', enabled: true, active: false },
  ],
  lighting: { enabled: true, mode: '渐变', color: '#b87ab0', supportsSpeed: true, supportsBrightness: true, receiverLinked: true },
};

