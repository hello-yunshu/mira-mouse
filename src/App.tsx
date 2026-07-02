// SPDX-License-Identifier: AGPL-3.0-or-later
import { useCallback, useEffect, useMemo, useRef, useState, useTransition } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  BatteryHigh,
  CaretDown,
  Gauge,
  Gear,
  Info,
  Lightbulb,
  Minus,
  ReadCvLogo,
  SignOut,
  Timer,
  UserCircle,
  WaveSine,
  X,
} from '@phosphor-icons/react';
import { useTranslation } from 'react-i18next';
import { MOCK_DEVICE, MOCK_DEVICE_ENTRIES } from './mock';
import { applyTheme, pastelDisplayColor } from './theme';
import i18n, { applyLanguage, loadPluginLocales, resolveLabelKey } from './i18n';
import { SettingsPage } from './Settings';
import { AboutPage } from './About';
import type { AboutInfo, AppSettings, DeviceBattery, DeviceCapabilities, DeviceSnapshot, DeviceSnapshotEntry, DeviceState, EffectOption, PluginCapability, PluginUpdateInfo, ReceiverLightingOptions, ThemeMode } from './types';
import {
  offValue as pluginOffValue,
  requiresExtraColor as pluginRequiresExtraColor,
  supportsAnyLighting as pluginSupportsAnyLighting,
  supportsLightingMutation as pluginSupportsLightingMutation,
  supportedEffectValues as pluginSupportedEffectValues,
  compatibilityCapabilities,
  effectDefaults,
  type PluginSummaryItem,
  MAX_CONTROL_GROUPS,
  MAX_STATUS_ITEMS,
  pickMutation,
  pluginMutations,
  pluginOptions,
  pluginRange,
  pluginSummaryItems,
  pluginValueFormat,
} from './pluginAdapter';
import type { RangeSpec } from './types';
import { onAppNotification, notifyError, notifyInfo, notifySuccess, type AppNotification } from './notify';
import { relaunchAfterUpdate, startAutomaticAppUpdateCheck } from './updater';
import './styles.css';

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
  return new URLSearchParams(window.location.search).get('platform') === 'windows';
}

function isPureWebPreview(): boolean {
  // 纯浏览器环境（非 Tauri 运行时），用于网页预览
  return !(typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window);
}

function WindowsPreviewControls() {
  const { t } = useTranslation();
  return (
    <div className="windows-preview-controls" aria-label={t('dashboard.windowsControls')}>
      <button type="button" aria-label={t('dashboard.minimizeWindow')}><Minus weight="regular" /></button>
      <button type="button" className="windows-close" aria-label={t('dashboard.closeWindow')}><X weight="regular" /></button>
    </div>
  );
}

function WindowsWindowControls() {
  const { t } = useTranslation();
  return (
    <div className="windows-window-controls" aria-label={t('dashboard.windowsControls')}>
      <button type="button" aria-label={t('dashboard.minimizeWindow')} onClick={() => getCurrentWindow().minimize()}><Minus weight="regular" /></button>
      <button type="button" className="windows-close" aria-label={t('dashboard.closeWindow')} onClick={() => invoke('hide_to_tray')}><X weight="regular" /></button>
    </div>
  );
}

function connectionDisplay(connection: string | undefined, t: (key: string) => string): string {
  switch (connection) {
    case 'usb': return t('connection.usb');
    case 'wireless': return t('connection.wireless');
    case 'bluetooth': return t('connection.bluetooth');
    case 'virtual': return t('connection.virtual');
    default: return t('connection.unknown');
  }
}

// 界面不硬编码品牌灯效名称。灯效名称由插件 parsers.json 的 derived.lookup 提供（effectName/optionName）。
// 当插件未提供名称时，界面只显示通用占位符，避免将品牌数据耦合到 UI 层。
function capabilityObject(capabilities: DeviceCapabilities | undefined, key: string): Record<string, unknown> | undefined {
  const value = capabilities?.[key];
  return value && typeof value === 'object' ? value : undefined;
}

function lightingCapability(capabilities: DeviceCapabilities | undefined, group: 'mouseLighting' | 'receiverLighting'): Record<string, unknown> | undefined {
  if (group === 'receiverLighting') return capabilityObject(capabilities, 'receiverLighting');
  return capabilityObject(capabilities, 'mouseLighting') ?? capabilityObject(capabilities, 'mouseEffect');
}

function confirmedMouseLightColor(snapshot: DeviceSnapshot, receiverLighting: Record<string, unknown> | undefined): string | undefined {
  if (!snapshot.confirmedLightColor) return undefined;
  if (typeof receiverLighting?.color === 'string') return undefined;
  return snapshot.confirmedLightColor;
}

function getLightingEffectName(
  capabilities?: DeviceCapabilities,
  group: 'mouseLighting' | 'receiverLighting' = 'mouseLighting',
  pluginCapabilities: PluginCapability[] = [],
  pluginId?: string,
): string {
  const lighting = lightingCapability(capabilities, group);
  if (!lighting) return i18n.t('lighting.hardwareSync');
  // 仅使用插件提供的 effectName（来自 parsers.json derived.lookup）
  if (typeof lighting.effectName === 'string' && lighting.effectName) return lighting.effectName;
  const effect = lighting.effect;
  if (typeof effect !== 'number') return i18n.t('lighting.hardwareSync');
  const lightingCap = pluginCapabilities.find((c) => c.control === 'LightingZone');
  if (group === 'receiverLighting') {
    const receiverOptions = lightingCap?.metadata.receiverLightingOptions as ReceiverLightingOptions | undefined;
    const option = receiverOptions?.effect?.find((candidate) => candidate.value === effect);
    if (option) {
      const label = resolveLabelKey(option.labelKey, pluginId);
      if (label !== option.labelKey) return label;
      if (effect === 0) return i18n.t('lighting.off');
    }
  }
  // 从插件 LightingZone capability 的 effectOptions.offValue 读取"关闭"值（替代硬编码 effect === 0）
  const off = lightingCap ? pluginOffValue(lightingCap) : undefined;
  if (off !== undefined && effect === off) return i18n.t('lighting.off');
  return i18n.t('lighting.effectN', { value: effect });
}

function getLightingColorMode(capabilities?: DeviceCapabilities, group: 'mouseLighting' | 'receiverLighting' = 'mouseLighting'): string {
  const lighting = lightingCapability(capabilities, group);
  if (!lighting) return i18n.t('common.notReported');
  // 仅使用插件提供的 optionName（来自 parsers.json derived.lookup）
  if (typeof lighting.optionName === 'string' && lighting.optionName) return lighting.optionName;
  const option = lighting.option;
  if (typeof option !== 'number') return i18n.t('common.notReported');
  return i18n.t('lighting.modeN', { value: option });
}

