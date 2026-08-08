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
export type PluginEditor = 'inline-toggle' | 'inline-segmented' | 'inline-range' | 'inline-value' | 'inline-action' | 'modal-select' | 'modal-color' | 'modal-range' | 'modal-number' | 'modal-dpi-stage' | 'modal-gradient' | 'static-readonly';

/** 字段值格式化方式。 */
export type PluginFieldFormat = 'sleep' | 'percent' | 'hertz' | 'connection' | 'color' | 'default';

/** 字段可见性条件：当 snapshot 中 path 的值满足 eq/ne/in 时显示。
 *  `in` 用于多值匹配，典型场景是 `path: "family"` 区分 Protocol A 与 AM35，
 *  避免在宿主代码中按 pluginId/vendorId 硬编码协议分支。 */
export interface PluginVisibleWhen {
  path: string;
  eq?: unknown;
  ne?: unknown;
  /** 当 path 的值存在于该数组时显示。与 eq 互斥（同时声明时 eq 优先）。 */
  in?: unknown[];
}

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
  /** P1-A：字段展示层级。`primary` 在 Dashboard 首页渲染；`details` 进入 Advanced Settings。
   *  未声明时默认为 `primary`（向后兼容）。用于 Receiver Lighting 十字段分层。 */
  presentation?: 'primary' | 'details';
  /** P0-C：Advanced Settings 分组。未进入首页的可写字段按此分组展示。 */
  advancedSection?: 'performance' | 'lighting-details' | 'profiles' | 'buttons' | 'power' | 'sensor' | 'device';
  /** P0-C：同一 advancedSection 内的排序权重（越小越靠前）。 */
  advancedOrder?: number;
  /** P0-F/P0-G：子块优先级（0..100，越高越优先）。用于回报率/灯光子块选择。
   *  未声明时按声明顺序处理。 */
  priority?: number;
  /** P0-G：灯光子块角色。
   *  - `effect`：固定最左（灯效/模式）；
   *  - `primary-color`：固定最右（主颜色）；
   *  - `candidate`：参与中间最多 4 个位置竞争。
   *  未声明时视为 `candidate`（向后兼容）。 */
  lightingRole?: 'effect' | 'primary-color' | 'candidate';
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
  /** 当 variants 存在时可省略：由匹配的 variant 提供 valueSource。 */
  valueSource?: string;
  valueFormat?: PluginFieldFormat;
  valueOptions?: PluginFieldOption[];
  onClickField?: string;
  /** P0-E：按设备状态选择不同的显示来源（如 AM35 sleep 按 family/connection 分支）。 */
  variants?: PluginStatusDisplayVariant[];
}

/** P0-E：状态栏显示变体。第一个 visibleWhen 匹配的 variant 生效。 */
export interface PluginStatusDisplayVariant {
  visibleWhen: PluginVisibleWhen;
  valueSource: string;
  valueFormat?: PluginFieldFormat;
  valueOptions?: PluginFieldOption[];
  onClickField?: string;
  labelKey?: string;
}

/** 控件下方的只读摘要项；内容与路径均由插件声明。 */
export interface PluginSummaryItem {
  labelKey?: string;
  /** 兼容旧插件的直接标签；新插件应优先使用 labelKey。 */
  label?: string;
  source: string;
  /** 同一语义在不同协议族使用不同 snapshot 路径时，按顺序尝试的备用来源。 */
  sourceFallbacks?: string[];
  unit?: string;
  format?: PluginFieldFormat;
  options?: PluginFieldOption[];
  /** P0-F：子块优先级（0..100，越高越优先）。用于回报率页面子块选择。
   *  未声明时按声明顺序处理。超过 maxSubBlocks 的低优先级项进入 Advanced Settings。 */
  priority?: number;
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
  /** ITERATION-004 §2.1：Dashboard priority 0..100（越高越优先）。
   *  P1-B：类型层面必填；运行时仍兼容旧数据（undefined 按 0 处理）。 */
  priority: number;
  /** Dashboard 角色：fixed-core（DPI/回报率/灯光）| candidate（竞争槽位）| system（系统入口如全部读数）。
   *  P1-B：类型层面必填；运行时仍兼容旧数据（undefined 按 'candidate' 处理）。 */
  dashboardRole: 'fixed-core' | 'candidate' | 'system';
  /** 固定槽位 1..3（仅 DPI=1、回报率=2、灯光=3）。插件不得声明 fixedSlot=4。 */
  fixedSlot?: 1 | 2 | 3;
  /** 是否有资格竞争第 4 槽位（需同时 priority >= 90）。 */
  fourthSlotEligible?: boolean;
  /** 全局去重键（如 "dashboard.dpi"、"system.all-readings"）。相同 dedupeKey 只保留一个。 */
  dedupeKey?: string;
  /** 未进入首页时的回退区域。
   *  P1-B：类型层面必填；运行时仍兼容旧数据（undefined 按 'advanced' 处理）。
   *  ITERATION-006 §P0-A：跨仓统一为 advanced | inventory | hidden（不再接受 details）。 */
  fallbackRegion: 'advanced' | 'inventory' | 'hidden';
  /** P0-E：唯一候选槽位位置。仅对 dashboardRole='candidate' 有效。
   *  - leading：放在核心序列（DPI→回报率→灯光）之前；
   *  - trailing：放在核心序列之后；
   *  未声明时默认 trailing。
   *  不得用 fixedSlot=4 表达候选；核心三项相对顺序不可被打断。 */
  optionalPosition?: 'leading' | 'trailing';
  /** 首页切换块的短标签；完整 capability label 仍用于页面内容与无障碍名称。 */
  compactLabelKey?: string;
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
  /** 接收器场景下鼠标是否就位，详见 DeviceSnapshot.mouseReady。 */
  mouseReady?: boolean;
  /** 设备所属的协议 family，详见 DeviceSnapshot.family。
   *  `resolveVisibleWhen` 通过 `path: "family"` 读取此字段。 */
  family?: string;
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
  /** Windows 独立数字电量图标：开启后在鼠标状态图标外创建独立的数字电量托盘图标。 */
  trayShowBatteryIcon: boolean;
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
  previousVersion?: string;
  error?: string;
}

export type LocalAiComponent = 'runtime' | 'model' | 'handler';

export interface LocalAiUpdateInfo {
  component: LocalAiComponent;
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
  /**
   * 接收器场景下鼠标是否就位（基于 receiverIdle.mouseOnline）。
   * - `undefined`：非接收器连接（USB 直连 / 蓝牙），鼠标总是视为就位。
   * - `true`：接收器已插入且鼠标在线。
   * - `false`：接收器已插入但鼠标未就位，UI 显示等待提示而非残缺 Dashboard。
   * 由后端协议层填充，前端只读。
   */
  mouseReady?: boolean;
  /** 设备所属的协议 family（如 "protocol-a-direct"、"am35-direct"）。
   *  用于 `visibleWhen.path = "family"` 实现协议感知的能力可见性，
   *  避免在宿主代码中按 pluginId/vendorId 硬编码协议分支。
   *  由后端从 `MatchedDevice.family` 填充。 */
  family?: string;
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
