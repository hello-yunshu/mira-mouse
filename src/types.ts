// SPDX-License-Identifier: AGPL-3.0-or-later
export type Evidence = 'source-confirmed' | 'fixture-verified' | 'build-verified' | 'hardware-verified' | 'inferred' | 'unknown' | 'blocked';
export type ThemeMode = 'system' | 'light' | 'dark';
export interface DpiStage { value: number; color: string; active: boolean; enabled: boolean }
export interface Lighting { enabled: boolean; mode: string; color?: string; supportsSpeed: boolean; supportsBrightness: boolean; receiverLinked: boolean }
export interface DeviceState {
  name: string;
  connection: 'USB' | '无线' | '蓝牙' | '虚拟';
  battery?: number;
  charging: boolean;
  pollingRate?: number;
  profile?: string;
  dpiStages: DpiStage[];
  lighting?: Lighting;
  evidence: Evidence;
  updatedAt: string;
}

