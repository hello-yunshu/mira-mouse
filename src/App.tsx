// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  BatteryHigh,
  Gauge,
  Gear,
  Info,
  Lightbulb,
  Minus,
  ReadCvLogo,
  SignOut,
  Square,
  Timer,
  UserCircle,
  WaveSine,
  X,
} from '@phosphor-icons/react';
import { MOCK_DEVICE } from './mock';
import { applyTheme } from './theme';
import { SettingsPage } from './Settings';
import { AboutPage } from './About';
import type { AboutInfo, AppSettings, DeviceBattery, DeviceCapabilities, DeviceSnapshot, DeviceState, PluginCapability, PluginUpdateInfo, ThemeMode } from './types';
import './styles.css';
import { notifyError, notifyInfo, onAppNotification, type AppNotification } from './notify';
import { startAutomaticAppUpdateCheck } from './updater';

type View = 'dashboard' | 'settings' | 'about';
type ControlMode = string;
let automaticPluginCheckStarted = false;

function isWindowsPlatform(): boolean {
  const previewPlatform = new URLSearchParams(window.location.search).get('platform');
  return previewPlatform === 'windows' || navigator.userAgent.includes('Windows');
}

function isMacPlatform(): boolean {
  const previewPlatform = new URLSearchParams(window.location.search).get('platform');
  return previewPlatform === 'macos'
    || (previewPlatform === null && /Macintosh|Mac OS X/.test(navigator.userAgent));
}

function isWindowsWebPreview(): boolean {
  return new URLSearchParams(window.location.search).get('platform') === 'windows'
    && !navigator.userAgent.includes('Windows');
}

function isPureWebPreview(): boolean {
  // 纯浏览器环境（非 Tauri 运行时），用于网页预览
  return !(typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window);
}

function WindowsPreviewControls() {
  return (
    <div className="windows-preview-controls" aria-label="Windows 窗口控件">
      <button type="button" aria-label="最小化窗口"><Minus weight="regular" /></button>
      <button type="button" aria-label="最大化窗口"><Square weight="regular" /></button>
      <button type="button" className="windows-close" aria-label="关闭窗口"><X weight="regular" /></button>
    </div>
  );
}

const CONNECTION_LABEL: Record<DeviceSnapshot['connection'], DeviceState['connection']> = {
  usb: 'USB',
  wireless: '无线',
  bluetooth: '蓝牙',
  virtual: '虚拟',
};

function connectionLabel(connection: string | undefined): DeviceState['connection'] {
  return CONNECTION_LABEL[connection as DeviceSnapshot['connection']] ?? '未知连接';
}

// 界面不硬编码品牌灯效名称。灯效名称由插件 parsers.json 的 derived.lookup 提供（effectName/optionName）。
// 当插件未提供名称时，界面只显示通用占位符，避免将品牌数据耦合到 UI 层。
function lightingCapability(capabilities: DeviceCapabilities | undefined, group: 'mouseEffect' | 'receiverLighting'): Record<string, unknown> | undefined {
  return capabilities?.[group] ?? (group === 'mouseEffect' ? capabilities?.lighting : undefined);
}

function getLightingEffectName(capabilities?: DeviceCapabilities, group: 'mouseEffect' | 'receiverLighting' = 'mouseEffect'): string {
  const lighting = lightingCapability(capabilities, group);
  if (!lighting) return '硬件同步';
  // 仅使用插件提供的 effectName（来自 parsers.json derived.lookup）
  if (typeof lighting.effectName === 'string' && lighting.effectName) return lighting.effectName;
  const effect = lighting.effect;
  if (typeof effect !== 'number') return '硬件同步';
  if (effect === 0) return '已关闭';
  return `灯效 ${effect}`;
}

function getLightingColorMode(capabilities?: DeviceCapabilities, group: 'mouseEffect' | 'receiverLighting' = 'mouseEffect'): string {
  const lighting = lightingCapability(capabilities, group);
  if (!lighting) return '未报告';
  // 仅使用插件提供的 optionName（来自 parsers.json derived.lookup）
  if (typeof lighting.optionName === 'string' && lighting.optionName) return lighting.optionName;
  const option = lighting.option;
  if (typeof option !== 'number') return '未报告';
  return `模式 ${option}`;
}

