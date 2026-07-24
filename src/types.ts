// SPDX-License-Identifier: AGPL-3.0-or-later
export type Evidence = 'source-confirmed' | 'fixture-verified' | 'build-verified' | 'hardware-verified' | 'inferred' | 'unknown' | 'blocked';
export type ThemeMode = 'system' | 'light' | 'dark';
export interface DpiStage { value: number; color: string; active: boolean; enabled: boolean }
export interface DeviceBattery { id: string; label: string; percentage: number; charging?: boolean }
export interface DeviceIdentity { group: string; displayName?: string; aliases?: string[] }

/** Per-output read status from the workflow engine. */
export type ReadStatus = 'ok' | 'skipped' | 'not-supported' | { failed: string };

/** 灯效范围声明（speed/brightness）。 */
export interface RangeSpec {
  min: number;
  max: number;
  step?: number;
}

// ─── 声明式插件 UI 类型 ─────────────────────────────────────────────────────

/** 字段编辑器类型，决定 UI 渲染方式。 */
export type PluginEditor = 'inline-toggle' | 'inline-segmented' | 'inline-value' | 'inline-action' | 'modal-select' | 'modal-color' | 'modal-range' | 'modal-number' | 'modal-dpi-stage' | 'modal-gradient' | 'static-readonly';

/** 字段值格式化方式。 */
export type PluginFieldFormat = 'sleep' | 'percent' | 'hertz' | 'connection' | 'color' | 'default';

/** 字段可见性条件：当 snapshot 中 path 的值满足 eq/ne 时显示。 */
export interface PluginVisibleWhen { path: string; eq?: unknown; ne?: unknown }

/** 一个 mutation 或按声明优先级排列的候选 mutation。 */
export type PluginMutation = string | string[];

/** 开关切换声明：从 source 读取开关状态，关闭时写入 offValue，恢复时读取 restoreField。 */
export interface PluginSwitch { source: string; offValue: unknown; restoreField?: string }

/** 选项条目（用于 select/segmented 等编辑器）。 */
export interface PluginFieldOption { value: string | number | boolean; labelKey: string }

/** 声明式字段定义，描述一个可编辑的设备状态项。 */
export interface PluginField {
  id: string;
  source: string;
  mutation?: PluginMutation;
  param?: string;
  params?: Record<string, unknown>;
  /** 写入组合参数时，从当前设备快照读取其余参数。当前字段值最后覆盖同名参数。 */
  paramSources?: Record<string, string>;
  editor: PluginEditor;
  labelKey?: string;
  labelSource?: string;
  editTitleKey?: string;
  editLabelKey?: string;
  options?: PluginFieldOption[];
  optionSource?: string;
  range?: RangeSpec;
  /** 动态 range 来源：指向 snapshot 中的数字路径，运行时覆盖 range.max。 */
  rangeSource?: string;
  /** rangeSource 解析后的偏移量（默认 0），用于 count → index 转换（如 -1）。 */
  rangeMaxOffset?: number;
  format?: PluginFieldFormat;
  visibleWhen?: PluginVisibleWhen;
  switch?: PluginSwitch;
}

/** 灯光区域声明：一组相关字段的集合。 */
export interface PluginZone { id: string; labelKey: string; fields: PluginField[]; visibleWhen?: PluginVisibleWhen }

/** DPI 分档布局声明。 */
export interface PluginStageLayout {
  dotsSource: string;
  selectMutation: PluginMutation;
  setMutation: PluginMutation;
  valueSource: string;
  colorSource?: string;
  range: RangeSpec;
  /** 动态 range 来源：指向 snapshot 中的数字路径，运行时覆盖 range.max。 */
  rangeSource?: string;
  /** rangeSource 解析后的偏移量（默认 0），用于 count → index 转换（如 -1）。 */
  rangeMaxOffset?: number;
  /** 切换分档时的 mutation 参数名，默认 value。 */
  selectParam?: string;
  /** 修改分档时的档位参数名，默认 stage。 */
  stageParam?: string;
  /** 修改分档时的数值参数名，默认 value。 */
  valueParam?: string;
}

/** 状态栏显示声明。 */
export interface PluginStatusDisplay {
  labelKey?: string;
  valueSource: string;
  valueFormat?: PluginFieldFormat;
  valueOptions?: PluginFieldOption[];
  onClickField?: string;
}

/** 控件下方的只读摘要项；内容与路径均由插件声明。 */
export interface PluginSummaryItem {
  labelKey?: string;
  /** 兼容旧插件的直接标签；新插件应优先使用 labelKey。 */
  label?: string;
  source: string;
  unit?: string;
  format?: PluginFieldFormat;
  options?: PluginFieldOption[];
}

/** 字段名 → snapshot source 路径映射。 */
export interface PluginStateMapping { [field: string]: string }