function rgbToHex(rgb: unknown): string | undefined {
  if (typeof rgb === 'string' && /^#[0-9a-f]{6}$/i.test(rgb)) return rgb;
  if (!Array.isArray(rgb) || rgb.length < 3) return undefined;
  const [r, g, b] = rgb.map((v) => Number(v));
  if ([r, g, b].some((v) => Number.isNaN(v))) return undefined;
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
  const mouseLighting = lightingCapability(caps, 'mouseLighting');
  const receiverLighting = lightingCapability(caps, 'receiverLighting');
  const mouseLightColorOutput = capabilityObject(caps, 'mouseLightColor');
  const settings = caps.settings;
  // 鼠标灯光状态只接受明确的鼠标侧来源，接收器侧灯光状态不能覆盖这里。
  const mouseLightSwitch = typeof caps.mouseLightSwitch === 'object' && caps.mouseLightSwitch !== null
    ? caps.mouseLightSwitch as Record<string, unknown>
    : undefined;
  const mouseLightEnabled = typeof settings?.mouseLightEnabled === 'boolean'
    ? settings.mouseLightEnabled
    : typeof mouseLightSwitch?.enabled === 'boolean'
      ? mouseLightSwitch.enabled
      : typeof mouseLighting?.enabled === 'boolean' ? mouseLighting.enabled : undefined;
  const mouseLightColor = (typeof mouseLighting?.color === 'string' ? mouseLighting.color : undefined)
    ?? rgbToHex(settings?.mouseLightStartColor)
    ?? rgbToHex(mouseLightColorOutput?.color)
    ?? confirmedMouseLightColor(snapshot, receiverLighting);
  const mouseLightEndColor = (typeof mouseLighting?.endColor === 'string' ? mouseLighting.endColor : undefined)
    ?? rgbToHex(settings?.mouseLightEndColor);
  // HID++ 扩展灯效参数：从 mouseLighting capability 读取 effect/speed/brightness/extraColor。
  // amaster 等旧插件不提供这些字段时为 undefined，UI 自动回退到简单颜色编辑。
  const mouseLightEffect = typeof mouseLighting?.effect === 'number' ? mouseLighting.effect : undefined;
  const mouseLightSpeed = typeof mouseLighting?.speed === 'number' ? mouseLighting.speed : undefined;
  const mouseLightBrightness = typeof mouseLighting?.brightness === 'number' ? mouseLighting.brightness : undefined;
  const mouseLightExtraColor = typeof mouseLighting?.extraColor === 'string' ? mouseLighting.extraColor : undefined;
  const fallbackBatteries: DeviceBattery[] = snapshot.batteryPercent === undefined ? [] : [{
    id: 'mouse', label: i18n.t('mock.mouseLabel'), percentage: snapshot.batteryPercent, charging: snapshot.charging,
  }];
  // Build lighting state only when at least one lighting field is reported.
  // Avoid defaulting to 'enabled' when the device never reported lighting.
  const hasLightingData = mouseLightColor !== undefined
    || mouseLightEndColor !== undefined
    || mouseLighting !== undefined
    || receiverLighting !== undefined;
  return {
    name: snapshot.displayName ?? i18n.t('common.unknownDevice'),
    connection: snapshot.connection,
    battery: snapshot.batteryPercent,
    charging: snapshot.charging,
    batteries: snapshot.batteries?.length ? snapshot.batteries : fallbackBatteries,
    pollingRate: snapshot.pollingRateHz,
    supportedPollingRates: snapshot.supportedPollingRatesHz ?? [],
    profile: snapshot.profile?.replace(/^Profile\s+/i, i18n.t('common.profilePrefix') + ' '),
    evidence: snapshot.evidence,
    readonly: snapshot.readonly ?? false,
    updatedAt: now,
    dpiStages: stages,
    lighting: hasLightingData
      ? {
          enabled: mouseLightEnabled !== false,
          mode: mouseLighting ? getLightingEffectName(caps, 'mouseLighting', snapshot.pluginCapabilities ?? [], snapshot.pluginId) : mouseLightEnabled === false ? i18n.t('lighting.off') : i18n.t('lighting.on'),
          color: mouseLightColor,
          supportsSpeed: typeof mouseLighting?.speed === 'number',
          supportsBrightness: typeof mouseLighting?.brightness === 'number',
          receiverLinked: snapshot.connection === 'wireless',
          mouseLightEnabled,
          mouseLightColor,
          mouseLightEndColor,
          mouseLightEffect,
          mouseLightSpeed,
          mouseLightBrightness,
          mouseLightExtraColor,
          receiverLightEnabled: typeof receiverLighting?.enabled === 'boolean' ? receiverLighting.enabled : undefined,
          receiverLightMode: receiverLighting ? getLightingEffectName(caps, 'receiverLighting', snapshot.pluginCapabilities ?? [], snapshot.pluginId) : undefined,
          receiverLightColor: typeof receiverLighting?.color === 'string' ? receiverLighting.color : undefined,
        }
      : undefined,
    capabilities: caps,
    pluginCapabilities: snapshot.pluginCapabilities ?? [],
    writableMutations: snapshot.writableMutations ?? [],
    pluginId: snapshot.pluginId,
  };
}

function selectedDeviceEntry(entries: DeviceSnapshotEntry[]): DeviceSnapshotEntry | undefined {
  return entries.find((entry) => entry.selected) ?? entries[0];
}

function entryToState(entry: DeviceSnapshotEntry | undefined): DeviceState | undefined {
  return entry ? snapshotToState(entry.snapshot) : undefined;
}

function DeviceAura({ color }: { color?: string }) {
  const [paused, setPaused] = useState(false);

  useEffect(() => {
    if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
    let unlisten: (() => void) | undefined;
    try {
      const win = getCurrentWindow();
      win.isVisible().then(v => setPaused(!v)).catch(() => {});
      win.onFocusChanged(({ payload: focused }) => {
        if (focused) {
          setPaused(false);
        } else {
          win.isVisible().then(v => setPaused(!v)).catch(() => {});
        }
      }).then((fn: () => void) => { unlisten = fn; }).catch(() => {});
    } catch {
      // 非 Tauri 环境忽略
    }
    return () => { if (unlisten) unlisten(); };
  }, []);

  return (
    <div className={`device-aura${paused ? ' is-paused' : ''}`} data-animation="realtime-deformation" style={{ '--device-color': color ?? '#b87ab0' } as React.CSSProperties} aria-hidden="true">
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
  const { t } = useTranslation();
  return (
    <main className="empty">
      <DeviceAura />
      <p className="eyebrow">{t('dashboard.eyebrow')}</p>
      <h1>{t('dashboard.noDevice')}</h1>
      <p>{t('dashboard.plugInHint')}</p>
      <div className="actions">
        <button onClick={onRefresh}>{t('common.refresh')}</button>
        <button className="secondary" onClick={onOpenSettings}>{t('dashboard.deviceAndDiagnostics')}</button>
      </div>
      <button className="demo" onClick={onDemo}>{t('dashboard.openFixture')}</button>
    </main>
  );
}

function capabilityGroupLabel(group: string): string {
  return i18n.t(`capability.group.${group}`, { defaultValue: group });
}

function capabilityFieldLabel(key: string): string {
  return i18n.t(`capability.field.${key}`, { defaultValue: key });
}

function capabilityValue(value: unknown, key: string): string {
  if (typeof value === 'boolean') return value ? i18n.t('common.on') : i18n.t('common.off');
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
  if (value === null || value === undefined || value === '') return i18n.t('common.notReported');
  if (typeof value === 'object') return JSON.stringify(value);
  if (key === 'connection' && typeof value === 'string') return connectionDisplay(value, i18n.t);
  return String(value);
}

function valueLooksColor(value: unknown): boolean {
  return typeof value === 'string' && /^#(?:[0-9a-f]{3}|[0-9a-f]{6})$/i.test(value.trim());
}

function shouldRenderColorValue(value: unknown, format?: string): boolean {
  return format === 'color' || valueLooksColor(value);
}

function displayColor(value: unknown): string | undefined {
  if (typeof value !== 'string') return undefined;
  const color = value.trim();
  return /^#(?:[0-9a-f]{3}|[0-9a-f]{6})$/i.test(color) ? color : undefined;
}

function colorValueStyle(value: unknown): React.CSSProperties | undefined {
  const color = displayColor(value);
  return color ? { '--value-color': color } as React.CSSProperties : undefined;
}

function ColorValue({ value, fallback, className }: { value: unknown; fallback?: string; className?: string }) {
  const label = typeof value === 'string' && value ? value : fallback ?? i18n.t('common.notReported');
  const style = colorValueStyle(value);
  const classes = [className, style ? 'color-value' : undefined].filter(Boolean).join(' ') || undefined;
  return <strong className={classes} style={style}>{label}</strong>;
}

function FormattedValue({ value, label, keyName, format, className }: {
  value: unknown;
  label?: string;
  keyName: string;
  format?: string;
  className?: string;
}) {
  const resolvedFormat = pluginValueFormat(format);
  const text = label ?? (resolvedFormat === 'sleep' ? formatSleepTime(value) : capabilityValue(value, keyName));
  return shouldRenderColorValue(value, resolvedFormat)
    ? <ColorValue className={className} value={value} fallback={text} />
    : <strong className={className}>{text}</strong>;
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
  if (!Number.isFinite(seconds) || seconds <= 0) return i18n.t('common.notReported');
  if (seconds % 60 === 0) return i18n.t('common.minute', { count: seconds / 60 });
  return i18n.t('common.second', { count: seconds });
}

function pluginLabel(capability: PluginCapability, pluginId?: string): string {
  const key = `plugin.label.${capability.labelKey}`;
  const localized = resolveLabelKey(key, pluginId);
  if (localized !== key) return localized;
  return typeof capability.metadata.label === 'string' ? capability.metadata.label : capability.labelKey;
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

function pluginSummaryValue(item: PluginSummaryItem, value: unknown): string {
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
      aria-label={i18n.t('dashboard.deviceSummary')}
      style={{ gridTemplateColumns: `repeat(${items.length}, minmax(0, 1fr))` }}
    >
      {items.map((item) => (
        <span key={`${item.label}:${item.source}`}>
          {i18n.t(item.label, { defaultValue: item.label })}
          {(() => {
            const value = readPath(device, item.source);
            const label = pluginSummaryValue(item, value);
            return <FormattedValue value={value} label={label} keyName={item.source.split('.').at(-1) ?? 'value'} format={item.format} />;
          })()}
        </span>
      ))}
    </div>
  );
}

function pluginValueLabel(capability: PluginCapability, value: unknown): string {
  const option = pluginOptions(capability).find((candidate) => candidate.value === value);
  if (option) return option.label;
  if (pluginValueFormat(capability.metadata.format) === 'sleep') return formatSleepTime(value);
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
      label: typeof binding.label === 'string' ? i18n.t(binding.label, { defaultValue: binding.label }) : pluginLabel(capability, device.pluginId),
      value: readPath(device, binding.source),
      mutation: pickMutation(binding.mutation, device.writableMutations),
      param: typeof binding.param === 'string' ? binding.param : 'value',
    };
  }
  return {
    label: pluginLabel(capability, device.pluginId),
    value: readPath(device, capability.metadata.source),
    mutation: pluginMutations(capability, device.writableMutations).default
      ?? pickMutation(capability.metadata.mutation, device.writableMutations),
    param: typeof capability.metadata.param === 'string' ? capability.metadata.param : 'value',
  };
}