function rgbToHex(rgb: unknown): string | undefined {
  if (typeof rgb === 'string' && /^#[0-9a-f]{6}$/i.test(rgb)) return rgb;
  if (!Array.isArray(rgb) || rgb.length < 3) return undefined;
  const [r, g, b] = rgb.map((v) => Number(v));
  if ([r, g, b].some((v) => Number.isNaN(v))) return undefined;
  // 钳制到 [0, 255] 范围，避免负数或 >255 的值产生无效的 hex 字符串
  return `#${[r, g, b].map((v) => Math.max(0, Math.min(255, Math.trunc(v))).toString(16).padStart(2, '0')).join('')}`;
}

function snapshotToState(snapshot: DeviceSnapshot): DeviceState {
  const now = new Date().toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' });
  // Only synthesize stages when the device actually reports DPI data.
  // A missing value means the capability is absent, not that DPI is 800.
  const stages = snapshot.dpiStages?.length
    ? snapshot.dpiStages
    : snapshot.dpi !== undefined && snapshot.dpi !== null
      ? [{ value: snapshot.dpi, color: '#9a8bd0', enabled: true, active: true }]
      : [];
  const caps = snapshot.capabilities ?? {};
  const mouseEffect = lightingCapability(caps, 'mouseEffect');
  const receiverLighting = lightingCapability(caps, 'receiverLighting');
  const settings = caps.settings;
  const mouseLightEnabled = typeof settings?.mouseLightEnabled === 'boolean'
    ? settings.mouseLightEnabled
    : typeof mouseEffect?.enabled === 'boolean' ? mouseEffect.enabled : undefined;
  const mouseLightColor = rgbToHex(settings?.mouseLightStartColor)
    ?? (typeof mouseEffect?.color === 'string' ? mouseEffect.color : snapshot.confirmedLightColor);
  const mouseLightEndColor = rgbToHex(settings?.mouseLightEndColor);
  const fallbackBatteries: DeviceBattery[] = snapshot.batteryPercent === undefined ? [] : [{
    id: 'mouse', label: '鼠标', percentage: snapshot.batteryPercent, charging: snapshot.charging,
  }];
  // Build lighting state only when at least one lighting field is reported.
  // Avoid defaulting to 'enabled' when the device never reported lighting.
  const hasLightingData = mouseLightColor !== undefined
    || mouseLightEndColor !== undefined
    || mouseEffect !== undefined
    || receiverLighting !== undefined;
  return {
    name: snapshot.displayName ?? '未知设备',
    connection: connectionLabel(snapshot.connection),
    battery: snapshot.batteryPercent,
    charging: snapshot.charging,
    batteries: snapshot.batteries?.length ? snapshot.batteries : fallbackBatteries,
    pollingRate: snapshot.pollingRateHz,
    supportedPollingRates: snapshot.supportedPollingRatesHz,
    profile: snapshot.profile ? `配置 ${snapshot.profile}` : undefined,
    evidence: snapshot.evidence,
    updatedAt: now,
    dpiStages: stages,
    lighting: hasLightingData
      ? {
          enabled: mouseLightEnabled !== false,
          mode: mouseEffect ? getLightingEffectName(caps, 'mouseEffect') : mouseLightEnabled === false ? '已关闭' : '已开启',
          color: mouseLightColor,
          supportsSpeed: typeof mouseEffect?.speed === 'number',
          supportsBrightness: typeof mouseEffect?.brightness === 'number',
          receiverLinked: snapshot.connection === 'wireless',
          mouseLightEnabled,
          mouseLightColor,
          mouseLightEndColor,
          receiverLightEnabled: typeof receiverLighting?.enabled === 'boolean' ? receiverLighting.enabled : undefined,
          receiverLightMode: receiverLighting ? getLightingEffectName(caps, 'receiverLighting') : undefined,
          receiverLightColor: typeof receiverLighting?.color === 'string' ? receiverLighting.color : undefined,
        }
      : undefined,
    capabilities: caps,
    pluginCapabilities: snapshot.pluginCapabilities ?? [],
    writableMutations: snapshot.writableMutations ?? [],
    readonly: snapshot.readonly === true,
  };
}

function DeviceAura({ color }: { color?: string }) {
  return (
    <div className="device-aura" data-animation="realtime-deformation" style={{ '--device-color': color ?? '#b87ab0' } as React.CSSProperties} aria-hidden="true">
      <div className="aura-cloud aura-cloud-1" />
      <div className="aura-cloud aura-cloud-2" />
      <div className="aura-cloud aura-cloud-3" />
      <div className="aura-cloud aura-cloud-4" />
      <div className="aura-cloud aura-cloud-5" />
      <div className="aura-star aura-star-1" />
      <div className="aura-star aura-star-2" />
      <div className="aura-star aura-star-3" />
      <div className="aura-star aura-star-4" />
      <div className="aura-star aura-star-5" />
      <div className="aura-star aura-star-6" />
    </div>
  );
}

function EmptyState({ onRefresh, onDemo, onOpenSettings }: { onRefresh: () => void; onDemo: () => void; onOpenSettings: () => void }) {
  return (
    <main className="empty">
      <DeviceAura />
      <p className="eyebrow">Mira Mouse</p>
      <h1>没有找到支持的鼠标</h1>
      <p>插上鼠标后，可用的设置会出现在这里。</p>
      <div className="actions">
        <button onClick={onRefresh}>刷新</button>
        <button className="secondary" onClick={onOpenSettings}>设备与诊断</button>
      </div>
      <button className="demo" onClick={onDemo}>打开 Fixture 演示</button>
    </main>
  );
}

const CAPABILITY_GROUP_LABELS: Record<string, string> = {
  device: '设备连接',
  deviceInfo: '设备标识',
  deviceName: '设备名称',
  deviceNameLength: '名称信息',
  battery: '电池',
  batteryCapability: '电池能力',
  unifiedBatteryCapability: '统一电池能力',
  dpi: 'DPI 档位',
  dpiExtended: '扩展 DPI 档位',
  featureSet: 'HID++ 功能集',
  featureIndexBattery: '电池功能索引',
  featureIndexDeviceName: '名称功能索引',
  featureIndexDpi: 'DPI 功能索引',
  featureIndexExtendedDpi: '扩展 DPI 功能索引',
  featureIndexFeatureSet: '功能集索引',
  featureIndexMousePointer: '指针功能索引',
  featureIndexPointerSpeed: '指针速度功能索引',
  featureIndexSurfaceTuning: '表面调校功能索引',
  featureIndexXyStats: 'XY 统计功能索引',
  featureIndexWheelStats: '滚轮统计功能索引',
  featureIndexReportRate: '回报率功能索引',
  featureIndexExtendedReportRate: '扩展回报率功能索引',
  featureIndexColorLed: 'Color LED 功能索引',
  featureIndexRgbEffects: 'RGB 功能索引',
  featureIndexOnboardProfiles: '板载配置功能索引',
  featureIndexProfileManagement: '配置管理功能索引',
  featureIndexUnifiedBattery: '统一电池功能索引',
  mousePointer: '鼠标指针',
  pointerSpeed: '指针速度',
  settings: '传感器与连接',
  settingsExtended: '扩展传感器与连接',
  reportRateList: '可用回报率',
  reportRateListExtended: '扩展可用回报率',
  colorLedInfo: 'Color LED 信息',
  rgbEffectsInfo: 'RGB_EFFECTS 信息',
  rgbControl: 'RGB 接管状态',
  controlMode: '板载/软件控制',
  onboardDescription: '板载配置描述',
  onboardMode: '板载控制模式',
  onboardCurrentProfile: '当前板载配置',
  onboardCurrentDpiIndex: '当前 DPI 索引',
  profileMgmtInfo: '配置管理信息',
  profileMgmtCount: '配置数量',
  profileMgmtCurrent: '当前配置文件',
  lighting: '主灯光（旧插件）',
  mouseEffect: '鼠标灯效',
  receiverLighting: '接收器灯光',
  fps: '传感器帧率',
  dpiButton: 'DPI 快切',
  firmwareUsb: '鼠标 USB 固件',
  firmwareSoc: '鼠标主控固件',
  receiverFirmwareUsb: '接收器 USB 固件',
  receiverFirmwareSoc: '接收器主控固件',
  receiverFirmwareLed: '接收器灯光固件',
  buttonMappings: '按键映射',
};

const CAPABILITY_FIELD_LABELS: Record<string, string> = {
  deviceIndex: '设备索引', featureIndex: '功能索引', featureVersion: '功能版本', connection: '连接方式', name: '设备名称', length: '名称长度',
  entityCount: '固件实体数量', unitId: '设备单元 ID', transport: '传输类型', modelId: '型号 ID',
  percentage: '电量', charging: '充电中', valid: '数据有效', profile: '配置编号', currentStage: '当前档位', stageCount: '档位数量',
  nextPercentage: '下一电量阈值', statusRaw: '充电状态原始值', statusName: '充电状态', chargingStatus: '充电状态值', externalPowerStatus: '外部供电状态', levelFlags: '电量等级',
  supportedLevels: '支持的电量等级', capabilityFlags: '能力标志', sensorIndex: '传感器索引', dpiValue: '当前 DPI', defaultDpi: '默认 DPI',
  count: '数量', dpi: 'DPI', flags: '标志', accelerationRaw: '加速度原始值', acceleration: '加速度', suggestOsBallistics: '建议系统弹道', suggestVerticalOrientation: '建议垂直方向',
  speedRaw: '速度原始值', target: '目标', mode: '模式', modeName: '模式名称', zoneCount: '区域数量', response0: '响应字节 0', response1: '响应字节 1', responseFlags: '响应标志', capabilities: '能力标志',
  maxProfileCount: '最大配置数', profileNameLength: '配置名长度', profileCount: '配置数量', profileIndex: '配置索引', memoryModelId: '内存模型 ID', profileFormatId: '配置格式 ID',
  macroFormatId: '宏格式 ID', readOnlyProfileCount: '只读配置数', buttonCount: '按键数量', sectorCount: '扇区数量', sectorSize: '扇区大小', mechanicalLayout: '机械布局', variousInfo: '附加信息',
  dpiX: 'X 轴 DPI', dpiY: 'Y 轴 DPI', stageColors: '档位颜色', pollingRaw: '回报率原始值', pollingRate: '回报率',
  usbDebounce: 'USB 防抖', wirelessDebounce: '2.4G 防抖', bluetoothDebounce: '蓝牙防抖', rippleCorrection: '波纹修正',
  buttonChangeTime: '按键切换时间', wheelToButton: '滚轮转按键', buttonToWheel: '按键转滚轮', bluetoothSleepValue: '蓝牙休眠值',
  wirelessSleepValue: '2.4G 休眠值', liftCutOff: '抬升高度', angleSnap: '角度吸附', motionSync: '运动同步',
  mouseLightStartColor: '鼠标灯光颜色', mouseLightEndColor: '鼠标灯光结束色', mouseLightEnabled: '鼠标灯光启用', effect: '灯效', speed: '速度',
  brightness: '亮度', option: '颜色模式', color: '灯光颜色', enabled: '启用', versionRaw: '固件原始版本值',
  effectName: '灯效名称', optionName: '颜色模式名称', speedLabel: '速度等级', brightnessLabel: '亮度等级',
};

function capabilityValue(value: unknown, key: string): string {
  if (typeof value === 'boolean') return value ? '开启' : '关闭';
  if (typeof value === 'number') {
    // 灯效/颜色模式的友好名称由插件 derived.lookup 提供（effectName/optionName 字段），
    // 此处仅显示原始数值，避免在界面硬编码品牌映射表。
    if (key === 'percentage' || key === 'brightness') return `${value}%`;
    if (key === 'pollingRate') return `${value} Hz`;
    return String(value);
  }
  if (Array.isArray(value)) {
    if (key.startsWith('0x') && value.every((item) => typeof item === 'number')) {
      return value.map((item) => Number(item).toString(16).toUpperCase().padStart(2, '0')).join(' ');
    }
    return value.join(' · ');
  }
  if (value === null || value === undefined || value === '') return '未报告';
  if (typeof value === 'object') return JSON.stringify(value);
  return String(value);
}

function readCapability(capabilities: DeviceCapabilities, group: string, field: string): unknown {
  return capabilities[group]?.[field];
}

function preferredCapability(capabilities: DeviceCapabilities, group: string, preferred: string, fallback: string): string {
  const preferredValue = readCapability(capabilities, group, preferred);
  return preferredValue === undefined
    ? capabilityValue(readCapability(capabilities, group, fallback), fallback)
    : capabilityValue(preferredValue, preferred);
}

function formatSleepTime(value: unknown): string {
  const seconds = typeof value === 'number' ? value : Number(value);
  if (!Number.isFinite(seconds) || seconds <= 0) return '未报告';
  if (seconds % 60 === 0) return `${seconds / 60} 分钟`;
  return `${seconds} 秒`;
}

const PLUGIN_LABELS: Record<string, string> = {
  'capability.battery': '电量',
  'capability.dpi': 'DPI',
  'capability.polling-rate': '回报率',
  'capability.pointer-speed': '指针速度',
  'capability.sleep-time': '休眠时间',
  'capability.profile': '配置文件',
  'capability.profile-mgmt-current': '当前配置文件',
  'capability.firmware': '固件',
  'capability.lighting': '灯光',
  'capability.mouse-lighting': '鼠标灯光',
  'capability.rgb-control': 'RGB 接管',
  'capability.control-mode': '配置控制',
  'capability.onboard-profile': '板载配置',
};

function pluginLabel(capability: PluginCapability): string {
  return typeof capability.metadata.label === 'string'
    ? capability.metadata.label
    : PLUGIN_LABELS[capability.labelKey] ?? capability.labelKey ?? capability.id;
}

function readPath(device: DeviceState, path: unknown): unknown {
  if (typeof path !== 'string' || !path) return undefined;
  let value: unknown = device;
  for (const key of path.split('.')) {
    if (!value || typeof value !== 'object') return undefined;
    value = (value as Record<string, unknown>)[key];
  }
  return value;
}

interface PluginOption { value: string | number | boolean; label: string }

const MAX_CONTROL_GROUPS = 6;
const MAX_STATUS_ITEMS = 6;
const MAX_CONTROL_OPTIONS = 8;
const MAX_SUMMARY_ITEMS = 4;

interface PluginSummaryItem {
  label: string;
  source: string;
  unit?: string;
  format?: string;
  options: PluginOption[];
}

function parsePluginOptions(value: unknown): PluginOption[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((option) => {
    if (!option || typeof option !== 'object') return [];
    const optionValue = (option as Record<string, unknown>).value;
    const label = (option as Record<string, unknown>).label;
    return (typeof optionValue === 'string' || typeof optionValue === 'number' || typeof optionValue === 'boolean') && typeof label === 'string'
      ? [{ value: optionValue, label }]
      : [];
  });
}

function pluginOptions(capability: PluginCapability, device?: DeviceState): PluginOption[] {
  if (capability.labelKey === 'capability.polling-rate' && device?.supportedPollingRates?.length) {
    return device.supportedPollingRates
      .slice(0, MAX_CONTROL_OPTIONS)
      .map((value) => ({ value, label: `${value} Hz` }));
  }
  return parsePluginOptions(capability.metadata.options).slice(0, MAX_CONTROL_OPTIONS);
}

function pluginSummaryItems(capability: PluginCapability): PluginSummaryItem[] {
  if (!Array.isArray(capability.metadata.summary)) return [];
  return capability.metadata.summary.slice(0, MAX_SUMMARY_ITEMS).flatMap((item) => {
    if (!item || typeof item !== 'object') return [];
    const record = item as Record<string, unknown>;
    if (typeof record.label !== 'string' || typeof record.source !== 'string') return [];
    return [{
      label: record.label,
      source: record.source,
      unit: typeof record.unit === 'string' ? record.unit : undefined,
      format: typeof record.format === 'string' ? record.format : undefined,
      options: parsePluginOptions(record.options),
    }];
  });
}

function pluginSummaryValue(item: PluginSummaryItem, device: DeviceState): string {
  const value = readPath(device, item.source);
  const option = item.options.find((candidate) => candidate.value === value);
  if (option) return option.label;
  if (item.format === 'sleep') return formatSleepTime(value);
  if (item.unit && typeof value === 'number') return `${value} ${item.unit}`;
  return capabilityValue(value, item.source.split('.').at(-1) ?? 'value');
}

function PluginSummary({ capability, device }: { capability: PluginCapability; device: DeviceState }) {
  const items = pluginSummaryItems(capability);
  if (items.length === 0) return null;
  return (
    <div
      className="capability-summary"
      aria-label="设备摘要"
      style={{ gridTemplateColumns: `repeat(${items.length}, minmax(0, 1fr))` }}
    >
      {items.map((item) => (
        <span key={`${item.label}:${item.source}`}>{item.label}<strong>{pluginSummaryValue(item, device)}</strong></span>
      ))}
    </div>
  );
}

function pluginValueLabel(capability: PluginCapability, value: unknown): string {
  const option = pluginOptions(capability).find((candidate) => candidate.value === value);
  if (option) return option.label;
  if (capability.metadata.format === 'sleep') return formatSleepTime(value);
  if (capability.metadata.unit === 'Hz' && typeof value === 'number') return `${value} Hz`;
  return capabilityValue(value, capability.id);
}

type PluginRegion = 'hero' | 'control' | 'status' | 'details';
type PluginIcon = typeof Gauge;

const PLUGIN_ICON_REGISTRY: Record<string, PluginIcon> = {
  battery: BatteryHigh,
  gauge: Gauge,
  info: Info,
  lightbulb: Lightbulb,
  profile: UserCircle,
  settings: Gear,
  timer: Timer,
  wave: WaveSine,
};

function pluginIcon(name: string | undefined): PluginIcon {
  return (name && PLUGIN_ICON_REGISTRY[name]) || Info;
}

function legacyPlacements(capability: PluginCapability): NonNullable<PluginCapability['placements']> {
  const section = capability.metadata.section;
  const placements: NonNullable<PluginCapability['placements']> = [];
  if (section === 'hero' || section === 'control' || section === 'status' || section === 'details') {
    placements.push({ region: section, group: capability.id, order: 0, span: 1 });
  } else if (capability.control === 'DpiStages' || capability.control === 'LightingZone') {
    placements.push({ region: 'control', group: capability.id, order: 0, span: 1 });
  }
  if (capability.metadata.status === true && !placements.some((placement) => placement.region === 'status')) {
    placements.push({ region: 'status', order: 0, span: 1 });
  }
  return placements;
}

function placementsFor(capability: PluginCapability, region: PluginRegion) {
  const declared = capability.placements?.length ? capability.placements : legacyPlacements(capability);
  return declared.filter((placement) => placement.region === region);
}

function metadataMutation(value: unknown, device?: DeviceState): string | undefined {
  if (typeof value === 'string') return value;
  if (!Array.isArray(value)) return undefined;
  const candidates = value.filter((candidate): candidate is string => typeof candidate === 'string');
  return candidates.find((candidate) => device?.writableMutations.includes(candidate)) ?? candidates[0];
}

function pluginMutations(capability: PluginCapability, device?: DeviceState): Record<string, string> {
  const mutations: Record<string, string> = {};
  const defaultMutation = metadataMutation(capability.metadata.mutation, device);
  if (defaultMutation) mutations.default = defaultMutation;
  if (capability.metadata.mutations && typeof capability.metadata.mutations === 'object') {
    for (const [key, value] of Object.entries(capability.metadata.mutations as Record<string, unknown>)) {
      const mutation = metadataMutation(value, device);
      if (mutation) mutations[key] = mutation;
    }
  }
  if (Object.keys(mutations).length === 0) {
    if (capability.control === 'DpiStages') {
      mutations.select = 'set-dpi-stage';
      mutations.value = 'set-dpi-value';
    } else if (capability.control === 'LightingZone') {
      mutations.mouse = 'set-mouse-lighting';
      mutations.receiver = 'set-receiver-lighting';
    } else if (!capability.readOnly) {
      mutations.default = `set-${capability.id}`;
    }
  }
  return mutations;
}

function compatibilityCapabilities(device: DeviceState): PluginCapability[] {
  // 能力动态协商：过滤掉设备实际不支持的能力（available=false）。
  // 无 available 字段（旧版本插件）默认可用，保持向后兼容。
  if (device.pluginCapabilities.length > 0) {
    return device.pluginCapabilities.filter((capability) => capability.available !== false);
  }
  const capabilities: PluginCapability[] = [];
  if (device.dpiStages.length > 0 || device.writableMutations.some((mutation) => mutation.startsWith('set-dpi-'))) {
    capabilities.push({
      id: 'compat-dpi', control: 'DpiStages', labelKey: 'capability.dpi', readOnly: false,
      placements: [{ region: 'control', group: 'compat-performance', order: 10, span: 1, icon: 'gauge' }],
      metadata: { label: 'DPI', source: 'dpiStages', mutations: { select: 'set-dpi-stage', value: 'set-dpi-value' } },
    });
  }
  if (device.pollingRate !== undefined || (device.supportedPollingRates?.length ?? 0) > 0 || device.writableMutations.includes('set-polling-rate')) {
    capabilities.push({
      id: 'compat-polling', control: 'Select', labelKey: 'capability.polling-rate', readOnly: false,
      placements: [{ region: 'control', group: 'compat-polling', order: 20, span: 1, icon: 'wave' }],
      metadata: {
        label: '回报率', source: 'pollingRate', mutation: 'set-polling-rate', param: 'rate', unit: 'Hz',
        options: (device.supportedPollingRates ?? []).map((value) => ({ value, label: `${value} Hz` })),
        summary: [
          { label: '运动同步', source: 'capabilities.settings.motionSync' },
          { label: '角度吸附', source: 'capabilities.settings.angleSnap' },
          { label: '抬升高度', source: 'capabilities.settings.liftCutOff' },
        ],
      },
    });
  }
  if (device.lighting || device.writableMutations.some((mutation) => mutation.endsWith('-lighting'))) {
    capabilities.push({
      id: 'compat-lighting', control: 'LightingZone', labelKey: 'capability.lighting', readOnly: false,
      placements: [{ region: 'control', group: 'compat-lighting', order: 30, span: 1, icon: 'lightbulb' }],
      metadata: {
        label: '灯光', source: 'lighting.mouseLightColor',
        mutations: { mouse: 'set-mouse-lighting', receiver: 'set-receiver-lighting' },
      },
    });
  }
  return capabilities;
}

interface CapabilityBinding {
  label: string;
  value: unknown;
  mutation?: string;
  param: string;
}

function capabilityBinding(capability: PluginCapability, device: DeviceState): CapabilityBinding {
  const bindings = Array.isArray(capability.metadata.bindings) ? capability.metadata.bindings : [];
  for (const candidate of bindings) {
    if (!candidate || typeof candidate !== 'object') continue;
    const binding = candidate as Record<string, unknown>;
    const when = binding.when && typeof binding.when === 'object' ? binding.when as Record<string, unknown> : undefined;
    if (when && readPath(device, when.path) !== when.eq) continue;
    return {
      label: typeof binding.label === 'string' ? binding.label : pluginLabel(capability),
      value: readPath(device, binding.source),
      mutation: metadataMutation(binding.mutation, device),
      param: typeof binding.param === 'string' ? binding.param : 'value',
    };
  }
  return {
    label: pluginLabel(capability),
    value: readPath(device, capability.metadata.source),
    mutation: metadataMutation(capability.metadata.mutation, device),
    param: typeof capability.metadata.param === 'string' ? capability.metadata.param : 'value',
  };
}

function capabilityVisible(capability: PluginCapability, device: DeviceState): boolean {
  const binding = capabilityBinding(capability, device);
  const mutations = Object.values(pluginMutations(capability, device));
  if (binding.value !== undefined || (binding.mutation && device.writableMutations.includes(binding.mutation))) return true;
  if (mutations.some((mutation) => device.writableMutations.includes(mutation))) return true;
  if (capability.control === 'DpiStages') return device.dpiStages.length > 0;
  if (capability.control === 'LightingZone') return device.lighting !== undefined;
  if (!capability.readOnly && Object.values(pluginMutations(capability, device)).some((mutation) => device.writableMutations.includes(mutation))) return true;
  return capability.control === 'Info';
}

function GenericPluginControl({
  capability,
  device,
  writeBusy,
  runMutation,
}: {
  capability: PluginCapability;
  device: DeviceState;
  writeBusy: boolean;
  runMutation: (mutation: string, params: Record<string, unknown>) => Promise<void>;
}) {
  const current = readPath(device, capability.metadata.source);
  const mutation = pluginMutations(capability, device).default ?? `set-${capability.id}`;
  const param = typeof capability.metadata.param === 'string' ? capability.metadata.param : 'value';
  const options = pluginOptions(capability, device);
  const [draft, setDraft] = useState<string | number>(() => typeof current === 'number' || typeof current === 'string' ? current : '');
  const [editingPollingRate, setEditingPollingRate] = useState(false);

  const writable = !capability.readOnly && device.writableMutations.includes(mutation);
  const apply = (value: unknown) => runMutation(mutation, { [param]: value });
  const description = typeof capability.metadata.description === 'string' ? capability.metadata.description : undefined;

  if (mutation === 'set-polling-rate' || capability.labelKey === 'capability.polling-rate') {
    return (
      <div className="control-reading mode-reading polling-reading">
        <WaveSine weight="regular" />
        <span>当前回报率</span>
        <button
          type="button"
          className="polling-rate editable-reading"
          aria-label={typeof current === 'number' ? `当前回报率：${current} Hz，点击编辑` : '回报率未报告，点击设置'}
          disabled={writeBusy || !writable || options.length === 0}
          onClick={() => setEditingPollingRate(true)}
        >
          <strong>{typeof current === 'number' ? current : '未报告'}</strong>
          {typeof current === 'number' && <em>Hz</em>}
        </button>
        <PluginSummary capability={capability} device={device} />
        {editingPollingRate && (
          <PollingRateEditModal
            currentValue={current}
            options={options}
            writeBusy={writeBusy}
            onClose={() => setEditingPollingRate(false)}
            onApply={(value) => {
              void apply(value);
              setEditingPollingRate(false);
            }}
          />
        )}
      </div>
    );
  }

  return (
    <div className="control-reading mode-reading plugin-control-reading">
      <UserCircle weight="regular" />
      <span>{pluginLabel(capability)}</span>
      {capability.control === 'Segmented' && options.length > 0 && (
        <div
          className="plugin-segmented"
          role="group"
          aria-label={pluginLabel(capability)}
          style={{ gridTemplateColumns: `repeat(${options.length}, minmax(0, 1fr))` }}
        >
          {options.map((option) => (
            <button
              key={String(option.value)}
              type="button"
              className={current === option.value ? 'active' : ''}
              aria-pressed={current === option.value}
              disabled={writeBusy || !writable}
              onClick={() => void apply(option.value)}
            >{option.label}</button>
          ))}
        </div>
      )}
      {capability.control === 'Select' && options.length > 0 && (
        <select
          className="plugin-select"
          aria-label={pluginLabel(capability)}
          value={String(current ?? '')}
          disabled={writeBusy || !writable}
          onChange={(event) => {
            const option = options.find((candidate) => String(candidate.value) === event.target.value);
            if (option) void apply(option.value);
          }}
        >
          {current === undefined && <option value="" disabled>未报告</option>}
          {options.map((option) => <option key={String(option.value)} value={String(option.value)}>{option.label}</option>)}
        </select>
      )}
      {capability.control === 'Toggle' && (
        <button
          type="button"
          className={`plugin-toggle ${current === true ? 'active' : ''}`}
          aria-pressed={current === true}
          disabled={writeBusy || !writable}
          onClick={() => void apply(current !== true)}
        >{current === true ? '开启' : '关闭'}</button>
      )}
      {(capability.control === 'Number' || capability.control === 'Slider' || capability.control === 'Color') && (
        <div className="plugin-input-row">
          <input
            aria-label={pluginLabel(capability)}
            type={capability.control === 'Color' ? 'color' : capability.control === 'Slider' ? 'range' : 'number'}
            value={draft}
            min={typeof capability.metadata.min === 'number' ? capability.metadata.min : undefined}
            max={typeof capability.metadata.max === 'number' ? capability.metadata.max : undefined}
            step={typeof capability.metadata.step === 'number' ? capability.metadata.step : undefined}
            disabled={writeBusy || !writable}
            onChange={(event) => setDraft(capability.control === 'Color' ? event.target.value : Number(event.target.value))}
          />
          <button type="button" disabled={writeBusy || !writable} onClick={() => void apply(draft)}>应用</button>
        </div>
      )}
      {capability.control === 'Action' && (
        <button
          type="button"
          className="plugin-action"
          disabled={writeBusy || !writable}
          onClick={() => void runMutation(mutation, (capability.metadata.params as Record<string, unknown>) ?? {})}
        >{typeof capability.metadata.actionLabel === 'string' ? capability.metadata.actionLabel : '执行'}</button>
      )}
      {(capability.readOnly || capability.control === 'ReadOnlyValue') && <strong className="plugin-current-value">{pluginValueLabel(capability, current)}</strong>}
      <PluginSummary capability={capability} device={device} />
      {description && <p>{description}</p>}
      {!writable && !capability.readOnly && <p className="setting-hint">当前设备未开放这项写入。</p>}
    </div>
  );
}

function DeviceDetails({ capabilities, pluginCapabilities, onClose }: { capabilities: DeviceCapabilities; pluginCapabilities: PluginCapability[]; onClose: () => void }) {
  const detailOrder = new Map<string, number>();
  for (const capability of pluginCapabilities) {
    const source = typeof capability.metadata.source === 'string' ? capability.metadata.source : '';
    const group = /^capabilities\.([^.]+)$/.exec(source)?.[1];
    const placement = placementsFor(capability, 'details')[0];
    if (group && placement) detailOrder.set(group, placement.order);
  }
  const groups = Object.entries(capabilities)
    .filter(([, fields]) => fields && Object.keys(fields).length > 0)
    .sort(([a], [b]) => (detailOrder.get(a) ?? 10_000) - (detailOrder.get(b) ?? 10_000));
  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => { if (event.key === 'Escape') onClose(); };
    window.addEventListener('keydown', closeOnEscape);
    return () => window.removeEventListener('keydown', closeOnEscape);
  }, [onClose]);
  return (
    <div className="details-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <section className="device-details" role="dialog" aria-modal="true" aria-labelledby="device-details-title">
        <header>
          <div><p className="eyebrow">只读设备报告</p><h2 id="device-details-title">全部读取信息</h2></div>
          <button className="icon-button" onClick={onClose} aria-label="关闭设备详情"><X weight="regular" /></button>
        </header>
        <p className="details-note">以下是已签名设备插件返回的完整只读报告；经过验证的可写项目会在主界面提供独立控件。</p>
        <div className="capability-groups">
          {groups.length ? groups.map(([group, fields]) => (
            <section className="capability-group" key={group}>
              <h3>{CAPABILITY_GROUP_LABELS[group] ?? group}</h3>
              <dl>
                {Object.entries(fields).map(([key, value]) => (
                  <div key={key}>
                    <dt>{CAPABILITY_FIELD_LABELS[key] ?? key}</dt>
                    <dd>{capabilityValue(value, key)}</dd>
                  </div>
                ))}
              </dl>
            </section>
          )) : <p className="setting-hint">设备没有报告扩展能力字段。</p>}
        </div>
      </section>
    </div>
  );
}

