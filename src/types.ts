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
  /** 设备实际是否支持该能力（运行时探测结果）。默认 true（向后兼容）。 */
  available?: boolean;
  /** 连接类型能力分支（#3）：声明该能力仅在指定连接类型下可见。 */
  connections?: string[];
  /** 固件版本门槛（#4）：声明该能力所需的最低固件版本。 */
  minFirmware?: string;
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
  connection: 'usb' | 'wireless' | 'bluetooth' | 'virtual';
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
  /** 插件未签名/未启用写入时为 true，UI 显示只读模式标记。 */
  readonly: boolean;
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
  source?: 'bundled' | 'installed';
}

export interface PluginUpdateInfo {
  pluginId: string;
  currentVersion: string;
  availableVersion?: string;
  releaseTag?: string;
  notes?: string;
  updateAvailable: boolean;
}

export interface PluginInstallResult {
  pluginId: string;
  version: string;
  previousVersion: string;
  restartedRuntime: boolean;
}

export interface ContactLinks {
  github?: string;
  repository?: string;
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
  language: 'auto' | 'zh-CN' | 'en';
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
  nightModeTriggerTime: boolean;
  nightModeTriggerTheme: boolean;
  nightModeThemeDark: boolean;
  nightModeTriggerCharging: boolean;
  nightModeTriggerLowBattery: boolean;
  nightModeTargetMouse: boolean;
  nightModeTargetReceiver: boolean;
  refreshIntervalSeconds: number;
  telemetryDisabled: boolean;
  automaticUpdateChecks: boolean;
  automaticUpdateInstall: boolean;
  automaticPluginUpdateChecks: boolean;
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
  /** 插件未签名/签名失效/未启用写入时为 true，UI 显示只读模式标记。 */
  readonly?: boolean;
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