function capabilityVisible(capability: PluginCapability, device: DeviceState): boolean {
  const binding = capabilityBinding(capability, device);
  const mutations = Object.values(pluginMutations(capability, device.writableMutations));
  if (binding.value !== undefined) return true;
  if (binding.mutation && device.writableMutations.includes(binding.mutation)) {
    if (capability.control === 'Select' || capability.control === 'Segmented' || capability.control === 'Toggle' || capability.control === 'Action') return true;
    return false;
  }
  if (capability.control === 'Number' || capability.control === 'Slider' || capability.control === 'Color') return false;
  if (mutations.some((mutation) => device.writableMutations.includes(mutation))) return true;
  if (capability.control === 'DpiStages') return device.dpiStages.length > 0;
  if (capability.control === 'LightingZone') return device.lighting !== undefined;
  if (!capability.readOnly && Object.values(pluginMutations(capability, device.writableMutations)).some((mutation) => device.writableMutations.includes(mutation))) return true;
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
  const mutation = pluginMutations(capability, device.writableMutations).default;
  const param = typeof capability.metadata.param === 'string' ? capability.metadata.param : 'value';
  const options = pluginOptions(capability, device);
  const valueFormat = pluginValueFormat(capability.metadata.format);
  const range = pluginRange(capability);
  const [draft, setDraft] = useState<string | number>(() => typeof current === 'number' || typeof current === 'string' ? current : '');
  const [editingPollingRate, setEditingPollingRate] = useState(false);
  const [editingValue, setEditingValue] = useState(false);

  const writable = Boolean(mutation && !capability.readOnly && device.writableMutations.includes(mutation));
  const apply = (value: unknown) => mutation ? runMutation(mutation, { [param]: value }) : Promise.resolve();
  const openValueEditor = () => {
    if (capability.control === 'Select' && current === undefined && options[0]) {
      setDraft(String(options[0].value));
    } else {
      setDraft(typeof current === 'number' || typeof current === 'string' ? current : '');
    }
    setEditingValue(true);
  };
  const valueLabel = pluginValueLabel(capability, current);
  if (capability.labelKey === 'capability.polling-rate') {
    return (
      <div className="control-reading mode-reading polling-reading">
        <WaveSine weight="regular" />
        <span>{i18n.t('dashboard.currentPollingRate')}</span>
        <button
          type="button"
          className="polling-rate editable-reading"
          aria-label={typeof current === 'number'
            ? i18n.t('dashboard.currentPollingRateEdit', { value: current })
            : i18n.t('dashboard.pollingRateNotReportedEdit')}
          disabled={writeBusy || !writable || options.length === 0}
          onClick={() => setEditingPollingRate(true)}
        >
          <strong>{typeof current === 'number' ? current : i18n.t('common.notReported')}</strong>
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
        {!writable && <p className="setting-hint">{i18n.t('dashboard.pollingReadonlyHint')}</p>}
      </div>
    );
  }

  return (
    <div className="control-reading mode-reading plugin-control-reading">
      <UserCircle weight="regular" />
      <span>{pluginLabel(capability, device.pluginId)}</span>
      {capability.control === 'Segmented' && options.length > 0 && (
        <div
          className="plugin-segmented"
          role="group"
          aria-label={pluginLabel(capability, device.pluginId)}
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
      {capability.control === 'Toggle' && (
        <button
          type="button"
          className={`plugin-toggle ${current === true ? 'active' : ''}`}
          aria-pressed={current === true}
          disabled={writeBusy || !writable}
          onClick={() => void apply(current !== true)}
        >{current === true ? i18n.t('common.on') : i18n.t('common.off')}</button>
      )}
      {(['Select', 'Number', 'Slider', 'Color'] as PluginCapability['control'][]).includes(capability.control) && (
        <button
          type="button"
          className="plugin-value-button editable-reading"
          aria-label={`${pluginLabel(capability, device.pluginId)}：${valueLabel}，点击编辑`}
          disabled={writeBusy || !writable || (capability.control === 'Select' && options.length === 0)}
          onClick={openValueEditor}
        >
          {capability.control === 'Color' && typeof current === 'string' && <i style={{ '--light-color': current } as React.CSSProperties} />}
          <FormattedValue value={current} label={valueLabel} keyName={capability.id} format={capability.control === 'Color' ? 'color' : valueFormat} />
        </button>
      )}
      {capability.control === 'Action' && (
        <button
          type="button"
          className="plugin-action"
          disabled={writeBusy || !writable}
          onClick={() => void runMutation(mutation, (capability.metadata.params as Record<string, unknown>) ?? {})}
        >{typeof capability.metadata.actionLabel === 'string' ? capability.metadata.actionLabel : i18n.t('common.execute')}</button>
      )}
      {(capability.readOnly || capability.control === 'ReadOnlyValue') && (
        <FormattedValue
          className="plugin-current-value"
          value={current}
          label={pluginValueLabel(capability, current)}
          keyName={capability.id}
          format={valueFormat}
        />
      )}
      <PluginSummary capability={capability} device={device} />
      {!writable && !capability.readOnly && <p className="setting-hint">{i18n.t('dashboard.writeUnavailableHint')}</p>}
      {editingValue && (
        <EditModal
          title={pluginLabel(capability, device.pluginId)}
          submitDisabled={writeBusy || !writable}
          onClose={() => setEditingValue(false)}
          onSubmit={() => {
            if (capability.control === 'Select') {
              const option = options.find((candidate) => String(candidate.value) === String(draft));
              if (option) void apply(option.value);
            } else {
              void apply(draft);
            }
            setEditingValue(false);
          }}
        >
          <label className="edit-field">
            <span>{pluginLabel(capability, device.pluginId)}</span>
            {capability.control === 'Select' ? (
              <select
                autoFocus
                aria-label={pluginLabel(capability, device.pluginId)}
                value={String(draft)}
                disabled={writeBusy}
                onChange={(event) => setDraft(event.target.value)}
              >
                {options.map((option) => (
                  <option key={String(option.value)} value={String(option.value)}>{option.label}</option>
                ))}
              </select>
            ) : (
              <input
                autoFocus
                aria-label={pluginLabel(capability, device.pluginId)}
                type={capability.control === 'Color' ? 'color' : capability.control === 'Slider' ? 'range' : 'number'}
                value={draft}
                min={range?.min}
                max={range?.max}
                step={range?.step}
                disabled={writeBusy}
                onChange={(event) => setDraft(capability.control === 'Color' ? event.target.value : Number(event.target.value))}
              />
            )}
          </label>
        </EditModal>
      )}
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
          <div><p className="eyebrow">{i18n.t('dashboard.readonlyReport')}</p><h2 id="device-details-title">{i18n.t('dashboard.allReadInfo')}</h2></div>
          <button className="icon-button" onClick={onClose} aria-label={i18n.t('dashboard.closeDeviceDetails')}><X weight="regular" /></button>
        </header>
        <p className="details-note">{i18n.t('dashboard.detailsNote')}</p>
        <div className="capability-groups">
          {groups.length ? groups.map(([group, fields]) => (
            <section className="capability-group" key={group}>
              <h3>{capabilityGroupLabel(group)}</h3>
              <dl>
                {Object.entries(fields).map(([key, value]) => (
                  <div key={key}>
                    <dt>{capabilityFieldLabel(key)}</dt>
                    <dd><FormattedValue value={value} keyName={key} /></dd>
                  </div>
                ))}
              </dl>
            </section>
          )) : <p className="setting-hint">{i18n.t('dashboard.noCapabilities')}</p>}
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

function EditModal({ title, children, submitLabel = i18n.t('common.apply'), submitDisabled, onClose, onSubmit }: EditModalProps) {
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
          <button type="button" className="secondary" onClick={onClose}>{i18n.t('common.cancel')}</button>
          <button type="submit" disabled={submitDisabled}>{submitLabel}</button>
        </footer>
      </form>
    </div>
  );
}

interface DpiEditModalProps {
  stage: number;
  currentValue: number;
  range?: RangeSpec;
  writeBusy: boolean;
  onClose: () => void;
  onApply: (value: number) => void;
}

function DpiEditModal({ stage, currentValue, range, writeBusy, onClose, onApply }: DpiEditModalProps) {
  const [draft, setDraft] = useState(currentValue);
  const outOfRange = range ? draft < range.min || draft > range.max : false;
  const stepMismatch = range?.step
    ? Math.abs((draft - range.min) / range.step - Math.round((draft - range.min) / range.step)) > Number.EPSILON
    : false;
  return (
    <EditModal
      title={i18n.t('dashboard.editStageDpi', { stage })}
      submitDisabled={writeBusy || outOfRange || stepMismatch || draft === currentValue}
      onClose={onClose}
      onSubmit={() => onApply(draft)}
    >
      <label className="edit-field">
        <span>{i18n.t('dashboard.dpiValue')}</span>
        <input
          type="number"
          min={range?.min}
          max={range?.max}
          step={range?.step}
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
  options: { value: string | number | boolean; label: string }[];
  writeBusy: boolean;
  onClose: () => void;
  onApply: (value: string | number | boolean) => void;
}) {
  const [draft, setDraft] = useState(String(currentValue ?? options[0]?.value ?? ''));
  const selected = options.find((option) => String(option.value) === draft);
  return (
    <EditModal
      title={i18n.t('dashboard.setPollingRateTitle')}
      submitDisabled={writeBusy || !selected || selected.value === currentValue}
      onClose={onClose}
      onSubmit={() => selected && onApply(selected.value)}
    >
      <label className="edit-field">
        <span>{i18n.t('plugin.label.capability.polling-rate')}</span>
        <select
          autoFocus
          aria-label={i18n.t('plugin.label.capability.polling-rate')}
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
        <span>{i18n.t('common.color')}</span>
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

function SleepEditModal({ label, currentSeconds, range, writeBusy, onClose, onApply }: {
  label: string;
  currentSeconds: number;
  range?: RangeSpec;
  writeBusy: boolean;
  onClose: () => void;
  onApply: (seconds: number) => void;
}) {
  const [draft, setDraft] = useState(currentSeconds);
  const outOfRange = range ? draft < range.min || draft > range.max : false;
  return (
    <EditModal
      title={i18n.t('dashboard.setSleepTitle', { label })}
      submitDisabled={writeBusy || outOfRange || draft === currentSeconds}
      onClose={onClose}
      onSubmit={() => onApply(draft)}
    >
      <label className="edit-field">
        <span>{i18n.t('dashboard.timeoutSeconds')}</span>
        <input
          type="number"
          min={range?.min}
          max={range?.max}
          step={range?.step}
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

interface ReceiverLightingEditModalProps {
  field: ReceiverLightingField;
  pluginId?: string;
  initial: {
    effect: number;
    speed: number;
    brightness: number;
    option: number;
    color: string;
  };
  options?: ReceiverLightingOptions;
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

function receiverLightingFieldLabel(field: ReceiverLightingField): string {
  return i18n.t(`receiverLighting.field.${field}`, { defaultValue: field });
}

function ReceiverLightingEditModal({ field, pluginId, initial, options, writeBusy, onClose, onApply }: ReceiverLightingEditModalProps) {
  const [draft, setDraft] = useState(initial);
  const submitDisabled = useMemo(
    () => writeBusy || Object.keys(draft).every((k) => draft[k as keyof typeof draft] === initial[k as keyof typeof initial]),
    [writeBusy, draft, initial],
  );
  return (
    <EditModal
      title={i18n.t('dashboard.editReceiverLightingTitle', { field: receiverLightingFieldLabel(field) })}
      submitDisabled={submitDisabled}
      onClose={onClose}
      onSubmit={() => onApply(draft)}
    >
      {field === 'effect' && <label className="edit-field">
        <span>{i18n.t('receiverLighting.field.effect')}</span>
        <select
          value={draft.effect}
          disabled={writeBusy}
          onChange={(event) => setDraft({ ...draft, effect: Number(event.target.value) })}
        >
          {(options?.effect ?? []).map((opt) => (
            <option key={opt.value} value={opt.value}>{resolveLabelKey(opt.labelKey, pluginId)}</option>
          ))}
        </select>
      </label>}
      {field === 'speed' && <label className="edit-field">
        <span>{i18n.t('receiverLighting.field.speed')}</span>
        <select
          value={draft.speed}
          disabled={writeBusy}
          onChange={(event) => setDraft({ ...draft, speed: Number(event.target.value) })}
        >
          {(options?.speed ?? []).map((opt) => (
            <option key={opt.value} value={opt.value}>{resolveLabelKey(opt.labelKey, pluginId)}</option>
          ))}
        </select>
      </label>}
      {field === 'brightness' && <label className="edit-field">
        <span>{i18n.t('receiverLighting.field.brightness')}</span>
        <select
          value={draft.brightness}
          disabled={writeBusy}
          onChange={(event) => setDraft({ ...draft, brightness: Number(event.target.value) })}
        >
          {(options?.brightness ?? []).map((opt) => (
            <option key={opt.value} value={opt.value}>{resolveLabelKey(opt.labelKey, pluginId)}</option>
          ))}
        </select>
      </label>}
      {field === 'option' && <label className="edit-field">
        <span>{i18n.t('receiverLighting.field.option')}</span>
        <select
          value={draft.option}
          disabled={writeBusy}
          onChange={(event) => setDraft({ ...draft, option: Number(event.target.value) })}
        >
          {(options?.option ?? []).map((opt) => (
            <option key={opt.value} value={opt.value}>{resolveLabelKey(opt.labelKey, pluginId)}</option>
          ))}
        </select>
      </label>}
      {field === 'color' && <label className="edit-field color-field">
        <span>{i18n.t('common.color')}</span>
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

type MouseLightingField = 'effect' | 'speed' | 'brightness' | 'color' | 'extraColor';

interface MouseLightingEditModalProps {
  field: MouseLightingField;
  pluginId?: string;
  initial: {
    effect: number;
    speed: number;
    brightness: number;
    color: string;
    extraColor?: string;
  };
  effectOptions: EffectOption[];
  /** "关闭"值（来自插件 effectOptions.offValue），用于判断 enabled 状态。 */
  offValue?: number;
  supportedEffects: number[];
  speedRange: { min: number; max: number; step: number };
  brightnessRange: { min: number; max: number; step: number };
  writeBusy: boolean;
  onClose: () => void;
  onApply: (params: {
    color: string;
    enabled: boolean;
    effect: number;
    speed: number;
    brightness: number;
    extraColor?: string;
  }) => void;
}

function mouseLightingFieldLabel(field: MouseLightingField): string {
  if (field === 'extraColor') return i18n.t('lighting.extraColor');
  return i18n.t(`receiverLighting.field.${field}`, { defaultValue: field });
}

function mouseLightingOnState(device: DeviceState, capability?: PluginCapability): boolean | undefined {
  // 优先基于 effect 字段判断灯效状态：effect === offValue 视为关闭，
  // 否则视为开启。这避免了 mutation 只改 effect 字段而不改设备
  // rgbControl.enabled 时，UI 仍基于 mouseLightEnabled=false 显示"关闭"
  // 但灯实际已亮起的歧义。
  const effect = device.lighting?.mouseLightEffect;
  if (typeof effect === 'number') {
    const effectOptions = capability?.metadata?.effectOptions as { offValue?: number } | undefined;
    return effect !== (effectOptions?.offValue ?? 0);
  }
  if (device.lighting?.mouseLightEnabled === false) return false;
  return typeof device.lighting?.mouseLightEnabled === 'boolean'
    ? device.lighting.mouseLightEnabled
    : undefined;
}

function MouseLightingEditModal({ field, pluginId, initial, effectOptions, offValue, supportedEffects, speedRange, brightnessRange, writeBusy, onClose, onApply }: MouseLightingEditModalProps) {
  const [draft, setDraft] = useState(initial);
  const submitDisabled = useMemo(
    () => writeBusy || Object.keys(draft).every((k) => draft[k as keyof typeof draft] === initial[k as keyof typeof initial]),
    [writeBusy, draft, initial],
  );
  const visibleEffects = effectOptions.filter((opt) => supportedEffects.includes(opt.value));
  const isOff = (effect: number) => offValue !== undefined ? effect === offValue : effect === 0;
  return (
    <EditModal
      title={i18n.t('lighting.editMouseLightingTitle', { field: mouseLightingFieldLabel(field) })}
      submitDisabled={submitDisabled}
      onClose={onClose}
      onSubmit={() => onApply({
        color: draft.color,
        enabled: !isOff(draft.effect),
        effect: draft.effect,
        speed: draft.speed,
        brightness: draft.brightness,
        extraColor: draft.extraColor ?? '#000000',
      })}
    >
      {field === 'effect' && <label className="edit-field">
        <span>{i18n.t('receiverLighting.field.effect')}</span>
        <select
          value={draft.effect}
          disabled={writeBusy}
          onChange={(event) => setDraft({ ...draft, effect: Number(event.target.value) })}
        >
          {visibleEffects.map((opt) => (
            <option key={opt.value} value={opt.value}>{resolveLabelKey(opt.labelKey, pluginId)}</option>
          ))}
        </select>
      </label>}
      {field === 'speed' && <label className="edit-field range-field">
        <span>{i18n.t('receiverLighting.field.speed')}</span>
        <input
          type="range"
          min={speedRange.min}
          max={speedRange.max}
          step={speedRange.step}
          value={draft.speed}
          disabled={writeBusy}
          onChange={(event) => setDraft({ ...draft, speed: Number(event.target.value) })}
        />
        <span className="range-value">{draft.speed}</span>
      </label>}
      {field === 'brightness' && <label className="edit-field range-field">
        <span>{i18n.t('receiverLighting.field.brightness')}</span>
        <input
          type="range"
          min={brightnessRange.min}
          max={brightnessRange.max}
          step={brightnessRange.step}
          value={draft.brightness}
          disabled={writeBusy}
          onChange={(event) => setDraft({ ...draft, brightness: Number(event.target.value) })}
        />
        <span className="range-value">{draft.brightness}%</span>
      </label>}
      {field === 'color' && <label className="edit-field color-field">
        <span>{i18n.t('common.color')}</span>
        <input
          type="color"
          value={draft.color}
          disabled={writeBusy}
          onChange={(event) => setDraft({ ...draft, color: event.target.value })}
        />
      </label>}
      {field === 'extraColor' && <label className="edit-field color-field">
        <span>{i18n.t('lighting.extraColor')}</span>
        <input
          type="color"
          value={draft.extraColor ?? '#000000'}
          disabled={writeBusy}
          onChange={(event) => setDraft({ ...draft, extraColor: event.target.value })}
        />
      </label>}
    </EditModal>
  );
}

function Dashboard({
  device,
  deviceEntries,
  onDeviceChange,
  onDeviceSelect,
  pluginLocaleRevision,
}: {
  device: DeviceState;
  deviceEntries: DeviceSnapshotEntry[];
  onDeviceChange: (device: DeviceState) => void;
  onDeviceSelect: (deviceKey: string) => void;
  pluginLocaleRevision: number;
}) {
  const { t } = useTranslation();
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
  const [showDeviceSwitcher, setShowDeviceSwitcher] = useState(false);
  const batteryControlRef = useRef<HTMLDivElement>(null);
  const deviceSwitcherRef = useRef<HTMLDivElement>(null);
  const [writeBusy, setWriteBusy] = useState(false);
  const [editingDpiStage, setEditingDpiStage] = useState<number | null>(null);
  const [editingMouseLightColor, setEditingMouseLightColor] = useState(false);
  const [editingMouseLightEndColor, setEditingMouseLightEndColor] = useState(false);
  const [editingMouseLightField, setEditingMouseLightField] = useState<MouseLightingField | null>(null);
  const [editingReceiverLighting, setEditingReceiverLighting] = useState<ReceiverLightingField | null>(null);
  const activeDpi = initialDpi;
  const writable = useCallback(
    (mutation: string) => device.writableMutations.includes(mutation),
    [device.writableMutations],
  );

  useEffect(() => {
    const closeOnOutsideClick = (event: MouseEvent) => {
      if (!batteryControlRef.current?.contains(event.target as Node)) setShowBatteries(false);
      if (!deviceSwitcherRef.current?.contains(event.target as Node)) setShowDeviceSwitcher(false);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setShowBatteries(false);
        setShowDeviceSwitcher(false);
      }
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
    setPreviewMessage(i18n.t('dashboard.writing'));
    try {
      const snapshot = await invoke<DeviceSnapshot>('device_mutate', { mutation, params });
      onDeviceChange(snapshotToState(snapshot));
      setPreviewMessage('');
      notifySuccess(i18n.t('dashboard.writeConfirmed'));
    } catch (error) {
      setPreviewMessage('');
      const errorString = String(error);
      // 设备层面拒绝（featureIndex=0 等）：通常是官方配置工具独占 HID 设备，
      // 导致 Mira 查询 feature 时收到 0 响应。跨平台提示用户关闭官方软件。
      if (errorString.includes('is not available on this device')) {
        notifyError(
          i18n.t('notification.mutationUnavailable'),
          i18n.t('notification.mutationUnavailableBody'),
        );
      } else {
        notifyError(i18n.t('notification.writeFailed'), i18n.t('notification.writeFailedBody', { error: errorString }));
      }
      // 写入失败后设备的实际可用 mutation 集合可能与界面缓存的 writableMutations
      // 不一致（例如 read_device_once 读取时设备处于板载模式，但调用写入时设备
      // 已切换到 host 模式，set-mouse-lighting-onboard 被守门拒绝）。这里主动
      // 触发一次设备刷新，让界面拿到最新的 writableMutations，避免下次还选到
      // 不支持的 mutation。
      invoke('device_refresh').catch(() => {});
    } finally {
      setWriteBusy(false);
    }
  };

  const currentStage = Math.max(1, stages.findIndex((stage) => stage.active) + 1);
  const selectedEntry = selectedDeviceEntry(deviceEntries);
  const multipleDevices = deviceEntries.length > 1;
  const receiverLighting = lightingCapability(device.capabilities, 'receiverLighting');
  const [sleepSetting, setSleepSetting] = useState<{ label: string; seconds: number; mutation: string; param: string; range?: RangeSpec }>();
  const [editingSleep, setEditingSleep] = useState(false);
  const pluginDescriptors = useMemo(() => compatibilityCapabilities(device), [device]);
  const {
    controls,
    activeMode,
    activeDpiDescriptor,
    activeLightingDescriptor,
    activeGenericDescriptors,
    selectDpiMutation,
    setDpiMutation,
    mouseLightingMutation,
    receiverLightingMutation,
    supportsReceiverLighting,
    activeLightingView,
    mouseEffectOptions,
    hasMouseEffectOptions,
    mouseOffEffect,
    mouseOnEffect,
    mouseSupportedEffects,
    mouseLightOn,
    mouseEffectDefaults,
    mouseHasEndColor,
    mouseNeedsExtraColor,
    mouseLightingRowCount,
  } = useMemo(() => {
    const controlPlacements = pluginDescriptors
      .flatMap((capability) => placementsFor(capability, 'control').map((placement) => ({ capability, placement })))
      .filter(({ capability }) => capabilityVisible(capability, device))
      .sort((a, b) => a.placement.order - b.placement.order);
    const controlGroups = new Map<string, { id: string; label: string; icon: PluginIcon; capabilities: PluginCapability[] }>();
    for (const { capability, placement } of controlPlacements) {
      const id = placement.group || capability.id;
      const existing = controlGroups.get(id);
      if (existing) existing.capabilities.push(capability);
      else controlGroups.set(id, { id, label: pluginLabel(capability, device.pluginId), icon: pluginIcon(placement.icon), capabilities: [capability] });
    }
    const controls = [...controlGroups.values()].slice(0, MAX_CONTROL_GROUPS);
    const activeMode = controls.some((control) => control.id === mode) ? mode : controls[0]?.id;
    const activeDescriptors = activeMode ? controlGroups.get(activeMode)?.capabilities ?? [] : [];
    const activeDpiDescriptor = activeDescriptors.find((capability) => capability.control === 'DpiStages');
    const activeLightingDescriptor = activeDescriptors.find((capability) => capability.control === 'LightingZone');
    const activeGenericDescriptors = activeDescriptors.filter((capability) => !['DpiStages', 'LightingZone'].includes(capability.control));
    const dpiMutations = activeDpiDescriptor ? pluginMutations(activeDpiDescriptor, device.writableMutations) : {};
    const selectDpiMutation = dpiMutations.select;
    const setDpiMutation = dpiMutations.value;
    const lightingMutations = activeLightingDescriptor ? pluginMutations(activeLightingDescriptor, device.writableMutations) : {};
    const mouseLightingMutation = lightingMutations.mouse;
    const receiverLightingMutation = lightingMutations.receiver;
    const supportsReceiverLighting = Boolean(receiverLightingMutation && writable(receiverLightingMutation))
      || Boolean(device.lighting?.receiverLightColor)
      || Boolean(device.capabilities.receiverLighting);
    const activeLightingView = lightingView === 'receiver' && !supportsReceiverLighting ? 'mouse' : lightingView;
    // HID++ 灯效扩展：从插件 metadata 读取 effectOptions，从设备能力读取实际支持的灯效。
    // amaster 等旧插件无 effectOptions 时 hasMouseEffectOptions=false，UI 回退到简单颜色编辑。
    const mouseEffectOptions = activeLightingDescriptor?.metadata?.effectOptions as {
      offValue?: number;
      effect?: EffectOption[];
      speed?: { min: number; max: number; step: number };
      brightness?: { min: number; max: number; step: number };
    } | undefined;
    const hasMouseEffectOptions = Boolean(mouseEffectOptions?.effect?.length);
    const mouseOffEffect = mouseEffectOptions?.offValue ?? 0;
    const mouseOnEffect = mouseEffectOptions?.effect?.find((effect) => effect.value !== mouseOffEffect)?.value ?? 1;
    // 设备支持的灯效列表：优先从 device.capabilities.mouseLighting.supportedEffects 读取
    // （由插件 protocol 解析提供），未提供时回退到 effectOptions.effect 全集。
    const mouseSupportedEffects = activeLightingDescriptor
      ? pluginSupportedEffectValues(activeLightingDescriptor, device.capabilities)
      : [];
    const mouseLightOn = mouseLightingOnState(device, activeLightingDescriptor);
    // 灯效写入默认值：从插件 effectOptions 提取（effect/speed/brightness），
    // 替代 UI 硬编码 effect=1 / speed=0 / brightness=100。
    const mouseEffectDefaults = effectDefaults(activeLightingDescriptor);
    const mouseHasEndColor = Boolean(
      device.lighting?.mouseLightEndColor
        && device.lighting.mouseLightEndColor !== device.lighting.mouseLightColor,
    );
    const mouseNeedsExtraColor = Boolean(
      activeLightingDescriptor
        && device.lighting?.mouseLightEffect !== undefined
        && pluginRequiresExtraColor(activeLightingDescriptor, device.lighting.mouseLightEffect),
    );
    const mouseLightingRowCount = hasMouseEffectOptions
      ? 3
        + (device.lighting?.supportsSpeed ? 1 : 0)
        + (device.lighting?.supportsBrightness ? 1 : 0)
        + (mouseNeedsExtraColor ? 1 : 0)
      : 2 + (mouseHasEndColor ? 1 : 0);
    return {
      controls, activeMode, activeDpiDescriptor, activeLightingDescriptor, activeGenericDescriptors,
      selectDpiMutation, setDpiMutation, mouseLightingMutation, receiverLightingMutation,
      supportsReceiverLighting, activeLightingView, mouseEffectOptions, hasMouseEffectOptions,
      mouseOffEffect, mouseOnEffect, mouseSupportedEffects, mouseLightOn, mouseEffectDefaults,
      mouseHasEndColor, mouseNeedsExtraColor, mouseLightingRowCount,
    };
    // device/pluginDescriptors 驱动 capability 查询；mode/lightingView 影响 activeMode 与 activeLightingView；
    // writable 闭包依赖 device.writableMutations。pluginLocaleRevision 在插件 locale 异步加载后刷新
    // pluginLabel 标签（隐式读取 i18n store，linter 无法静态检测），故显式禁用本行检查。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [device, pluginDescriptors, mode, lightingView, writable, pluginLocaleRevision]);
  // 跟踪"上一次非 OFF 的灯效"，用于点击"状态"按钮开启时恢复之前的灯效，
  // 而不是始终回退到默认 mouseOnEffect（通常是 FIXED/常亮）。
  const lastNonOffEffectRef = useRef<number>(mouseOnEffect);
  useEffect(() => {
    const effect = device.lighting?.mouseLightEffect;
    const supportedEffects = activeLightingDescriptor
      ? pluginSupportedEffectValues(activeLightingDescriptor, device.capabilities)
      : [];
    if (typeof effect === 'number' && effect !== mouseOffEffect && supportedEffects.includes(effect)) {
      lastNonOffEffectRef.current = effect;
    }
  }, [activeLightingDescriptor, device.capabilities, device.lighting?.mouseLightEffect, mouseOffEffect]);
  const statusPlacements = useMemo(() => pluginDescriptors
    .flatMap((capability) => placementsFor(capability, 'status').map((placement) => ({ capability, placement })))
    .filter(({ capability }) => capabilityVisible(capability, device))
    .sort((a, b) => a.placement.order - b.placement.order)
    .slice(0, MAX_STATUS_ITEMS), [device, pluginDescriptors]);
  const statusItems = useMemo(() => {
    const items: {
      id: string;
      label: string;
      value: string;
      icon: typeof Gauge;
      disabled?: boolean;
      color?: string;
      onClick?: () => void;
    }[] = [];
    for (const { capability, placement } of statusPlacements) {
      const binding = capabilityBinding(capability, device);
      const controlPlacement = placementsFor(capability, 'control')[0];
      if (pluginValueFormat(capability.metadata.format) === 'sleep' || (capability.control === 'Number' && Array.isArray(capability.metadata.bindings))) {
        const seconds = Number(binding.value);
        if (!Number.isFinite(seconds) || !binding.mutation) continue;
        const sleepRange = pluginRange(capability);
        items.push({
          id: capability.id, label: binding.label, value: formatSleepTime(seconds), icon: pluginIcon(placement.icon),
          disabled: !writable(binding.mutation),
          onClick: () => {
            setSleepSetting({ label: binding.label, seconds, mutation: binding.mutation!, param: binding.param, range: sleepRange });
            setEditingSleep(true);
          },
        });
      } else if (capability.control === 'LightingZone') {
        const mutation = pluginMutations(capability, device.writableMutations).mouse;
        const statusMouseLightOn = mouseLightingOnState(device, capability);
        items.push({
          id: capability.id, label: binding.label,
          value: typeof statusMouseLightOn === 'boolean'
            ? (statusMouseLightOn ? i18n.t('lighting.on') : i18n.t('lighting.off'))
            : device.lighting?.mouseLightColor ?? i18n.t('common.notReported'),
          icon: pluginIcon(placement.icon), color: device.lighting?.mouseLightColor,
          disabled: !mutation || !writable(mutation), onClick: () => setEditingMouseLightColor(true),
        });
      } else if (binding.value !== undefined) {
        const target = controlPlacement?.group || (controlPlacement ? capability.id : undefined);
        items.push({
          id: capability.id, label: binding.label, value: pluginValueLabel(capability, binding.value),
          icon: pluginIcon(placement.icon),
          disabled: !target, onClick: target ? () => setMode(target) : undefined,
        });
      }
    }
    return items;
    // statusPlacements 派生自 device/pluginDescriptors；writable 闭包依赖 device.writableMutations。
    // pluginLocaleRevision 在插件 locale 异步加载后刷新 binding.label（隐式读取 i18n store，linter
    // 无法静态检测），故显式禁用本行检查。setState setter 稳定，无需列入。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [device, statusPlacements, writable, pluginLocaleRevision]);

  return (
    <main className="dashboard">
      <section className="device-hero" aria-label={t('dashboard.connectedDevice')}>
        <div className="device-column">
          <h2 className="app-title">Mira</h2>
          <div className="device-copy">
            <p className="connection-state"><span />{connectionDisplay(device.connection, t)} · {t('common.connected')}</p>
            {multipleDevices ? (
              <div ref={deviceSwitcherRef} className={`device-switcher ${showDeviceSwitcher ? 'open' : ''}`}>
                <h1>
                  <button
                    type="button"
                    className="device-name-switch"
                    aria-expanded={showDeviceSwitcher}
                    aria-controls="device-switcher-list"
                    aria-label={t('dashboard.switchDevice')}
                    onClick={() => setShowDeviceSwitcher((visible) => !visible)}
                  >
                    <span>{device.name}</span>
                    <span className="device-switch-icon" aria-hidden="true">
                      <CaretDown weight="bold" />
                    </span>
                  </button>
                </h1>
                <section id="device-switcher-list" className="device-switcher-popover" aria-label={t('dashboard.switchDevice')}>
                  {showDeviceSwitcher && deviceEntries.map((entry) => {
                    const state = snapshotToState(entry.snapshot);
                    const selected = selectedEntry?.deviceKey === entry.deviceKey;
                    return (
                      <button
                        key={entry.deviceKey}
                        type="button"
                        className={selected ? 'active' : ''}
                        aria-pressed={selected}
                        onClick={() => {
                          setShowDeviceSwitcher(false);
                          if (!selected) onDeviceSelect(entry.deviceKey);
                        }}
                      >
                        <strong>{state.name}</strong>
                        <span>{connectionDisplay(state.connection, t)}</span>
                      </button>
                    );
                  })}
                </section>
              </div>
            ) : (
              <h1>{device.name}</h1>
            )}
            {device.batteries.length > 0 && (
            <div ref={batteryControlRef} className={`battery-control ${showBatteries ? 'open' : ''}`}>
              <button
                className="battery-state"
                aria-expanded={showBatteries}
                aria-controls="device-batteries"
                onClick={() => setShowBatteries((visible) => !visible)}
              >
                <BatteryHigh weight="regular" />
                {device.batteries[0].percentage}%
                {device.batteries[0].charging ? ` · ${t('common.charging')}` : ''}
                <span className="battery-count">{t('dashboard.deviceCount', { count: device.batteries.length })}</span>
              </button>
              <section id="device-batteries" className="battery-popover" aria-label={t('dashboard.deviceBattery')}>
                <p>{t('dashboard.deviceBattery')}</p>
                {device.batteries.map((battery) => (
                  <div key={battery.id} className="battery-device">
                    <span><BatteryHigh weight="regular" />{t(battery.label, { defaultValue: battery.label })}</span>
                    <strong>{battery.percentage}%{battery.charging ? ` · ${t('common.charging')}` : ''}</strong>
                  </div>
                ))}
              </section>
            </div>
            )}
          </div>
        </div>
        <DeviceAura color={device.lighting?.mouseLightColor} />
      </section>

      <div
        className="control-tabs"
        role="tablist"
        aria-label={t('dashboard.deviceControl')}
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

      <section className={`control-stage ${previewMessage ? 'has-preview-message' : ''}`} aria-live="polite">
        {activeDpiDescriptor && (
          <div className="control-reading dpi-reading">
            <button
              type="button"
              className="primary-reading editable-reading"
              aria-label={activeDpi ? t('dashboard.currentDpiEdit', { value: activeDpi }) : t('dashboard.dpiNotReported')}
              disabled={writeBusy || !setDpiMutation || !writable(setDpiMutation) || !activeDpi}
              onClick={() => activeDpi && setEditingDpiStage(currentStage)}
            >
              <strong>{activeDpi || i18n.t('common.notReported')}</strong><em>DPI</em>
            </button>
            <div className="dpi-scale" aria-label={t('dashboard.dpiStages')} style={{ '--stage-count': Math.max(displayedStages.length, 1) } as React.CSSProperties}>
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
                      aria-label={t('dashboard.switchToStage', { stage: stageNumber })}
                    >
                      <i style={{ '--stage-source-color': pastelDisplayColor(stage.color) } as React.CSSProperties} />
                    </button>
                    <button
                      type="button"
                      className="dpi-stage-value"
                      disabled={writeBusy || !setDpiMutation || !writable(setDpiMutation)}
                      onClick={() => setEditingDpiStage(stageNumber)}
                      aria-label={t('dashboard.editStageDpi', { stage: stageNumber })}
                    >
                      {stage.value}
                    </button>
                  </div>
                );
              })}
            </div>
            {displayedStages.length === 0 && <p className="setting-hint">{t('dashboard.noDpiStages')}</p>}
            {(!setDpiMutation || !writable(setDpiMutation)) && displayedStages.length > 0 && <p className="setting-hint">{t('dashboard.dpiWriteUnavailable')}</p>}
            {editingDpiStage !== null && (
              <DpiEditModal
                stage={editingDpiStage}
                currentValue={stages[editingDpiStage - 1]?.value ?? activeDpi}
                range={activeDpiDescriptor ? pluginRange(activeDpiDescriptor) : undefined}
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
              aria-label={t('dashboard.lightingTarget')}
              style={{ gridTemplateColumns: `repeat(${supportsReceiverLighting ? 2 : 1}, minmax(0, 1fr))` }}
            >
              <button
                role="tab"
                aria-selected={lightingView === 'mouse'}
                className={lightingView === 'mouse' ? 'active' : ''}
                onClick={() => setLightingView('mouse')}
              >{t('dashboard.mouseLighting')}</button>
              {supportsReceiverLighting && (
                <button
                  role="tab"
                  aria-selected={lightingView === 'receiver'}
                  className={lightingView === 'receiver' ? 'active' : ''}
                  onClick={() => setLightingView('receiver')}
                >{t('dashboard.receiverLighting')}</button>
              )}
            </div>
            <div className="lighting-swatch" style={{ '--light-color': (
              activeLightingView === 'mouse' ? device.lighting?.mouseLightColor : device.lighting?.receiverLightColor
            ) ?? '#b87ab0' } as React.CSSProperties} />
            <div className="lighting-sections" aria-label={t('dashboard.lightingGroups')}>
              {activeLightingView === 'mouse' && (
                <div className="lighting-group lighting-group-mouse">
                  <p className="lighting-group-title">{t('dashboard.mouseLighting')}</p>
                  <div
                    className="lighting-rows"
                    style={{
                      gridTemplateColumns: `repeat(${mouseLightingRowCount}, minmax(0, 1fr))`,
                    }}
                  >
                    <button
                      type="button"
                      className="lighting-row"
                      disabled={writeBusy || !mouseLightingMutation || !writable(mouseLightingMutation)}
                      onClick={() => {
                        if (hasMouseEffectOptions) {
                          const currentEffect = device.lighting?.mouseLightEffect ?? mouseOffEffect;
                          const isOff = currentEffect === mouseOffEffect;
                          // 开启时恢复上一次非 OFF 的灯效（如 BREATHING），
                          // 而不是始终回退到 FIXED/常亮；关闭时设为 OFF。
                          const mouseSupportedEffects = activeLightingDescriptor
                            ? pluginSupportedEffectValues(activeLightingDescriptor, device.capabilities)
                            : [];
                          const mouseRestoreEffect = mouseSupportedEffects.includes(lastNonOffEffectRef.current)
                            ? lastNonOffEffectRef.current
                            : mouseSupportedEffects.find((effect) => effect !== mouseOffEffect) ?? mouseOnEffect;
                          const newEffect = isOff ? mouseRestoreEffect : mouseOffEffect;
                          void runMutation(mouseLightingMutation!, {
                            color: device.lighting?.mouseLightColor ?? '#b87ab0',
                            enabled: newEffect !== mouseOffEffect,
                            effect: newEffect,
                            speed: device.lighting?.mouseLightSpeed ?? mouseEffectDefaults.speed,
                            brightness: device.lighting?.mouseLightBrightness ?? mouseEffectDefaults.brightness,
                            extraColor: device.lighting?.mouseLightExtraColor ?? '#000000',
                          });
                        } else {
                          const enabled = mouseLightOn === false;
                          void runMutation(mouseLightingMutation!, {
                            color: device.lighting?.mouseLightColor ?? '#b87ab0',
                            enabled,
                          });
                        }
                      }}
                    >
                      <span>{t('dashboard.status')}</span>
                      <strong>{mouseLightOn === false ? i18n.t('common.off') : i18n.t('common.on')}</strong>
                    </button>
                    {hasMouseEffectOptions ? (
                      <>
                        <button
                          type="button"
                          className="lighting-row"
                          disabled={writeBusy || !mouseLightingMutation || !writable(mouseLightingMutation)}
                          onClick={() => setEditingMouseLightField('effect')}
                        >
                          <span>{i18n.t('receiverLighting.field.effect')}</span>
                          <strong>{(() => {
                            const eff = device.lighting?.mouseLightEffect;
                            if (eff === undefined) return i18n.t('common.notReported');
                            // 优先使用设备运行时报送的 effectName（来自插件 derived.lookup），
                            // 避免在插件 locale 未加载时显示原始 i18n key。
                            const runtimeName = lightingCapability(device.capabilities, 'mouseLighting')?.effectName;
                            if (typeof runtimeName === 'string' && runtimeName) return runtimeName;
                            const opt = mouseEffectOptions?.effect?.find((e) => e.value === eff);
                            return opt ? resolveLabelKey(opt.labelKey, device.pluginId) : i18n.t('lighting.effectN', { value: eff });
                          })()}</strong>
                        </button>
                        {device.lighting?.supportsSpeed && (
                          <button
                            type="button"
                            className="lighting-row"
                            disabled={writeBusy || !mouseLightingMutation || !writable(mouseLightingMutation)}
                            onClick={() => setEditingMouseLightField('speed')}
                          >
                            <span>{i18n.t('receiverLighting.field.speed')}</span>
                            <strong>{device.lighting?.mouseLightSpeed ?? 0}</strong>
                          </button>
                        )}
                        {device.lighting?.supportsBrightness && (
                          <button
                            type="button"
                            className="lighting-row"
                            disabled={writeBusy || !mouseLightingMutation || !writable(mouseLightingMutation)}
                            onClick={() => setEditingMouseLightField('brightness')}
                          >
                            <span>{i18n.t('receiverLighting.field.brightness')}</span>
                            <strong>{(device.lighting?.mouseLightBrightness ?? 100)}%</strong>
                          </button>
                        )}
                        <button
                          type="button"
                          className="lighting-row"
                          disabled={writeBusy || !mouseLightingMutation || !writable(mouseLightingMutation)}
                          onClick={() => setEditingMouseLightField('color')}
                        >
                          <span>{i18n.t('common.color')}</span>
                          <ColorValue value={device.lighting?.mouseLightColor} />
                        </button>
                        {mouseNeedsExtraColor && (
                          <button
                            type="button"
                            className="lighting-row"
                            disabled={writeBusy || !mouseLightingMutation || !writable(mouseLightingMutation)}
                            onClick={() => setEditingMouseLightField('extraColor')}
                          >
                            <span>{i18n.t('lighting.extraColor')}</span>
                            <ColorValue value={device.lighting?.mouseLightExtraColor ?? '#000000'} />
                          </button>
                        )}
                      </>
                    ) : (
                      <>
                        <button
                          type="button"
                          className="lighting-row"
                          disabled={writeBusy || !mouseLightingMutation || !writable(mouseLightingMutation)}
                          onClick={() => setEditingMouseLightColor(true)}
                        >
                          <span>{i18n.t('common.color')}</span>
                          <ColorValue value={device.lighting?.mouseLightColor} />
                        </button>
                        {mouseHasEndColor && (
                          <button
                            type="button"
                            className="lighting-row"
                            disabled={writeBusy || !mouseLightingMutation || !writable(mouseLightingMutation)}
                            onClick={() => setEditingMouseLightEndColor(true)}
                          >
                            <span>{t('dashboard.endColor')}</span>
                            <ColorValue value={device.lighting?.mouseLightEndColor} />
                          </button>
                        )}
                      </>
                    )}
                  </div>
                  {editingMouseLightEndColor && (
                    <ColorEditModal
                      title={t('dashboard.mouseLightEndColor')}
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
                  {editingMouseLightField && hasMouseEffectOptions && mouseEffectOptions?.effect && (
                    <MouseLightingEditModal
                      field={editingMouseLightField}
                      pluginId={device.pluginId}
                      initial={{
                        effect: device.lighting?.mouseLightEffect ?? mouseEffectOptions?.offValue ?? 0,
                        speed: device.lighting?.mouseLightSpeed ?? 0,
                        brightness: device.lighting?.mouseLightBrightness ?? 100,
                        color: device.lighting?.mouseLightColor ?? '#b87ab0',
                        extraColor: device.lighting?.mouseLightExtraColor,
                      }}
                      effectOptions={mouseEffectOptions.effect}
                      offValue={mouseEffectOptions.offValue}
                      supportedEffects={mouseSupportedEffects}
                      speedRange={mouseEffectOptions.speed ?? { min: 0, max: 255, step: 1 }}
                      brightnessRange={mouseEffectOptions.brightness ?? { min: 0, max: 100, step: 1 }}
                      writeBusy={writeBusy}
                      onClose={() => setEditingMouseLightField(null)}
                      onApply={(params) => {
                        void runMutation(mouseLightingMutation!, params);
                        setEditingMouseLightField(null);
                      }}
                    />
                  )}
                </div>
              )}
              {activeLightingView === 'receiver' && (
                <div className="lighting-group lighting-group-dongle">
                  <p className="lighting-group-title">{t('dashboard.receiverLighting')}</p>
                  <div className="lighting-rows" style={{ gridTemplateColumns: 'repeat(5, minmax(0, 1fr))' }}>
                    <button
                      type="button"
                      className="lighting-row"
                      disabled={writeBusy || !receiverLightingMutation || !writable(receiverLightingMutation)}
                      onClick={() => setEditingReceiverLighting('effect')}
                    >
                      <span>{i18n.t('receiverLighting.field.effect')}</span>
                      <strong>{device.lighting?.receiverLightMode ? i18n.t(device.lighting.receiverLightMode, { defaultValue: device.lighting.receiverLightMode }) : i18n.t('common.notReported')}</strong>
                    </button>
                    <button
                      type="button"
                      className="lighting-row"
                      disabled={writeBusy || !receiverLightingMutation || !writable(receiverLightingMutation)}
                      onClick={() => setEditingReceiverLighting('option')}
                    >
                      <span>{i18n.t('receiverLighting.field.option')}</span>
                      <strong>{getLightingColorMode(device.capabilities, 'receiverLighting')}</strong>
                    </button>
                    <button
                      type="button"
                      className="lighting-row"
                      disabled={writeBusy || !receiverLightingMutation || !writable(receiverLightingMutation)}
                      onClick={() => setEditingReceiverLighting('speed')}
                    >
                      <span>{i18n.t('receiverLighting.field.speed')}</span>
                      <strong>{preferredCapability(device.capabilities, 'receiverLighting', 'speedLabel', 'speed')}</strong>
                    </button>
                    <button
                      type="button"
                      className="lighting-row"
                      disabled={writeBusy || !receiverLightingMutation || !writable(receiverLightingMutation)}
                      onClick={() => setEditingReceiverLighting('brightness')}
                    >
                      <span>{i18n.t('receiverLighting.field.brightness')}</span>
                      <strong>{preferredCapability(device.capabilities, 'receiverLighting', 'brightnessLabel', 'brightness')}</strong>
                    </button>
                    <button
                      type="button"
                      className="lighting-row"
                      disabled={writeBusy || !receiverLightingMutation || !writable(receiverLightingMutation)}
                      onClick={() => setEditingReceiverLighting('color')}
                    >
                      <span>{i18n.t('common.color')}</span>
                      <ColorValue value={receiverLighting?.color} />
                    </button>
                  </div>
                  {editingReceiverLighting !== null && receiverLighting && (
                    <ReceiverLightingEditModal
                      field={editingReceiverLighting}
                      pluginId={device.pluginId}
                      initial={{
                        effect: Number(receiverLighting.effect ?? 0),
                        speed: Number(receiverLighting.speed ?? 0),
                        brightness: Number(receiverLighting.brightness ?? 0),
                        option: Number(receiverLighting.option ?? 0),
                        color: String(receiverLighting.color ?? '#b87ab0'),
                      }}
                      options={activeLightingDescriptor?.metadata?.receiverLightingOptions as ReceiverLightingOptions | undefined}
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
            key={descriptor.id}
            capability={descriptor}
            device={device}
            writeBusy={writeBusy}
            runMutation={runMutation}
          />
        ))}
        {previewMessage && <p className="preview-message">{previewMessage}</p>}
        {editingSleep && sleepSetting && (
          <SleepEditModal
            label={sleepSetting.label}
            currentSeconds={sleepSetting.seconds}
            range={sleepSetting.range}
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
            title={t('dashboard.mouseLightColor')}
            currentColor={device.lighting?.mouseLightColor ?? '#b87ab0'}
            writeBusy={writeBusy}
            onClose={() => setEditingMouseLightColor(false)}
            onApply={(color) => {
              const statusLighting = statusPlacements.find(({ capability }) => capability.control === 'LightingZone')?.capability;
              const mutation = statusLighting ? pluginMutations(statusLighting, device.writableMutations).mouse : mouseLightingMutation;
              if (!mutation) return;
              void runMutation(
                mutation,
                {
                  color,
                  enabled: mouseLightingOnState(device, statusLighting) !== false,
                  effect: device.lighting?.mouseLightEffect ?? mouseEffectDefaults.effect,
                  speed: device.lighting?.mouseLightSpeed ?? mouseEffectDefaults.speed,
                  brightness: device.lighting?.mouseLightBrightness ?? mouseEffectDefaults.brightness,
                  extraColor: device.lighting?.mouseLightExtraColor ?? '#000000',
                },
              );
              setEditingMouseLightColor(false);
            }}
          />
        )}
      </section>

      {statusItems.length > 0 && (
      <section
        className="status-strip"
        aria-label={t('dashboard.deviceStatus')}
        data-status-count={statusItems.length}
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
        <span>{t('dashboard.lastUpdate', { time: device.updatedAt })}</span>
        <button className="details-button" onClick={() => setShowDetails(true)}><ReadCvLogo weight="regular" />{t('dashboard.allReadInfo')}</button>
      </div>
      {showDetails && <DeviceDetails capabilities={device.capabilities} pluginCapabilities={pluginDescriptors} onClose={() => setShowDetails(false)} />}
    </main>
  );
}

export default function App() {
  const { t } = useTranslation();
  const pureWeb = isPureWebPreview();
  const [device, setDevice] = useState<DeviceState | undefined>(pureWeb ? MOCK_DEVICE : undefined);
  const [deviceEntries, setDeviceEntries] = useState<DeviceSnapshotEntry[]>(pureWeb ? MOCK_DEVICE_ENTRIES : []);
  const deviceEntriesRef = useRef<DeviceSnapshotEntry[]>(pureWeb ? MOCK_DEVICE_ENTRIES : []);
  // F11: device-updated 事件高频触发时，用 startTransition 将 Dashboard 渲染标记为低优先级，
  // 避免 settling polls 期间（6 次 500ms）阻塞主线程。writeBusy 等用户交互状态保持同步。
  const [, startTransition] = useTransition();
  const [theme, setTheme] = useState<ThemeMode>('system');
  const [themeLoaded, setThemeLoaded] = useState(pureWeb);
  const [view, setView] = useState<View>('dashboard');
  const [aboutFocusToken, setAboutFocusToken] = useState(0);
  const [settingsPluginFocusToken, setSettingsPluginFocusToken] = useState(0);
  const [demoMode, setDemoMode] = useState(pureWeb);
  const [refreshNonce, setRefreshNonce] = useState(0);
  const [appNotification, setAppNotification] = useState<AppNotification>();
  const [pluginLocaleRevision, setPluginLocaleRevision] = useState(0);
  const windowsPlatform = isWindowsPlatform();
  const macPlatform = isMacPlatform();
  const windowsWebPreview = isWindowsWebPreview();
  const fallbackPlatform = !pureWeb && !windowsPlatform && !macPlatform;
  const exitDemo = useCallback(() => {
    setDemoMode(false);
    setDevice(undefined);
    setDeviceEntries([]);
    deviceEntriesRef.current = [];
    setView('dashboard');
    setRefreshNonce((value) => value + 1);
    invoke('device_refresh').catch(() => {});
  }, []);
  const openAboutUpdate = useCallback(() => {
    setView('about');
    setAboutFocusToken((value) => value + 1);
  }, []);
  const openSettingsPluginUpdate = useCallback(() => {
    setView('settings');
    setSettingsPluginFocusToken((value) => value + 1);
  }, []);

  useEffect(() => onAppNotification(setAppNotification), []);

  useEffect(() => {
    if (pureWeb) return;
    let unlisten: (() => void) | undefined;
    let unlistenResume: (() => void) | undefined;
    listen('navigate-about-update', () => openAboutUpdate())
      .then((un) => { unlisten = un; })
      .catch(() => {});
    listen('window-resumed', () => {
      setRefreshNonce((value) => value + 1);
    }).then((un) => { unlistenResume = un; })
      .catch(() => {});
    return () => {
      if (unlisten) unlisten();
      if (unlistenResume) unlistenResume();
    };
  }, [openAboutUpdate, pureWeb]);

  // 加载插件 locale，注册为 i18n namespace（以插件 ID 命名）。
  // 异步加载完成后刷新插件标签 memo，加载前使用 host 回退标签。
  useEffect(() => {
    let cancelled = false;
    void loadPluginLocales().then((loaded) => {
      if (loaded && !cancelled) setPluginLocaleRevision((value) => value + 1);
    });
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    if (!appNotification) return;
    const timeout = window.setTimeout(() => setAppNotification(undefined), 6000);
    return () => window.clearTimeout(timeout);
  }, [appNotification]);

  // 从后端加载已保存的主题设置
  useEffect(() => {
    if (pureWeb) return;
    invoke<AppSettings>('settings_get')
      .then((settings) => {
        setTheme(settings.theme as ThemeMode);
        setThemeLoaded(true);
        applyLanguage(settings.language ?? 'auto');
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
              if (available.length > 0) {
                notifyInfo(
                  i18n.t('dashboard.pluginUpdateFound'),
                  i18n.t('dashboard.pluginUpdateFoundBody', { count: available.length }),
                  'settings-plugin-update',
                );
              }
            })
            .catch(() => { /* Automatic checks stay quiet when offline. */ });
        }
      })
      .catch(() => setThemeLoaded(true));
  }, [pureWeb]);

  // 周期性从后端读取真实设备状态
  useEffect(() => {
    if (demoMode) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    let unlistenEntries: (() => void) | undefined;

    // 启动时立即读取一次缓存
    invoke<DeviceSnapshotEntry[]>('device_snapshots')
      .then((entries) => {
        if (!cancelled) {
          deviceEntriesRef.current = entries;
          setDeviceEntries(entries);
          setDevice(entryToState(selectedDeviceEntry(entries)));
        }
      })
      .catch(() => {
        if (!cancelled) {
          setDeviceEntries([]);
          setDevice(undefined);
        }
      });

    // 监听后台线程发出的 device-updated 事件，无需轮询
    listen<DeviceSnapshot | null>('device-updated', (event) => {
      if (cancelled) return;
      const snapshot = event.payload;
      // F11: 高频 device-updated 事件用 startTransition 降低渲染优先级，
      // 让用户交互（如点击按钮）能优先响应。
      startTransition(() => {
        setDevice(deviceEntriesRef.current.length > 1
          ? entryToState(selectedDeviceEntry(deviceEntriesRef.current))
          : (snapshot ? snapshotToState(snapshot) : undefined));
      });
    }).then((un) => {
      if (cancelled) {
        un();
      } else {
        unlisten = un;
      }
    }).catch(() => {});

    listen<DeviceSnapshotEntry[]>('device-snapshots-updated', (event) => {
      if (cancelled) return;
      const entries = event.payload;
      deviceEntriesRef.current = entries;
      startTransition(() => {
        setDeviceEntries(entries);
        setDevice(entryToState(selectedDeviceEntry(entries)));
      });
    }).then((un) => {
      if (cancelled) {
        un();
      } else {
        unlistenEntries = un;
      }
    }).catch(() => {});

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
      if (unlistenEntries) unlistenEntries();
    };
  }, [demoMode, refreshNonce]);

  const selectDevice = useCallback((deviceKey: string) => {
    if (demoMode) {
      const nextEntries = (deviceEntriesRef.current.length ? deviceEntriesRef.current : MOCK_DEVICE_ENTRIES)
        .map((entry) => ({ ...entry, selected: entry.deviceKey === deviceKey }));
      deviceEntriesRef.current = nextEntries;
      setDeviceEntries(nextEntries);
      setDevice(entryToState(selectedDeviceEntry(nextEntries)));
      return;
    }
    void invoke<DeviceSnapshot>('device_select', { deviceKey })
      .then((snapshot) => {
        setDevice(snapshotToState(snapshot));
        setDeviceEntries((entries) => {
          const nextEntries = entries.map((entry) => ({ ...entry, selected: entry.deviceKey === deviceKey }));
          deviceEntriesRef.current = nextEntries;
          return nextEntries;
        });
      })
      .catch((error) => notifyError(i18n.t('notification.selectDeviceFailed'), String(error)));
  }, [demoMode]);

  const activeDpiColor = device?.dpiStages.find((stage) => stage.enabled && stage.active)?.color
    ?? device?.dpiStages.find((stage) => stage.enabled)?.color;
  const themeColor = device?.lighting?.mouseLightColor ?? activeDpiColor;
  useEffect(() => {
    if (!themeLoaded) return;
    applyTheme(theme, themeColor);
  }, [themeLoaded, theme, themeColor]);

  return <div className={`app-shell ${pureWeb ? 'web-preview' : ''} ${windowsPlatform ? 'platform-windows' : ''} ${macPlatform ? 'platform-macos' : ''} ${fallbackPlatform ? 'platform-fallback' : ''} ${windowsWebPreview ? 'windows-web-preview' : ''}`}>
    {windowsWebPreview && <WindowsPreviewControls />}
    {windowsPlatform && !windowsWebPreview && !pureWeb && <WindowsWindowControls />}
    {windowsPlatform && !windowsWebPreview && !pureWeb && <div className="windows-drag-strip" data-tauri-drag-region />}
    <nav className="top-nav" data-tauri-drag-region />
    <div className="nav-links">
      <button className={`nav-link ${view === 'dashboard' ? 'active' : ''}`} onClick={() => setView('dashboard')}>{t('nav.dashboard')}</button>
      <button className={`nav-link ${view === 'settings' ? 'active' : ''}`} onClick={() => setView('settings')}>{t('nav.settings')}</button>
      <button className={`nav-link nav-about ${view === 'about' ? 'active' : ''}`} onClick={() => setView('about')} aria-label={t('nav.about')}><Info weight="regular" /></button>
      {demoMode && <button className="nav-link nav-exit" onClick={exitDemo} aria-label={t('nav.exitDemo')} title={t('nav.exitDemo')}><SignOut weight="regular" /></button>}
    </div>
    {view === 'dashboard' && (device ? <Dashboard device={device} deviceEntries={deviceEntries} onDeviceChange={setDevice} onDeviceSelect={selectDevice} pluginLocaleRevision={pluginLocaleRevision} /> : <EmptyState onRefresh={() => { setDemoMode(false); setDevice(undefined); setDeviceEntries([]); deviceEntriesRef.current = []; setRefreshNonce((value) => value + 1); invoke('device_refresh').catch(() => {}); }} onDemo={() => { setDemoMode(true); setDevice(MOCK_DEVICE); setDeviceEntries(MOCK_DEVICE_ENTRIES); deviceEntriesRef.current = MOCK_DEVICE_ENTRIES; }} onOpenSettings={() => setView('settings')} />)}
    {view === 'settings' && <SettingsPage previewMode={pureWeb} focusPluginUpdateToken={settingsPluginFocusToken} onNavigateAbout={() => setView('about')} onThemeChange={setTheme} supportsAnyLighting={device ? pluginSupportsAnyLighting(compatibilityCapabilities(device), device.writableMutations) : false} supportsMouseLighting={device ? pluginSupportsLightingMutation(compatibilityCapabilities(device), device.writableMutations, 'mouse') : false} supportsReceiverLighting={device ? pluginSupportsLightingMutation(compatibilityCapabilities(device), device.writableMutations, 'receiver') : false} />}
    {view === 'about' && <AboutPage previewMode={pureWeb} focusUpdateToken={aboutFocusToken} onBack={() => setView('settings')} />}
    {appNotification && (
      <aside
        className={`app-notification ${appNotification.kind} ${appNotification.action ? 'actionable' : ''}`}
        role={appNotification.kind === 'error' ? 'alert' : 'status'}
        aria-live={appNotification.kind === 'error' ? 'assertive' : 'polite'}
        onClick={appNotification.action === 'about-update' ? openAboutUpdate : appNotification.action === 'settings-plugin-update' ? openSettingsPluginUpdate : appNotification.action === 'relaunch' ? () => void relaunchAfterUpdate() : undefined}
      >
        <div><strong>{appNotification.title}</strong>{appNotification.body && <p>{appNotification.body}</p>}</div>
        <button type="button" onClick={(event) => { event.stopPropagation(); setAppNotification(undefined); }} aria-label={t('dashboard.closeNotification')}><X weight="bold" /></button>
      </aside>
    )}
  </div>;
}