interface EditModalProps {
  title: string;
  children: React.ReactNode;
  submitLabel?: string;
  submitDisabled?: boolean;
  onClose: () => void;
  onSubmit: () => void;
}

function EditModal({ title, children, submitLabel = '应用', submitDisabled, onClose, onSubmit }: EditModalProps) {
  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => { if (event.key === 'Escape') onClose(); };
    window.addEventListener('keydown', closeOnEscape);
    return () => window.removeEventListener('keydown', closeOnEscape);
  }, [onClose]);
  return (
    <div className="edit-modal-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <form
        className="edit-modal"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onSubmit={(event) => { event.preventDefault(); onSubmit(); }}
      >
        <header>
          <h3>{title}</h3>
        </header>
        <div className="edit-modal-body">{children}</div>
        <footer>
          <button type="button" className="secondary" onClick={onClose}>取消</button>
          <button type="submit" disabled={submitDisabled}>{submitLabel}</button>
        </footer>
      </form>
    </div>
  );
}

interface DpiEditModalProps {
  stage: number;
  currentValue: number;
  writeBusy: boolean;
  onClose: () => void;
  onApply: (value: number) => void;
}

function DpiEditModal({ stage, currentValue, writeBusy, onClose, onApply }: DpiEditModalProps) {
  const [draft, setDraft] = useState(currentValue);
  return (
    <EditModal
      title={`编辑第 ${stage} 档 DPI`}
      submitDisabled={writeBusy || draft < 50 || draft > 30000 || draft === currentValue}
      onClose={onClose}
      onSubmit={() => onApply(draft)}
    >
      <label className="edit-field">
        <span>DPI 数值</span>
        <input
          type="number"
          min={50}
          max={30000}
          step={50}
          autoFocus
          value={draft}
          disabled={writeBusy}
          onChange={(event) => setDraft(Number(event.target.value))}
        />
      </label>
    </EditModal>
  );
}