export type DeviceCapabilities = Record<string, Record<string, unknown>>;
export type PluginControl = 'Toggle' | 'Segmented' | 'Select' | 'Slider' | 'Number' | 'Color' | 'GradientStops' | 'DpiStages' | 'LightingZone' | 'ReadOnlyValue' | 'Action';
export interface PluginCapability {
  id: string;
  control: PluginControl;
  labelKey: string;
  readOnly: boolean;
  placements?: PluginCapabilityPlacement[];
  metadata: PluginCapabilityMetadata;
  /** 设备实际是否支持该能力（运行时探测结果）。默认 true（向后兼容）。 */
  available?: boolean;
  /** 连接类型能力分支（#3）：声明该能力仅在指定连接类型下可见。 */
  connections?: string[];
  /** 固件版本门槛（#4）：声明该能力所需的最低固件版本。 */
  minFirmware?: string;
}
export interface PluginCapabilityMetadata {
  /** 宿主主题色的设备状态来源。插件应指向鼠标灯光颜色，而非附属接收器颜色。 */
  accentSource?: string;
  fields?: PluginField[];
  zones?: PluginZone[];
  stageLayout?: PluginStageLayout;
  statusDisplay?: PluginStatusDisplay;
  summary?: PluginSummaryItem[];
  stateMapping?: PluginStateMapping;
  visibleWhen?: PluginVisibleWhen;
  [key: string]: unknown;
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
  state: Record<string, unknown>;
  capabilities: DeviceCapabilities;
  pluginCapabilities: PluginCapability[];
  writableMutations: string[];
  evidence: Evidence;
  /** 插件未签名/未启用写入时为 true，UI 显示只读模式标记。 */
  readonly: boolean;
  /** 匹配该设备的插件 ID，用于 i18n namespace 解析。 */
  pluginId?: string;
  updatedAt: string;
  /** Per-output read statuses from the workflow engine. */
  readStatuses?: Record<string, ReadStatus>;
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
  /** 托盘渲染模式：auto | native-macos | dynamic-image | static */
  trayRenderMode: 'auto' | 'native-macos' | 'dynamic-image' | 'static';
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
  telemetryDisabled: boolean;
  automaticUpdateChecks: boolean;
  automaticUpdateInstall: boolean;
  automaticPluginUpdateChecks: boolean;
  automaticLocalAiUpdateChecks: boolean;
  localAiAnalysisEnabled: boolean;
  localAiFeatures: Record<string, boolean>;
  batteryHistoryEnabled: boolean;
  batteryHistoryRetentionDays: number;
  unusualDrainAlerts: boolean;
  /** 屏幕解锁时主动唤醒鼠标：开启后由解锁事件接管主动读取 */
  wakeOnUnlock: boolean;
}

export interface LocalAiStatus {
  ready: boolean;
  bundleVersion?: string;
  runtimeVersion?: string;
  modelPackId?: string;
  modelPackVersion?: string;
  handlerId?: string;
  handlerVersion?: string;
  handlerApiVersion?: number;
  rollbackAvailable: boolean;
  error?: string;
}

export interface LocalAiUpdateInfo {
  component: 'bundle';
  currentVersion?: string;
  availableVersion: string;
  updateAvailable: boolean;
}

export interface LocalAiInstallResult {
  component: 'bundle';
  version: string;
  previousVersion?: string;
  ready: boolean;
}

export interface DeviceSnapshot {
  displayName: string;
  connection: 'usb' | 'wireless' | 'bluetooth' | 'virtual';
  /** 插件根据设备描述或实际连接类型声明的默认选择优先级。 */
  selectionPriority?: number;
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
  /** 匹配该设备的插件 ID，用于 i18n namespace 解析。 */
  pluginId?: string;
  /** 插件声明的跨连接/跨接口身份，用于历史统计等宿主通用功能做合并。 */
  historyIdentity?: DeviceIdentity;
  /** Per-output read statuses from the workflow engine. */
  readStatuses?: Record<string, ReadStatus>;
}

export interface DeviceSnapshotEntry {
  deviceKey: string;
  snapshot: DeviceSnapshot;
  selected: boolean;
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
  lastErrorKind?: string;
  lastError?: string;
}

// ─── 电量使用情况类型 ───────────────────────────────────────────────────────

export type BatteryHistoryRange = '24h' | '10d';

export type BatteryInsightType =
  | 'estimatedRemaining'
  | 'estimatedActiveRemaining'
  | 'estimatedRunout'
  | 'chargingHabit'
  | 'abnormalDrain'
  | 'powerSavingTip'
  | 'batteryConsistency'
  | 'deviceComparison'
  | 'averageDailyDrain'
  | 'chargingCount'
  | 'lowestLevel';

export type BatteryInsightSeverity = 'info' | 'warning' | 'critical';

export interface BatteryHistoryResponse {
  range: BatteryHistoryRange;
  devices: BatteryHistoryDevice[];
  series: BatteryHistorySeries[];
  insights: BatteryInsight[];
  generatedAt: string;
}

export interface BatteryHistoryDevice {
  key: string;
  deviceId: string;
  deviceName: string;
  connection: string;
  componentId: string;
  componentLabel: string;
  latestPercentage?: number;
  latestCharging?: boolean;
  latestAt?: string;
  lowBattery?: boolean;
}

export interface BatteryHistorySeries {
  key: string;
  points: BatteryHistoryPoint[];
}

export interface BatteryHistoryPoint {
  bucketStart: string;
  bucketLabel: string;
  usageElapsedMinutes?: number;
  percentage?: number | null;
  minPercentage?: number | null;
  maxPercentage?: number | null;
  charging?: boolean | null;
  lowBattery?: boolean | null;
  sampleCount: number;
}

export interface BatteryInsight {
  type: BatteryInsightType;
  severity: BatteryInsightSeverity;
  title: string;
  message: string;
  /** 关联设备 key（{deviceId}:{componentId}）。undefined 表示跨设备洞察，前端应始终展示。 */
  deviceKey?: string;
}
