// SPDX-License-Identifier: AGPL-3.0-or-later
export type Evidence = 'source-confirmed' | 'fixture-verified' | 'build-verified' | 'hardware-verified' | 'inferred' | 'unknown' | 'blocked';
export type ThemeMode = 'system' | 'light' | 'dark';
export interface DpiStage { value: number; color: string; active: boolean; enabled: boolean }
export interface DeviceBattery { id: string; label: string; percentage: number; charging?: boolean }
export interface Lighting {
  enabled: boolean;
  mode: string;
  color?: string;
  supportsSpeed: boolean;
  supportsBrightness: boolean;
  receiverLinked: boolean;
  /** 鼠标主灯效颜色，必须来自转发到鼠标的灯光读取。 */
  mouseLightEnabled?: boolean;
  mouseLightColor?: string;
  mouseLightEndColor?: string;
  receiverLightEnabled?: boolean;
  receiverLightMode?: string;
  receiverLightColor?: string;
}
export type DeviceCapabilities = Record<string, Record<string, unknown>>;
export type PluginControl = 'Toggle' | 'Segmented' | 'Select' | 'Slider' | 'Number' | 'Color' | 'GradientStops' | 'DpiStages' | 'LightingZone' | 'ReadOnlyValue' | 'Action' | 'Info';
export interface PluginCapability {
  id: string;
  control: PluginControl;
  labelKey: string;
  readOnly: boolean;
  placements?: PluginCapabilityPlacement[];
  metadata: Record<string, unknown>;
}
export interface PluginCapabilityPlacement {
  region: 'hero' | 'control' | 'status' | 'details';
  group?: string;
  order: number;
  span: number;
  icon?: string;
}
export interface DeviceState {
  name: string;
  connection: 'USB' | '无线' | '蓝牙' | '虚拟';
  battery?: number;
  charging?: boolean;
  batteries: DeviceBattery[];
  pollingRate?: number;
  supportedPollingRates?: number[];
  profile?: string;
  dpiStages: DpiStage[];
  lighting?: Lighting;
  capabilities: DeviceCapabilities;
  pluginCapabilities: PluginCapability[];
  writableMutations: string[];
  evidence: Evidence;
  updatedAt: string;
}

export interface BundledPluginInfo {
  pluginId: string;
  version: string;
  asset: string;
  sha256: string;
  publisherKeyId: string;
  releaseTag: string;
  bundleByDefault: boolean;
  signatureVerified: boolean;
  evidence: string;
}

export interface ContactLinks {
  github?: string;
  x?: string;
  telegram?: string;
  developerName?: string;
  copyright?: string;
}

export interface AboutInfo {
  name: string;
  version: string;
  identifier: string;
  platform: string;
  architecture: string;
  rustVersion: string;
  buildDate: string;
  gitCommit: string;
  bundledPlugins: BundledPluginInfo[];
  contact: ContactLinks;
  updaterActive: boolean;
}

export interface AppSettings {
  theme: ThemeMode;
  autostart: boolean;
  startHidden: boolean;
  trayShowBatteryTitle: boolean;
  trayIncludeReceiverBattery: boolean;
  trayShowConnection: boolean;
  trayIconColor: string;
  lowBatteryThreshold: number;
  nightModeEnabled: boolean;
  nightModeStart: string;
  nightModeEnd: string;
  refreshIntervalSeconds: number;
  telemetryDisabled: boolean;
}

export interface DeviceSnapshot {
  displayName: string;
  connection: 'usb' | 'wireless' | 'bluetooth' | 'virtual';
  batteryPercent?: number;
  charging?: boolean;
  batteries?: DeviceBattery[];
  dpi?: number;
  dpiStages?: DpiStage[];
  pollingRateHz?: number;
  supportedPollingRatesHz?: number[];
  profile?: string;
  confirmedLightColor?: string;
  capabilities?: DeviceCapabilities;
  pluginCapabilities?: PluginCapability[];
  writableMutations?: string[];
  evidence: Evidence;
}

export interface DiscoveredDevice {
  pluginId: string;
  family: string;
  connection: string;
  evidence: string;
  path: string;
  vendorId: number;
  productId: number;
  usagePage: number;
  usage: number;
}