function PollingRateEditModal({ currentValue, options, writeBusy, onClose, onApply }: {
  currentValue: unknown;
  options: PluginOption[];
  writeBusy: boolean;
  onClose: () => void;
  onApply: (value: PluginOption['value']) => void;
}) {
  const [draft, setDraft] = useState(String(currentValue ?? options[0]?.value ?? ''));
  const selected = options.find((option) => String(option.value) === draft);
  return (
    <EditModal
      title="设置回报率"
      submitDisabled={writeBusy || !selected || selected.value === currentValue}
      onClose={onClose}
      onSubmit={() => selected && onApply(selected.value)}
    >
      <label className="edit-field">
        <span>回报率</span>
        <select
          autoFocus
          aria-label="回报率"
          value={draft}
          disabled={writeBusy}
          onChange={(event) => setDraft(event.target.value)}
        >
          {options.map((option) => (
            <option key={String(option.value)} value={String(option.value)}>{option.label}</option>
          ))}
        </select>
      </label>
    </EditModal>
  );
}

interface ColorEditModalProps {
  title: string;
  currentColor: string;
  writeBusy: boolean;
  onClose: () => void;
  onApply: (color: string) => void;
}

function ColorEditModal({ title, currentColor, writeBusy, onClose, onApply }: ColorEditModalProps) {
  const [draft, setDraft] = useState(currentColor);
  return (
    <EditModal
      title={title}
      submitDisabled={writeBusy || draft === currentColor}
      onClose={onClose}
      onSubmit={() => onApply(draft)}
    >
      <label className="edit-field color-field">
        <span>颜色</span>
        <input
          type="color"
          value={draft}
          disabled={writeBusy}
          onChange={(event) => setDraft(event.target.value)}
        />
      </label>
    </EditModal>
  );
}

function SleepEditModal({ label, currentSeconds, writeBusy, onClose, onApply }: {
  label: string;
  currentSeconds: number;
  writeBusy: boolean;
  onClose: () => void;
  onApply: (seconds: number) => void;
}) {
  const [draft, setDraft] = useState(currentSeconds);
  return (
    <EditModal
      title={`设置${label}`}
      submitDisabled={writeBusy || draft < 10 || draft > 65535 || draft === currentSeconds}
      onClose={onClose}
      onSubmit={() => onApply(draft)}
    >
      <label className="edit-field">
        <span>超时时间（秒）</span>
        <input
          type="number"
          min={10}
          max={65535}
          step={10}
          autoFocus
          value={draft}
          disabled={writeBusy}
          onChange={(event) => setDraft(Number(event.target.value))}
        />
      </label>
    </EditModal>
  );
}

type ReceiverLightingField = 'effect' | 'option' | 'speed' | 'brightness' | 'color';

interface ReceiverLightingOption {
  value: number;
  label: string;
}

interface ReceiverLightingOptions {
  effect: ReceiverLightingOption[];
  speed: ReceiverLightingOption[];
  brightness: ReceiverLightingOption[];
  option: ReceiverLightingOption[];
}

// 向后兼容回退：当插件 manifest 未声明 receiverLightingOptions 时使用。
// 这些值对应 AMaster 协议的固有能力档位，插件声明后优先使用插件值。
const DEFAULT_RECEIVER_LIGHTING_OPTIONS: ReceiverLightingOptions = {
  effect: [
    { value: 0, label: '关闭' },
    { value: 1, label: '常亮' },
    { value: 2, label: '呼吸' },
    { value: 3, label: '霓虹' },
    { value: 4, label: '光波' },
    { value: 5, label: '跑马' },
    { value: 6, label: '圆环' },
    { value: 7, label: '缓冲' },
    { value: 8, label: '追捕' },
  ],
  speed: [
    { value: 0, label: '最快' },
    { value: 1, label: '快' },
    { value: 2, label: '中' },
    { value: 3, label: '慢' },
    { value: 4, label: '最慢' },
  ],
  brightness: [
    { value: 0, label: '最暗' },
    { value: 1, label: '暗' },
    { value: 2, label: '中' },
    { value: 3, label: '亮' },
    { value: 4, label: '最亮' },
  ],
  option: [
    { value: 0, label: '红' },
    { value: 1, label: '橙' },
    { value: 2, label: '黄' },
    { value: 3, label: '绿' },
    { value: 4, label: '青' },
    { value: 5, label: '蓝' },
    { value: 6, label: '紫' },
    { value: 7, label: '自定义' },
    { value: 8, label: '炫彩' },
  ],
};

function getReceiverLightingOptions(pluginCapabilities: PluginCapability[]): ReceiverLightingOptions {
  const lighting = pluginCapabilities.find((c) => c.id === 'lighting');
  const declared = lighting?.metadata?.receiverLightingOptions as ReceiverLightingOptions | undefined;
  return declared ?? DEFAULT_RECEIVER_LIGHTING_OPTIONS;
}

interface ReceiverLightingEditModalProps {
  field: ReceiverLightingField;
  initial: {
    effect: number;
    speed: number;
    brightness: number;
    option: number;
    color: string;
  };
  options: ReceiverLightingOptions;
  writeBusy: boolean;
  onClose: () => void;
  onApply: (params: {
    effect: number;
    speed: number;
    brightness: number;
    option: number;
    color: string;
  }) => void;
}

const RECEIVER_LIGHTING_FIELD_LABELS: Record<ReceiverLightingField, string> = {
  effect: '灯效',
  option: '颜色模式',
  speed: '速度',
  brightness: '亮度',
  color: '颜色',
};

function ReceiverLightingEditModal({ field, initial, options, writeBusy, onClose, onApply }: ReceiverLightingEditModalProps) {
  const [draft, setDraft] = useState(initial);
  return (
    <EditModal
      title={`编辑接收器${RECEIVER_LIGHTING_FIELD_LABELS[field]}`}
      submitDisabled={writeBusy || JSON.stringify(draft) === JSON.stringify(initial)}
      onClose={onClose}
      onSubmit={() => onApply(draft)}
    >
      {field === 'effect' && <label className="edit-field">
        <span>灯效</span>
        <select
          value={draft.effect}
          disabled={writeBusy}
          onChange={(event) => setDraft({ ...draft, effect: Number(event.target.value) })}
        >
          {options.effect.map((opt) => (
            <option key={opt.value} value={opt.value}>{opt.label}</option>
          ))}
        </select>
      </label>}
      {field === 'speed' && <label className="edit-field">
        <span>速度</span>
        <select
          value={draft.speed}
          disabled={writeBusy}
          onChange={(event) => setDraft({ ...draft, speed: Number(event.target.value) })}
        >
          {options.speed.map((opt) => (
            <option key={opt.value} value={opt.value}>{opt.label}</option>
          ))}
        </select>
      </label>}
      {field === 'brightness' && <label className="edit-field">
        <span>亮度</span>
        <select
          value={draft.brightness}
          disabled={writeBusy}
          onChange={(event) => setDraft({ ...draft, brightness: Number(event.target.value) })}
        >
          {options.brightness.map((opt) => (
            <option key={opt.value} value={opt.value}>{opt.label}</option>
          ))}
        </select>
      </label>}
      {field === 'option' && <label className="edit-field">
        <span>颜色模式</span>
        <select
          value={draft.option}
          disabled={writeBusy}
          onChange={(event) => setDraft({ ...draft, option: Number(event.target.value) })}
        >
          {options.option.map((opt) => (
            <option key={opt.value} value={opt.value}>{opt.label}</option>
          ))}
        </select>
      </label>}
      {field === 'color' && <label className="edit-field color-field">
        <span>颜色</span>
        <input
          type="color"
          value={draft.color}
          disabled={writeBusy}
          onChange={(event) => setDraft({ ...draft, color: event.target.value })}
        />
      </label>}
    </EditModal>
  );
}

function Dashboard({ device, onDeviceChange }: { device: DeviceState; onDeviceChange: (device: DeviceState) => void }) {
  const stages = device.dpiStages.filter((stage) => stage.enabled);
  // 界面最多只渲染 8 个 DPI 档位，避免过宽导致布局撑大。
  const displayedStages = stages.slice(0, 8);
  const current = stages.find((stage) => stage.active);
  const initialDpi = current?.value ?? stages[0]?.value ?? 0;
  const [mode, setMode] = useState<ControlMode>('dpi');
  const [lightingView, setLightingView] = useState<'mouse' | 'receiver'>('mouse');
  const [previewMessage, setPreviewMessage] = useState('');
  const [showDetails, setShowDetails] = useState(false);
  const [showBatteries, setShowBatteries] = useState(false);
  const [suppressBatteryHover, setSuppressBatteryHover] = useState(false);
  const batteryControlRef = useRef<HTMLDivElement>(null);
  const [writeBusy, setWriteBusy] = useState(false);
  const [editingDpiStage, setEditingDpiStage] = useState<number | null>(null);
  const [editingMouseLightColor, setEditingMouseLightColor] = useState(false);
  const [editingMouseLightEndColor, setEditingMouseLightEndColor] = useState(false);
  const [editingReceiverLighting, setEditingReceiverLighting] = useState<ReceiverLightingField | null>(null);
  const activeDpi = initialDpi;
  const writable = (mutation: string) => device.writableMutations.includes(mutation);

  useEffect(() => {
    const closeOnOutsideClick = (event: MouseEvent) => {
      if (!batteryControlRef.current?.contains(event.target as Node)) setShowBatteries(false);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setShowBatteries(false);
    };
    document.addEventListener('click', closeOnOutsideClick);
    document.addEventListener('keydown', closeOnEscape);
    return () => {
      document.removeEventListener('click', closeOnOutsideClick);
      document.removeEventListener('keydown', closeOnEscape);
    };
  }, []);

  const runMutation = async (
    mutation: string,
    params: Record<string, unknown>,
  ) => {
    setWriteBusy(true);
    try {
      const snapshot = await invoke<DeviceSnapshot>('device_mutate', { mutation, params });
      onDeviceChange(snapshotToState(snapshot));
      setPreviewMessage('已写入');
      setTimeout(() => setPreviewMessage(''), 1500);
    } catch (error) {
      // #5 事务可观测性：错误信息已包含事务详情（snapshot/rollback workflow 名称）。
      notifyError('写入失败', String(error));
    } finally {
      setWriteBusy(false);
    }
  };

  const currentStage = Math.max(1, stages.findIndex((stage) => stage.active) + 1);
  const receiverLighting = lightingCapability(device.capabilities, 'receiverLighting');
  const [sleepSetting, setSleepSetting] = useState<{ label: string; seconds: number; mutation: string; param: string }>();
  const [editingSleep, setEditingSleep] = useState(false);
  const pluginDescriptors = compatibilityCapabilities(device);
  const controlPlacements = pluginDescriptors
    .flatMap((capability) => placementsFor(capability, 'control').map((placement) => ({ capability, placement })))
    .filter(({ capability }) => capabilityVisible(capability, device))
    .sort((a, b) => a.placement.order - b.placement.order);
  const controlGroups = new Map<string, { id: string; label: string; icon: PluginIcon; capabilities: PluginCapability[] }>();
  for (const { capability, placement } of controlPlacements) {
    const id = placement.group || capability.id;
    const existing = controlGroups.get(id);
    if (existing) existing.capabilities.push(capability);
    else controlGroups.set(id, { id, label: pluginLabel(capability), icon: pluginIcon(placement.icon), capabilities: [capability] });
  }
  const controls = [...controlGroups.values()].slice(0, MAX_CONTROL_GROUPS);
  const activeMode = controls.some((control) => control.id === mode) ? mode : controls[0]?.id;
  const activeDescriptors = activeMode ? controlGroups.get(activeMode)?.capabilities ?? [] : [];
  const activeDpiDescriptor = activeDescriptors.find((capability) => capability.control === 'DpiStages');
  const activeLightingDescriptor = activeDescriptors.find((capability) => capability.control === 'LightingZone');
  const activeGenericDescriptors = activeDescriptors.filter((capability) => !['DpiStages', 'LightingZone'].includes(capability.control));
  const dpiMutations = activeDpiDescriptor ? pluginMutations(activeDpiDescriptor, device) : {};
  const selectDpiMutation = dpiMutations.select;
  const setDpiMutation = dpiMutations.value;
  const lightingMutations = activeLightingDescriptor ? pluginMutations(activeLightingDescriptor, device) : {};
  const mouseLightingMutation = lightingMutations.mouse;
  const receiverLightingMutation = lightingMutations.receiver;
  const supportsReceiverLighting = Boolean(receiverLightingMutation && writable(receiverLightingMutation))
    || Boolean(device.lighting?.receiverLightColor)
    || Boolean(device.capabilities.receiverLighting);
  const activeLightingView = lightingView === 'receiver' && !supportsReceiverLighting ? 'mouse' : lightingView;
  const statusItems: {
    id: string;
    label: string;
    value: string;
    icon: typeof Gauge;
    disabled?: boolean;
    color?: string;
    onClick?: () => void;
  }[] = [];
  const statusPlacements = pluginDescriptors
    .flatMap((capability) => placementsFor(capability, 'status').map((placement) => ({ capability, placement })))
    .filter(({ capability }) => capabilityVisible(capability, device))
    .sort((a, b) => a.placement.order - b.placement.order)
    .slice(0, MAX_STATUS_ITEMS);
  for (const { capability, placement } of statusPlacements) {
    const binding = capabilityBinding(capability, device);
    const controlPlacement = placementsFor(capability, 'control')[0];
    if (capability.metadata.format === 'sleep' || (capability.control === 'Number' && Array.isArray(capability.metadata.bindings))) {
      const seconds = Number(binding.value);
      if (!Number.isFinite(seconds) || !binding.mutation) continue;
      statusItems.push({
        id: capability.id, label: binding.label, value: formatSleepTime(seconds), icon: pluginIcon(placement.icon),
        disabled: !writable(binding.mutation),
        onClick: () => {
          setSleepSetting({ label: binding.label, seconds, mutation: binding.mutation!, param: binding.param });
          setEditingSleep(true);
        },
      });
    } else if (capability.control === 'LightingZone') {
      const mutation = pluginMutations(capability, device).mouse;
      statusItems.push({
        id: capability.id, label: binding.label,
        value: device.lighting?.mouseLightEnabled === false ? '已关闭' : device.lighting?.mouseLightColor ?? '未报告',
        icon: pluginIcon(placement.icon), color: device.lighting?.mouseLightColor,
        disabled: !mutation || !writable(mutation), onClick: () => setEditingMouseLightColor(true),
      });
    } else if (binding.value !== undefined) {
      const target = controlPlacement?.group || (controlPlacement ? capability.id : undefined);
      statusItems.push({
        id: capability.id, label: binding.label, value: pluginValueLabel(capability, binding.value),
        icon: pluginIcon(placement.icon),
        disabled: !target, onClick: target ? () => setMode(target) : undefined,
      });
    }
  }

  return (
    <main className="dashboard">
      <section className="device-hero" aria-label="已连接设备">
        <div className="device-column">
          <h2 className="app-title">Mira</h2>
          <div className="device-copy">
            <p className="connection-state"><span />{device.connection} · 已连接</p>
            <h1>{device.name}</h1>
            {device.readonly && <p className="readonly-notice">未信任插件 · 只读模式</p>}
            {device.batteries.length > 0 && (
            <div
              ref={batteryControlRef}
              className={`battery-control ${showBatteries ? 'open' : ''} ${suppressBatteryHover ? 'hover-suppressed' : ''}`}
              onMouseLeave={() => setSuppressBatteryHover(false)}
            >
              <button
                className="battery-state"
                aria-expanded={showBatteries}
                aria-controls="device-batteries"
                onClick={() => setShowBatteries((visible) => {
                  setSuppressBatteryHover(visible);
                  return !visible;
                })}
              >
                <BatteryHigh weight="regular" />
                {device.batteries[0].percentage}%
                {device.batteries[0].charging ? ' · 充电中' : ''}
                <span className="battery-count">{device.batteries.length} 台设备</span>
              </button>
              <section id="device-batteries" className="battery-popover" aria-label="设备电量">
                <p>设备电量</p>
                {device.batteries.map((battery) => (
                  <div key={battery.id} className="battery-device">
                    <span><BatteryHigh weight="regular" />{battery.label}</span>
                    <strong>{battery.percentage}%{battery.charging ? ' · 充电中' : ''}</strong>
                  </div>
                ))}
              </section>
            </div>
            )}
          </div>
        </div>
        <DeviceAura color={device.lighting?.mouseLightColor ?? device.lighting?.color} />
      </section>

      <div
        className="control-tabs"
        role="tablist"
        aria-label="设备控制"
        style={{
          gridTemplateColumns: `repeat(${Math.max(controls.length, 1)}, minmax(0, 1fr))`,
          width: `min(92%, ${Math.max(220, controls.length * 104)}px)`,
        }}
      >
        {controls.map(({ id, label, icon: ControlIcon }) => (
          <button
            key={id}
            role="tab"
            aria-selected={activeMode === id}
            className={activeMode === id ? 'active' : ''}
            onClick={() => { setMode(id); setPreviewMessage(''); }}
          >
            <ControlIcon weight="regular" />
            <span>{label}</span>
          </button>
        ))}
      </div>

      <section className="control-stage" aria-live="polite">
        {activeDpiDescriptor && (
          <div className="control-reading dpi-reading">
            <button
              type="button"
              className="primary-reading editable-reading"
              aria-label={activeDpi ? `当前 DPI：${activeDpi}，点击编辑` : 'DPI 未报告'}
              disabled={writeBusy || !setDpiMutation || !writable(setDpiMutation) || !activeDpi}
              onClick={() => activeDpi && setEditingDpiStage(currentStage)}
            >
              <strong>{activeDpi || '未报告'}</strong><em>DPI</em>
            </button>
            <div className="dpi-scale" aria-label="DPI 档位" style={{ '--stage-count': Math.max(displayedStages.length, 1) } as React.CSSProperties}>
              {displayedStages.map((stage, index) => {
                const stageNumber = index + 1;
                return (
                  <div key={`${index}-${stage.value}`} className="dpi-stage-item">
                    <button
                      type="button"
                      className={`dpi-stage-dot ${stage.active ? 'active' : ''}`}
                      aria-pressed={stage.active}
                      disabled={writeBusy || !selectDpiMutation || !writable(selectDpiMutation)}
                      onClick={() => runMutation(
                        selectDpiMutation!,
                        { stage: stageNumber },
                      )}
                      aria-label={`切换到第 ${stageNumber} 档`}
                    >
                      <i style={{ '--stage-source-color': stage.color } as React.CSSProperties} />
                    </button>
                    <button
                      type="button"
                      className="dpi-stage-value"
                      disabled={writeBusy || !setDpiMutation || !writable(setDpiMutation)}
                      onClick={() => setEditingDpiStage(stageNumber)}
                      aria-label={`编辑第 ${stageNumber} 档 DPI`}
                    >
                      {stage.value}
                    </button>
                  </div>
                );
              })}
            </div>
            {displayedStages.length === 0 && <p className="setting-hint">设备未报告 DPI 档位信息。</p>}
            {(!setDpiMutation || !writable(setDpiMutation)) && displayedStages.length > 0 && <p className="setting-hint">当前设备或插件未开放 DPI 写入。</p>}
            {editingDpiStage !== null && (
              <DpiEditModal
                stage={editingDpiStage}
                currentValue={stages[editingDpiStage - 1]?.value ?? activeDpi}
                writeBusy={writeBusy}
                onClose={() => setEditingDpiStage(null)}
                onApply={(value) => {
                  void runMutation(
                    setDpiMutation!,
                    { stage: editingDpiStage, dpi: value },
                  );
                  setEditingDpiStage(null);
                }}
              />
            )}
          </div>
        )}

        {activeLightingDescriptor && (
          <div className="control-reading mode-reading lighting-reading">
            <div
              className="lighting-sub-tabs"
              role="tablist"
              aria-label="灯光对象"
              style={{ gridTemplateColumns: `repeat(${supportsReceiverLighting ? 2 : 1}, minmax(0, 1fr))` }}
            >
              <button
                role="tab"
                aria-selected={lightingView === 'mouse'}
                className={lightingView === 'mouse' ? 'active' : ''}
                onClick={() => setLightingView('mouse')}
              >鼠标灯光</button>
              {supportsReceiverLighting && (
                <button
                  role="tab"
                  aria-selected={lightingView === 'receiver'}
                  className={lightingView === 'receiver' ? 'active' : ''}
                  onClick={() => setLightingView('receiver')}
                >接收器灯光</button>
              )}
            </div>
            {supportsReceiverLighting && <p className="lighting-hint">鼠标与接收器灯光分别读取，互不混用。</p>}
            <div className="lighting-swatch" style={{ '--light-color': (
              activeLightingView === 'mouse' ? device.lighting?.mouseLightColor : device.lighting?.receiverLightColor
            ) ?? '#b87ab0' } as React.CSSProperties} />
            <div className="lighting-sections" aria-label="灯光分组">
              {activeLightingView === 'mouse' && (
                <div className="lighting-group lighting-group-mouse">
                  <p className="lighting-group-title">鼠标灯光</p>
                  <div
                    className="lighting-rows"
                    style={{
                      gridTemplateColumns: `repeat(${device.lighting?.mouseLightEndColor && device.lighting.mouseLightEndColor !== device.lighting.mouseLightColor ? 3 : 2}, minmax(0, 1fr))`,
                    }}
                  >
                    <button
                      type="button"
                      className="lighting-row"
                      disabled={writeBusy || !mouseLightingMutation || !writable(mouseLightingMutation)}
                      onClick={() => {
                        const enabled = device.lighting?.mouseLightEnabled === false;
                        void runMutation(
                          mouseLightingMutation!,
                          { color: device.lighting?.mouseLightColor ?? '#b87ab0', enabled },
                        );
                      }}
                    >
                      <span>状态</span>
                      <strong>{device.lighting?.mouseLightEnabled === false ? '关闭' : '开启'}</strong>
                    </button>
                    <button
                      type="button"
                      className="lighting-row"
                      disabled={writeBusy || !mouseLightingMutation || !writable(mouseLightingMutation)}
                      onClick={() => setEditingMouseLightColor(true)}
                    >
                      <span>颜色</span>
                      <strong>{device.lighting?.mouseLightColor ?? '未报告'}</strong>
                    </button>
                    {device.lighting?.mouseLightEndColor && device.lighting.mouseLightEndColor !== device.lighting.mouseLightColor && (
                      <button
                        type="button"
                        className="lighting-row"
                        disabled={writeBusy || !mouseLightingMutation || !writable(mouseLightingMutation)}
                        onClick={() => setEditingMouseLightEndColor(true)}
                      >
                        <span>结束色</span>
                        <strong>{device.lighting.mouseLightEndColor}</strong>
                      </button>
                    )}
                  </div>
                  {editingMouseLightEndColor && (
                    <ColorEditModal
                      title="鼠标灯光结束色"
                      currentColor={device.lighting?.mouseLightEndColor ?? '#b87ab0'}
                      writeBusy={writeBusy}
                      onClose={() => setEditingMouseLightEndColor(false)}
                      onApply={(color) => {
                        void runMutation(
                          mouseLightingMutation!,
                          { color, enabled: device.lighting?.mouseLightEnabled !== false },
                        );
                        setEditingMouseLightEndColor(false);
                      }}
                    />
                  )}
                </div>
              )}
              {activeLightingView === 'receiver' && (
                <div className="lighting-group lighting-group-dongle">
                  <p className="lighting-group-title">接收器灯光</p>
                  <div className="lighting-rows" style={{ gridTemplateColumns: 'repeat(5, minmax(0, 1fr))' }}>
                    <button
                      type="button"
                      className="lighting-row"
                      disabled={writeBusy || !receiverLightingMutation || !writable(receiverLightingMutation)}
                      onClick={() => setEditingReceiverLighting('effect')}
                    >
                      <span>灯效</span>
                      <strong>{device.lighting?.receiverLightMode ?? '未报告'}</strong>
                    </button>
                    <button
                      type="button"
                      className="lighting-row"
                      disabled={writeBusy || !receiverLightingMutation || !writable(receiverLightingMutation)}
                      onClick={() => setEditingReceiverLighting('option')}
                    >
                      <span>颜色模式</span>
                      <strong>{getLightingColorMode(device.capabilities, 'receiverLighting')}</strong>
                    </button>
                    <button
                      type="button"
                      className="lighting-row"
                      disabled={writeBusy || !receiverLightingMutation || !writable(receiverLightingMutation)}
                      onClick={() => setEditingReceiverLighting('speed')}
                    >
                      <span>速度</span>
                      <strong>{preferredCapability(device.capabilities, 'receiverLighting', 'speedLabel', 'speed')}</strong>
                    </button>
                    <button
                      type="button"
                      className="lighting-row"
                      disabled={writeBusy || !receiverLightingMutation || !writable(receiverLightingMutation)}
                      onClick={() => setEditingReceiverLighting('brightness')}
                    >
                      <span>亮度</span>
                      <strong>{preferredCapability(device.capabilities, 'receiverLighting', 'brightnessLabel', 'brightness')}</strong>
                    </button>
                    <button
                      type="button"
                      className="lighting-row"
                      disabled={writeBusy || !receiverLightingMutation || !writable(receiverLightingMutation)}
                      onClick={() => setEditingReceiverLighting('color')}
                    >
                      <span>颜色</span>
                      <strong style={{ color: typeof receiverLighting?.color === 'string' ? receiverLighting.color : undefined }}>{typeof receiverLighting?.color === 'string' ? receiverLighting.color : '未报告'}</strong>
                    </button>
                  </div>
                  {editingReceiverLighting !== null && receiverLighting && (
                    <ReceiverLightingEditModal
                      field={editingReceiverLighting}
                      initial={{
                        effect: Number(receiverLighting.effect ?? 0),
                        speed: Number(receiverLighting.speed ?? 0),
                        brightness: Number(receiverLighting.brightness ?? 0),
                        option: Number(receiverLighting.option ?? 0),
                        color: String(receiverLighting.color ?? '#b87ab0'),
                      }}
                      options={getReceiverLightingOptions(device.pluginCapabilities)}
                      writeBusy={writeBusy}
                      onClose={() => setEditingReceiverLighting(null)}
                      onApply={(params) => {
                        void runMutation(receiverLightingMutation!, params);
                        setEditingReceiverLighting(null);
                      }}
                    />
                  )}
                </div>
              )}
            </div>
          </div>
        )}
        {activeGenericDescriptors.map((descriptor) => (
          <GenericPluginControl
            key={`${descriptor.id}:${String(readPath(device, descriptor.metadata.source))}`}
            capability={descriptor}
            device={device}
            writeBusy={writeBusy}
            runMutation={runMutation}
          />
        ))}
        {previewMessage && <span className="write-badge">{previewMessage}</span>}
        {editingSleep && sleepSetting && (
          <SleepEditModal
            label={sleepSetting.label}
            currentSeconds={sleepSetting.seconds}
            writeBusy={writeBusy}
            onClose={() => setEditingSleep(false)}
            onApply={(seconds) => {
              void runMutation(sleepSetting.mutation, { [sleepSetting.param]: seconds });
              setEditingSleep(false);
            }}
          />
        )}
        {editingMouseLightColor && (
          <ColorEditModal
            title="鼠标灯光颜色"
            currentColor={device.lighting?.mouseLightColor ?? '#b87ab0'}
            writeBusy={writeBusy}
            onClose={() => setEditingMouseLightColor(false)}
            onApply={(color) => {
              const statusLighting = statusPlacements.find(({ capability }) => capability.control === 'LightingZone')?.capability;
              const mutation = statusLighting ? pluginMutations(statusLighting, device).mouse : mouseLightingMutation;
              if (!mutation) return;
              void runMutation(
                mutation,
                { color, enabled: device.lighting?.mouseLightEnabled !== false },
              );
              setEditingMouseLightColor(false);
            }}
          />
        )}
      </section>

      {statusItems.length > 0 && (
      <section
        className="status-strip"
        aria-label="设备状态"
        style={{ gridTemplateColumns: `repeat(${statusItems.length}, minmax(0, 1fr))` }}
      >
        {statusItems.map(({ id, label, value, icon: StatusIcon, disabled, color, onClick }) => {
          const content = <><StatusIcon weight="regular" /><span>{label}<strong>{value}</strong></span>{color && <i style={{ '--light-color': color } as React.CSSProperties} />}</>;
          return onClick
            ? <button key={id} type="button" disabled={disabled} onClick={onClick}>{content}</button>
            : <div key={id}>{content}</div>;
        })}
      </section>
      )}
      <div className="dashboard-meta">
        <span>最后更新：{device.updatedAt}</span>
        <button className="details-button" onClick={() => setShowDetails(true)}><ReadCvLogo weight="regular" />全部读取信息</button>
      </div>
      {showDetails && <DeviceDetails capabilities={device.capabilities} pluginCapabilities={pluginDescriptors} onClose={() => setShowDetails(false)} />}
    </main>
  );
}

export default function App() {
  const pureWeb = isPureWebPreview();
  const [device, setDevice] = useState<DeviceState | undefined>(pureWeb ? MOCK_DEVICE : undefined);
  const [theme, setTheme] = useState<ThemeMode>('system');
  const [view, setView] = useState<View>('dashboard');
  const [demoMode, setDemoMode] = useState(pureWeb);
  const [, setRefreshIntervalSeconds] = useState(5);
  const [refreshNonce, setRefreshNonce] = useState(0);
  const [appNotification, setAppNotification] = useState<AppNotification>();
  const windowsPlatform = isWindowsPlatform();
  const macPlatform = isMacPlatform();
  const windowsWebPreview = isWindowsWebPreview();

  useEffect(() => onAppNotification(setAppNotification), []);

  useEffect(() => {
    if (!appNotification) return;
    const timeout = window.setTimeout(() => setAppNotification(undefined), 6000);
    return () => window.clearTimeout(timeout);
  }, [appNotification]);

  // 触发后台立即刷新设备状态，失败时通知用户
  const refreshDevice = () => {
    setDemoMode(false);
    setDevice(undefined);
    setRefreshNonce((value) => value + 1);
    invoke('device_refresh').catch((error) => notifyError('刷新设备失败', String(error)));
  };

  // 从后端加载已保存的主题设置
  useEffect(() => {
    if (pureWeb) return;
    invoke<AppSettings>('settings_get')
      .then((settings) => {
        setTheme(settings.theme as ThemeMode);
        setRefreshIntervalSeconds(Math.min(60, Math.max(1, settings.refreshIntervalSeconds || 5)));
        if (settings.automaticUpdateChecks) {
          void invoke<AboutInfo>('about_info')
            .then((info) => {
              if (info.updaterActive) return startAutomaticAppUpdateCheck(true, settings.automaticUpdateInstall);
            })
            .catch(() => { /* Pre-release and offline builds skip automatic application checks. */ });
        }
        if (settings.automaticPluginUpdateChecks && !automaticPluginCheckStarted) {
          automaticPluginCheckStarted = true;
          void invoke<PluginUpdateInfo[]>('plugin_updates_check')
            .then((updates) => {
              const available = updates.filter((item) => item.updateAvailable);
              if (available.length > 0) notifyInfo('发现插件更新', `${available.length} 个已安装插件有新版本，可在“设置 → 插件”中更新。`);
            })
            .catch(() => { /* Automatic checks stay quiet when offline. */ });
        }
      })
      .catch((error) => console.warn('settings_get failed:', error));
  }, [pureWeb]);

  // 周期性从后端读取真实设备状态
  useEffect(() => {
    if (demoMode) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    // 启动时立即读取一次缓存
    invoke<DeviceSnapshot | null>('device_snapshot')
      .then((snapshot) => {
        if (!cancelled) {
          setDevice(snapshot ? snapshotToState(snapshot) : undefined);
        }
      })
      .catch(() => {
        if (!cancelled) setDevice(undefined);
      });

    // 监听后台线程发出的 device-updated 事件，无需轮询
    listen<DeviceSnapshot | null>('device-updated', (event) => {
      if (cancelled) return;
      const snapshot = event.payload;
      setDevice(snapshot ? snapshotToState(snapshot) : undefined);
    }).then((un) => {
      // 修复竞态泄漏：若组件在 listen 注册完成前已卸载，立即注销监听器，
      // 否则 unlisten 永远不会被调用，导致监听器驻留后端进程。
      if (cancelled) {
        un();
      } else {
        unlisten = un;
      }
    }).catch((error) => console.warn('device-updated listener failed:', error));

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [demoMode, refreshNonce]);

  // 仅在主题或灯光颜色实际变化时重新应用，避免每次设备快照更新都触发
  const themeColor = device?.lighting?.mouseLightColor ?? device?.lighting?.color;
  useEffect(() => applyTheme(theme, themeColor), [theme, themeColor]);

  return <div className={`app-shell ${pureWeb ? 'web-preview' : ''} ${windowsPlatform ? 'platform-windows' : ''} ${macPlatform ? 'platform-macos' : ''} ${windowsWebPreview ? 'windows-web-preview' : ''}`}>
    {windowsWebPreview && <WindowsPreviewControls />}
    <nav className="top-nav" data-tauri-drag-region>
      <div className="nav-links">
        <button className={`nav-link ${view === 'dashboard' ? 'active' : ''}`} onClick={() => setView('dashboard')}>设备</button>
        <button className={`nav-link ${view === 'settings' ? 'active' : ''}`} onClick={() => setView('settings')}>设置</button>
        <button className={`nav-link nav-about ${view === 'about' ? 'active' : ''}`} onClick={() => setView('about')} aria-label="关于 Mira"><Info weight="regular" /></button>
        {demoMode && <button className="nav-link nav-exit" onClick={refreshDevice} aria-label="退出演示" title="退出演示"><SignOut weight="regular" /></button>}
      </div>
    </nav>
    {view === 'dashboard' && (device ? <Dashboard device={device} onDeviceChange={setDevice} /> : <EmptyState onRefresh={refreshDevice} onDemo={() => { setDemoMode(true); setDevice(MOCK_DEVICE); }} onOpenSettings={() => setView('settings')} />)}
    {view === 'settings' && <SettingsPage previewMode={pureWeb} onNavigateAbout={() => setView('about')} onThemeChange={setTheme} onRefreshIntervalChange={setRefreshIntervalSeconds} />}
    {view === 'about' && <AboutPage previewMode={pureWeb} onBack={() => setView('settings')} />}
    {appNotification && (
      <aside className={`app-notification ${appNotification.kind}`} role={appNotification.kind === 'error' ? 'alert' : 'status'} aria-live={appNotification.kind === 'error' ? 'assertive' : 'polite'}>
        <div><strong>{appNotification.title}</strong>{appNotification.body && <p>{appNotification.body}</p>}</div>
        <button type="button" onClick={() => setAppNotification(undefined)} aria-label="关闭通知"><X weight="bold" /></button>
      </aside>
    )}
  </div>;
}
