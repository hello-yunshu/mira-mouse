// SPDX-License-Identifier: AGPL-3.0-or-later
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useTransition } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  CaretDown,
  ChartBar,
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
import { BatteryUsageModal } from './BatteryUsage';
import { BatteryLevelIcon } from './BatteryLevelIcon';
import type { AboutInfo, AppSettings, DeviceSnapshot, DeviceSnapshotEntry, DeviceState, DpiStage, PluginCapability, PluginCapabilityPlacement, PluginField, PluginFieldFormat, RangeSpec, ThemeMode } from './types';
import {
  MAX_CONTROL_GROUPS,
  MAX_STATUS_ITEMS,
  readPath,
  resolveDetailValueLabel,
  resolveFieldMutationParams,
  resolveFieldParams,
  resolveMutation,
  resolveFieldLabel,
  resolveFieldValueLabel,
  resolveFieldOptions,
  resolveFieldRange,
  resolveStageLayout,
  resolveStateMapping,
  resolveStatusField,
  resolveStatusDisplay,
  resolveSwitchState,
  resolveVisibleWhen,
  resolveZones,
  simulateDemoMutation,
} from './pluginAdapter';
import { onAppNotification, notifyError, notifySuccess, type AppNotification } from './notify';
import { relaunchAfterUpdate, startAutomaticAppUpdateCheck } from './updater';
import { startAutomaticPluginUpdateCheck } from './plugin-updater';
import { LOCAL_AI_FEATURE, localAiFeatureEnabled } from './localAi';
import { segmentedIndicatorStyle } from './segmentedControl';
import { Modal, OverlayPortal, useHasOpenModal } from './overlay';
import './styles.css';

type View = 'dashboard' | 'settings' | 'about';
type ControlMode = string;

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

function formatSleepTime(value: unknown): string {
  const seconds = typeof value === 'number' ? value : Number(value);
  if (!Number.isFinite(seconds) || seconds <= 0) return i18n.t('common.notReported');
  if (seconds % 60 === 0) return i18n.t('common.minute', { count: seconds / 60 });
  return i18n.t('common.second', { count: seconds });
}

/// 按 PluginFieldFormat 格式化字段值。
function formatFieldValue(value: unknown, format: PluginFieldFormat | undefined, t: (key: string, options?: Record<string, unknown>) => string): string {
  if (value === null || value === undefined || value === '') return t('common.notReported');
  switch (format) {
    case 'sleep': return formatSleepTime(value);
    case 'percent': return `${value}%`;
    case 'hertz': return `${value} Hz`;
    case 'connection': return connectionDisplay(typeof value === 'string' ? value : undefined, t);
    case 'color': return typeof value === 'string' ? value : String(value);
    case 'default':
    default:
      if (typeof value === 'boolean') return value ? t('common.on') : t('common.off');
      return String(value);
  }
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

function secondaryRevealStyle(seed: string): React.CSSProperties {
  let hash = 2166136261;
  for (const char of seed) {
    hash ^= char.charCodeAt(0);
    hash = Math.imul(hash, 16777619);
  }
  const delay = 165 + (Math.abs(hash) % 45);
  return { '--control-detail-delay': `${delay}ms` } as React.CSSProperties;
}

function LiveValue({ text, className, style, duration = 160 }: {
  text: string;
  className?: string;
  style?: React.CSSProperties;
  duration?: number;
}) {
  const [currentValue, setCurrentValue] = useState(() => ({ text, style }));
  const [nextValue, setNextValue] = useState<{ text: string; style?: React.CSSProperties }>();
  const [transitioning, setTransitioning] = useState(false);

  useEffect(() => {
    if (text === currentValue.text && style === currentValue.style) return;

    let transitionFrame = 0;
    let timeout = 0;
    const incomingValue = { text, style };
    const prepareFrame = window.requestAnimationFrame(() => {
      setNextValue(incomingValue);
      setTransitioning(false);
      transitionFrame = window.requestAnimationFrame(() => {
        setTransitioning(true);
        timeout = window.setTimeout(() => {
          setCurrentValue(incomingValue);
          setNextValue(undefined);
          setTransitioning(false);
        }, duration);
      });
    });

    return () => {
      window.cancelAnimationFrame(prepareFrame);
      window.cancelAnimationFrame(transitionFrame);
      window.clearTimeout(timeout);
    };
  }, [currentValue.style, currentValue.text, duration, style, text]);

  return (
    <strong
      className={[className, 'live-value', transitioning ? 'is-transitioning' : undefined].filter(Boolean).join(' ')}
      aria-label={text}
    >
      <span className="live-value-current" style={currentValue.style} aria-hidden="true">{currentValue.text}</span>
      {nextValue !== undefined && (
        <span className="live-value-next" style={nextValue.style} aria-hidden="true">{nextValue.text}</span>
      )}
    </strong>
  );
}

type MetricFlipValue = {
  contextKey: string;
  text: string;
  unit: string;
  variant: SharedControlMetric['variant'];
};

function MorphingMetricValue({
  active,
  contextKey,
  text,
  unit,
  variant,
  duration = 280,
}: MetricFlipValue & { active: boolean; duration?: number }) {
  const [currentValue, setCurrentValue] = useState<MetricFlipValue>(() => ({
    contextKey,
    text,
    unit,
    variant,
  }));
  const [nextValue, setNextValue] = useState<MetricFlipValue>();
  const [transitioning, setTransitioning] = useState(false);
  const [transitionKind, setTransitionKind] = useState<'crossfade' | 'flip'>('crossfade');

  useEffect(() => {
    if (!active) return;
    if (
      text === currentValue.text
      && unit === currentValue.unit
      && variant === currentValue.variant
      && contextKey === currentValue.contextKey
    ) return;

    let transitionFrame = 0;
    let commitFrame = 0;
    let timeout = 0;
    const incomingValue = {
      contextKey,
      text,
      unit,
      variant,
    };
    const incomingTransitionKind = contextKey === currentValue.contextKey ? 'flip' : 'crossfade';
    const transitionDuration = incomingTransitionKind === 'crossfade' ? 360 : duration;
    const prepareFrame = window.requestAnimationFrame(() => {
      setTransitionKind(incomingTransitionKind);
      setNextValue(incomingValue);
      setTransitioning(false);
      transitionFrame = window.requestAnimationFrame(() => {
        setTransitioning(true);
        timeout = window.setTimeout(() => {
          commitFrame = window.requestAnimationFrame(() => {
            setCurrentValue(incomingValue);
            setNextValue(undefined);
            setTransitioning(false);
          });
        }, transitionDuration);
      });
    });

    return () => {
      window.cancelAnimationFrame(prepareFrame);
      window.cancelAnimationFrame(transitionFrame);
      window.cancelAnimationFrame(commitFrame);
      window.clearTimeout(timeout);
    };
  }, [active, contextKey, currentValue, duration, text, unit, variant]);

  const renderFace = (value: MetricFlipValue, className: string) => (
    <span
      key={`${value.contextKey}\u0000${value.variant}\u0000${value.text}\u0000${value.unit}`}
      className={`shared-control-metric-face ${className}`}
      data-variant={value.variant}
    >
      <strong className="shared-control-metric-text">{value.text}</strong>
      {value.unit && <em>{value.unit}</em>}
    </span>
  );

  return (
    <span
      className={`shared-control-metric-value${transitioning ? ' is-transitioning' : ''}`}
      data-transition={transitionKind}
      aria-label={`${text}${unit ? ` ${unit}` : ''}`}
    >
      {renderFace(currentValue, 'is-current')}
      {nextValue && renderFace(nextValue, 'is-next')}
    </span>
  );
}

function ColorValue({ value, fallback, className }: { value: unknown; fallback?: string; className?: string }) {
  const label = typeof value === 'string' && value ? value : fallback ?? i18n.t('common.notReported');
  const style = useMemo(() => colorValueStyle(value), [value]);
  const classes = [className, style ? 'color-value' : undefined].filter(Boolean).join(' ') || undefined;
  return <LiveValue text={label} className={classes} style={style} duration={220} />;
}

function FormattedValue({ value, format, label, className }: {
  value: unknown;
  format?: PluginFieldFormat;
  label?: string;
  className?: string;
}) {
  const text = label ?? formatFieldValue(value, format, i18n.t);
  const isColor = shouldRenderColorValue(value, format);
  return isColor
    ? <ColorValue className={className} value={value} fallback={text} />
    : <LiveValue text={text} className={className} />;
}

function CapabilitySummary({ capability, device }: { capability: PluginCapability; device: DeviceState }) {
  const items = capability.metadata.summary ?? [];
  if (items.length === 0) return null;
  return (
    <div
      className="capability-summary"
      aria-label={i18n.t('dashboard.deviceSummary')}
      style={{ gridTemplateColumns: `repeat(${items.length}, minmax(0, 1fr))` }}
    >
      {items.map((item) => {
        const value = readPath(device, item.source);
        const option = item.options?.find((candidate) => candidate.value === value);
        const valueLabel = option
          ? resolveLabelKey(option.labelKey, device.pluginId)
          : `${formatFieldValue(value, item.format, i18n.t)}${item.unit ? ` ${item.unit}` : ''}`;
        const label = item.labelKey
          ? resolveLabelKey(item.labelKey, device.pluginId)
          : item.label ?? item.source;
        return (
          <span
            key={`${label}:${item.source}`}
            className="secondary-control-item"
            style={secondaryRevealStyle(`${capability.id}:${item.source}:${label}`)}
          >
            {label}
            <FormattedValue value={value} format={item.format} label={valueLabel} />
          </span>
        );
      })}
    </div>
  );
}

function capabilityGroupLabel(group: string): string {
  return i18n.t(`capability.group.${group}`, { defaultValue: group });
}

function capabilityFieldLabel(key: string): string {
  return i18n.t(`capability.field.${key}`, { defaultValue: key });
}

function readSnapshotPath(snapshot: DeviceSnapshot, path: string): unknown {
  const parts = path.split('.');
  let current: unknown = snapshot;
  for (const part of parts) {
    if (current == null) return undefined;
    if (Array.isArray(current)) {
      const idx = Number(part);
      if (!Number.isInteger(idx)) return undefined;
      current = current[idx];
    } else if (typeof current === 'object') {
      current = (current as Record<string, unknown>)[part];
    } else {
      return undefined;
    }
  }
  return current;
}

function snapshotToState(snapshot: DeviceSnapshot): DeviceState {
  const now = new Date().toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' });
  const capabilities = snapshot.pluginCapabilities ?? [];
  const stateMapping = resolveStateMapping(capabilities);
  const state: Record<string, unknown> = {};
  for (const [field, source] of Object.entries(stateMapping)) {
    state[field] = readSnapshotPath(snapshot, source);
  }
  return {
    name: snapshot.displayName ?? i18n.t('common.unknownDevice'),
    connection: snapshot.connection,
    battery: snapshot.batteryPercent,
    charging: snapshot.charging,
    batteries: snapshot.batteries ?? [],
    state,
    capabilities: snapshot.capabilities ?? {},
    pluginCapabilities: capabilities,
    writableMutations: snapshot.writableMutations ?? [],
    evidence: snapshot.evidence,
    readonly: snapshot.readonly ?? false,
    pluginId: snapshot.pluginId,
    updatedAt: now,
  };
}

function selectedDeviceEntry(entries: DeviceSnapshotEntry[]): DeviceSnapshotEntry | undefined {
  return entries.find((entry) => entry.selected) ?? entries[0];
}

function entryToState(entry: DeviceSnapshotEntry | undefined): DeviceState | undefined {
  return entry ? snapshotToState(entry.snapshot) : undefined;
}

function batteryUsageTarget(entry: DeviceSnapshotEntry | undefined) {
  const snapshot = entry?.snapshot;
  if (!snapshot) return undefined;
  const battery = snapshot.batteries?.find((item) => item.id === 'mouse') ?? snapshot.batteries?.[0];
  return {
    name: snapshot.historyIdentity?.displayName ?? snapshot.displayName,
    componentId: battery?.id,
  };
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

type PluginRegion = 'hero' | 'control' | 'status' | 'details';

function placementsFor(capability: PluginCapability, region: PluginRegion): NonNullable<PluginCapability['placements']> {
  return (capability.placements ?? []).filter((p) => p.region === region);
}

function capabilityAvailable(capability: PluginCapability): boolean {
  return capability.available !== false;
}

/** 从插件声明中取得用于宿主装饰的颜色，不依赖任何厂商状态字段名。 */
function declaredAccentColor(device: DeviceState): string | undefined {
  // 主题色是插件明确声明的展示契约。鼠标与接收器同时存在时，插件可稳定
  // 指向鼠标灯光，不受 capability/zone 排列顺序影响。
  for (const capability of device.pluginCapabilities.filter(capabilityAvailable)) {
    const source = capability.metadata.accentSource;
    if (!source) continue;
    const value = readPath(device, source);
    if (typeof value === 'string') return value;
  }
  // 兼容尚未声明 accentSource 的旧插件：优先使用灯光颜色，再回退 DPI。
  for (const capability of device.pluginCapabilities.filter(capabilityAvailable)) {
    const zones = capability.metadata.zones ?? [];
    for (const zone of zones) {
      const color = zone.fields.find((field) => field.format === 'color' || field.editor === 'modal-color');
      if (color) {
        const value = readPath(device, color.source);
        if (typeof value === 'string') return value;
      }
    }
  }
  for (const capability of device.pluginCapabilities.filter(capabilityAvailable)) {
    const layout = capability.metadata.stageLayout;
    if (layout) {
      const stages = readPath(device, layout.colorSource ?? layout.dotsSource) as DpiStage[] | undefined;
      const active = stages?.find((stage) => stage.enabled && stage.active) ?? stages?.find((stage) => stage.enabled);
      if (active?.color) return active.color;
    }
  }
  return undefined;
}

function capabilityRuntimePending(capability: PluginCapability): boolean {
  return capability.metadata._miraRuntimePending === true;
}

function deviceRuntimePending(device: DeviceState): boolean {
  return device.pluginCapabilities.some(capabilityRuntimePending);
}

function PluginIconView({
  name,
  device,
}: {
  name: string | undefined;
  device: DeviceState;
}) {
  if (name === 'battery') {
    const batteryCapability = device.pluginCapabilities.find((capability) => capability.id === 'battery');
    const batteryField = batteryCapability?.metadata.fields?.[0];
    const battery = batteryField ? readPath(device, batteryField.source) as number | undefined : device.battery;
    const charging = device.charging;
    return (
      <BatteryLevelIcon
        className="plugin-battery-icon"
        percentage={battery}
        charging={charging}
      />
    );
  }
  switch (name) {
    case 'gauge':
      return <Gauge weight="regular" />;
    case 'lightbulb':
      return <Lightbulb weight="regular" />;
    case 'profile':
      return <UserCircle weight="regular" />;
    case 'settings':
      return <Gear weight="regular" />;
    case 'timer':
      return <Timer weight="regular" />;
    case 'wave':
      return <WaveSine weight="regular" />;
    case 'info':
    default:
      return <Info weight="regular" />;
  }
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
  return (
    <Modal
      open
      title={title}
      size="small"
      className="edit-modal"
      backdropClassName="edit-modal-backdrop"
      onClose={onClose}
    >
      <form
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
    </Modal>
  );
}

/// 统一字段编辑弹窗。按 field.editor 渲染对应输入控件。
function FieldEditModal({ field, device, writeBusy, onClose, onApply, title, currentValue }: {
  field: PluginField;
  device: DeviceState;
  writeBusy: boolean;
  onClose: () => void;
  onApply: (value: unknown) => void;
  title?: string;
  currentValue?: unknown;
}) {
  const fieldLabel = resolveFieldLabel(field, device, device.pluginId);
  const resolveEditKey = (key: string, params: Record<string, unknown>) => {
    const namespace = device.pluginId && i18n.exists(key, { ns: device.pluginId })
      ? device.pluginId
      : 'translation';
    return i18n.t(key, { ns: namespace, ...params });
  };
  const resolvedTitle = title
    ?? (field.editTitleKey ? resolveEditKey(field.editTitleKey, { label: fieldLabel, field: fieldLabel }) : fieldLabel);
  const editorLabel = field.editLabelKey
    ? resolveEditKey(field.editLabelKey, { label: fieldLabel, field: fieldLabel })
    : fieldLabel;
  const initialValue = currentValue ?? readPath(device, field.source);
  const range = resolveFieldRange(field);
  const options = resolveFieldOptions(field, device);
  const initialDraft = useMemo<unknown>(() => {
    switch (field.editor) {
      case 'modal-select':
        return initialValue != null ? String(initialValue) : (options[0] != null ? String(options[0].value) : '');
      case 'modal-color':
        return typeof initialValue === 'string' ? initialValue : '#000000';
      case 'modal-range':
      case 'modal-number':
        return typeof initialValue === 'number' ? initialValue : Number(initialValue ?? 0);
      case 'modal-gradient':
        return typeof initialValue === 'string' ? initialValue : String(initialValue ?? '');
      default:
        return initialValue;
    }
  }, [field.editor, initialValue, options]);
  const [draftState, setDraftState] = useState(() => ({
    baseline: initialDraft,
    value: initialDraft,
    touched: false,
  }));
  const draft = !draftState.touched && !Object.is(draftState.baseline, initialDraft)
    ? initialDraft
    : draftState.value;
  const updateDraft = (value: unknown) => {
    setDraftState({ baseline: initialDraft, value, touched: true });
  };

  const submitDisabled = useMemo(() => {
    if (writeBusy) return true;
    if (field.editor === 'modal-select') return String(draft) === String(initialValue ?? '');
    return draft === initialValue;
  }, [writeBusy, draft, initialValue, field.editor]);

  const optionLabel = (option: ReturnType<typeof resolveFieldOptions>[number]) => {
    const resolved = resolveLabelKey(option.labelKey, device.pluginId);
    return resolved === String(option.value)
      ? formatFieldValue(option.value, field.format, i18n.t)
      : resolved;
  };

  const handleSubmit = () => {
    if (field.editor === 'modal-select') {
      const option = options.find((opt) => String(opt.value) === String(draft));
      onApply(option ? option.value : draft);
    } else {
      onApply(draft);
    }
  };

  const renderEditor = () => {
    switch (field.editor) {
      case 'modal-select':
        return (
          <label className="edit-field">
            <span>{editorLabel}</span>
            <select
              autoFocus
              aria-label={editorLabel}
              value={String(draft ?? '')}
              disabled={writeBusy}
              onChange={(event) => updateDraft(event.target.value)}
            >
              {options.map((option) => (
                <option key={String(option.value)} value={String(option.value)}>{optionLabel(option)}</option>
              ))}
            </select>
          </label>
        );
      case 'modal-color':
        return (
          <label className="edit-field color-field">
            <span>{i18n.t('common.color')}</span>
            <input
              type="color"
              autoFocus
              aria-label={i18n.t('common.color')}
              value={typeof draft === 'string' ? draft : '#000000'}
              disabled={writeBusy}
              onChange={(event) => updateDraft(event.target.value)}
            />
          </label>
        );
      case 'modal-range':
        return (
          <label className="edit-field range-field">
            <span>{editorLabel}</span>
            <input
              type="range"
              autoFocus
              aria-label={editorLabel}
              value={typeof draft === 'number' ? draft : Number(draft ?? 0)}
              min={range?.min}
              max={range?.max}
              step={range?.step}
              disabled={writeBusy}
              onChange={(event) => updateDraft(Number(event.target.value))}
            />
            <span className="range-value">{typeof draft === 'number' ? draft : Number(draft ?? 0)}</span>
          </label>
        );
      case 'modal-number':
        return (
          <label className="edit-field">
            <span>{editorLabel}</span>
            <input
              type="number"
              autoFocus
              aria-label={editorLabel}
              value={typeof draft === 'number' ? draft : Number(draft ?? 0)}
              min={range?.min}
              max={range?.max}
              step={range?.step}
              disabled={writeBusy}
              onChange={(event) => updateDraft(Number(event.target.value))}
            />
          </label>
        );
      case 'modal-gradient':
        return (
          <label className="edit-field">
            <span>{editorLabel}</span>
            <input
              type="text"
              autoFocus
              aria-label={editorLabel}
              value={typeof draft === 'string' ? draft : String(draft ?? '')}
              disabled={writeBusy}
              onChange={(event) => updateDraft(event.target.value)}
            />
          </label>
        );
      default:
        return <p className="setting-hint">{i18n.t('common.notReported')}</p>;
    }
  };

  return (
    <EditModal
      title={resolvedTitle}
      submitDisabled={submitDisabled}
      onClose={onClose}
      onSubmit={handleSubmit}
    >
      {renderEditor()}
    </EditModal>
  );
}

/// 开关字段（inline-toggle + field.switch）。跟踪上次非 off 值用于恢复。
function SwitchField({ field, device, writeBusy, runMutation }: {
  field: PluginField;
  device: DeviceState;
  writeBusy: boolean;
  runMutation: (mutation: string, params: Record<string, unknown>) => Promise<void>;
}) {
  const sw = field.switch;
  const label = resolveFieldLabel(field, device, device.pluginId);
  const restoreRef = useRef<unknown>(undefined);
  const switchValue = sw ? readPath(device, sw.source) : undefined;

  useEffect(() => {
    if (sw && switchValue !== sw.offValue && switchValue != null) {
      restoreRef.current = switchValue;
    }
  }, [switchValue, sw]);

  if (!sw) return null;

  const mutation = resolveMutation(field.mutation, device.writableMutations);
  const isOn = resolveSwitchState(field, device);
  const writable = Boolean(mutation && !writeBusy);

  const handleClick = () => {
    if (!mutation) return;
    if (isOn) {
      void runMutation(mutation, resolveFieldMutationParams(field, device, sw.offValue));
    } else {
      let restoreValue = restoreRef.current;
      if (restoreValue === undefined && field.options) {
        const nonOff = field.options.find((opt) => opt.value !== sw.offValue);
        restoreValue = nonOff?.value;
      }
      if (restoreValue === undefined && typeof sw.offValue === 'boolean') {
        restoreValue = !sw.offValue;
      }
      if (restoreValue !== undefined) {
        void runMutation(mutation, resolveFieldMutationParams(field, device, restoreValue));
      }
    }
  };

  return (
    <button
      type="button"
      className="lighting-row"
      disabled={!writable}
      onClick={handleClick}
    >
      <span>{label}</span>
      <strong className="lighting-status-value">{isOn ? i18n.t('common.on') : i18n.t('common.off')}</strong>
    </button>
  );
}

/// 按 field.editor 渲染字段控件。声明式，不含字段级特殊分支。
function FieldRenderer({ field, device, writeBusy, runMutation }: {
  field: PluginField;
  device: DeviceState;
  writeBusy: boolean;
  runMutation: (mutation: string, params: Record<string, unknown>) => Promise<void>;
}) {
  const [editing, setEditing] = useState(false);

  if (!resolveVisibleWhen(field.visibleWhen, device)) return null;

  const mutation = resolveMutation(field.mutation, device.writableMutations);
  const writable = Boolean(mutation && !writeBusy);
  const label = resolveFieldLabel(field, device, device.pluginId);
  const value = readPath(device, field.source);
  const valueLabel = resolveFieldValueLabel(field, device, device.pluginId);

  const applyMutation = (mutation: string, params: Record<string, unknown>) => {
    void runMutation(mutation, params);
  };

  switch (field.editor) {
    case 'inline-toggle':
      if (field.switch) {
        return <SwitchField field={field} device={device} writeBusy={writeBusy} runMutation={runMutation} />;
      }
      return (
        <>
          <span>{label}</span>
          <button
            type="button"
            className={`plugin-toggle ${value === true ? 'active' : ''}`}
            aria-pressed={value === true}
            disabled={!writable}
            onClick={() => mutation && applyMutation(mutation, resolveFieldMutationParams(field, device, value !== true))}
          >{value === true ? i18n.t('common.on') : i18n.t('common.off')}</button>
        </>
      );

    case 'inline-segmented': {
      const options = resolveFieldOptions(field, device);
      const activeOptionIndex = Math.max(options.findIndex((option) => value === option.value), 0);
      return (
        <>
          <span>{label}</span>
          <div
            className="plugin-segmented segmented-slider"
            role="group"
            aria-label={label}
            data-active-index={activeOptionIndex}
            style={{
              gridTemplateColumns: `repeat(${options.length}, minmax(0, 1fr))`,
              ...segmentedIndicatorStyle(options.length, activeOptionIndex, { gap: 6, padding: 6 }),
            }}
          >
            {options.map((option) => (
              <button
                key={String(option.value)}
                type="button"
                className={value === option.value ? 'active' : ''}
                aria-pressed={value === option.value}
                disabled={!writable}
                onClick={() => mutation && applyMutation(mutation, resolveFieldMutationParams(field, device, option.value))}
              >{resolveLabelKey(option.labelKey, device.pluginId)}</button>
            ))}
          </div>
        </>
      );
    }

    case 'inline-value':
      return (
        <>
          <span>{label}</span>
          <FormattedValue value={value} format={field.format} label={valueLabel} className="plugin-current-value" />
        </>
      );

    case 'inline-action':
      return (
        <button
          type="button"
          className="plugin-action"
          disabled={!writable}
          onClick={() => mutation && applyMutation(mutation, resolveFieldParams(field, device))}
        >{label || i18n.t('common.execute')}</button>
      );

    case 'modal-select':
    case 'modal-color':
    case 'modal-range':
    case 'modal-number':
    case 'modal-gradient':
      return (
        <>
          <button
            type="button"
            className="lighting-row"
            disabled={!writable}
            onClick={() => {
              invoke('device_refresh_quick').catch(() => {});
              setEditing(true);
            }}
          >
            <span>{label}</span>
            <FormattedValue value={value} format={field.format} label={valueLabel} />
          </button>
          {editing && (
            <FieldEditModal
              field={field}
              device={device}
              writeBusy={writeBusy}
              onClose={() => setEditing(false)}
              onApply={(v) => {
                if (mutation) applyMutation(mutation, resolveFieldMutationParams(field, device, v));
                setEditing(false);
              }}
            />
          )}
        </>
      );

    case 'static-readonly':
      return (
        <>
          <span>{label}</span>
          <FormattedValue value={value} format={field.format} label={valueLabel} className="plugin-current-value" />
        </>
      );

    default:
      console.warn('Unknown field editor:', field.editor);
      return (
        <>
          <span>{label}</span>
          <FormattedValue value={value} format={field.format} label={valueLabel} className="plugin-current-value" />
        </>
      );
  }
}

/**
 * 通用数值指标读数。任何以 hertz 声明、且只有一个可编辑字段的能力都会
 * 使用该布局；视觉层只依据插件提供的格式和 placement，不依赖能力或厂商名。
 */
function MetricField({ capability, field, device, writeBusy, runMutation }: {
  capability: PluginCapability;
  field: PluginField;
  device: DeviceState;
  writeBusy: boolean;
  runMutation: (mutation: string, params: Record<string, unknown>) => Promise<void>;
}) {
  const [editing, setEditing] = useState(false);
  const mutation = resolveMutation(field.mutation, device.writableMutations);
  const writable = Boolean(mutation && !writeBusy);
  const value = readPath(device, field.source);
  const hasHertzValue = field.format === 'hertz' && typeof value === 'number';
  const valueText = hasHertzValue ? String(value) : formatFieldValue(value, field.format, i18n.t);

  return (
    <div className="control-reading mode-reading metric-reading">
      <div className="metric-reading-heading">
        <WaveSine weight="regular" />
        <span>{i18n.t('dashboard.currentPollingRate')}</span>
      </div>
      <button
        type="button"
        className="metric-reading-value editable-reading"
        aria-label={hasHertzValue
          ? i18n.t('dashboard.currentPollingRateEdit', { value: valueText })
          : i18n.t('dashboard.pollingRateNotReportedEdit')}
        disabled={!writable}
        onClick={() => {
          invoke('device_refresh_quick').catch(() => {});
          setEditing(true);
        }}
      >
        <strong>{valueText}</strong>
        {hasHertzValue && <em>Hz</em>}
      </button>
      <CapabilitySummary capability={capability} device={device} />
      {editing && (
        <FieldEditModal
          field={field}
          device={device}
          writeBusy={writeBusy}
          title={i18n.t('dashboard.setPollingRateTitle')}
          onClose={() => setEditing(false)}
          onApply={(nextValue) => {
            if (mutation) void runMutation(mutation, resolveFieldMutationParams(field, device, nextValue));
            setEditing(false);
          }}
        />
      )}
    </div>
  );
}

/**
 * 旧版普通设置块的表现壳层。
 *
 * 设备的字段、可写 mutation 和选项仍完全来自插件声明；这里仅固定普通设置
 * 在界面中的图标、标题和可编辑读数样式，避免解耦后被误渲染成灯光卡片。
 */
function GenericFieldControl({ field, device, writeBusy, runMutation }: {
  field: PluginField;
  device: DeviceState;
  writeBusy: boolean;
  runMutation: (mutation: string, params: Record<string, unknown>) => Promise<void>;
}) {
  const [editing, setEditing] = useState(false);
  const restoreRef = useRef<unknown>(undefined);

  const mutation = resolveMutation(field.mutation, device.writableMutations);
  const writable = Boolean(mutation && !writeBusy);
  const value = readPath(device, field.source);
  const label = resolveFieldLabel(field, device, device.pluginId);
  const valueLabel = resolveFieldValueLabel(field, device, device.pluginId);
  const switchValue = field.switch ? readPath(device, field.switch.source) : undefined;

  useEffect(() => {
    if (field.switch && switchValue !== field.switch.offValue && switchValue != null) {
      restoreRef.current = switchValue;
    }
  }, [field.switch, switchValue]);

  if (!resolveVisibleWhen(field.visibleWhen, device)) return null;

  const apply = (nextValue: unknown) => {
    if (mutation) void runMutation(mutation, resolveFieldMutationParams(field, device, nextValue));
  };

  switch (field.editor) {
    case 'inline-toggle': {
      const isOn = field.switch ? resolveSwitchState(field, device) : value === true;
      return (
        <button
          type="button"
          className={`plugin-toggle ${isOn ? 'active' : ''}`}
          aria-pressed={isOn}
          disabled={!writable}
          onClick={() => {
            if (!field.switch) {
              apply(!isOn);
              return;
            }
            if (isOn) {
              apply(field.switch.offValue);
              return;
            }
            const restored = restoreRef.current
              ?? field.options?.find((option) => option.value !== field.switch?.offValue)?.value;
            if (restored !== undefined) apply(restored);
          }}
        >{isOn ? i18n.t('common.on') : i18n.t('common.off')}</button>
      );
    }

    case 'inline-segmented': {
      const options = resolveFieldOptions(field, device);
      const activeOptionIndex = Math.max(options.findIndex((option) => value === option.value), 0);
      return (
        <div
          className="plugin-segmented segmented-slider"
          role="group"
          aria-label={label}
          data-active-index={activeOptionIndex}
          style={{
            gridTemplateColumns: `repeat(${options.length}, minmax(0, 1fr))`,
            ...segmentedIndicatorStyle(options.length, activeOptionIndex, { gap: 6, padding: 6 }),
          }}
        >
          {options.map((option) => (
            <button
              key={String(option.value)}
              type="button"
              className={value === option.value ? 'active' : ''}
              aria-pressed={value === option.value}
              disabled={!writable}
              onClick={() => apply(option.value)}
            >{resolveLabelKey(option.labelKey, device.pluginId)}</button>
          ))}
        </div>
      );
    }

    case 'inline-action':
      return (
        <button type="button" className="plugin-action" disabled={!writable} onClick={() => mutation && void runMutation(mutation, resolveFieldParams(field, device))}>
          {label || i18n.t('common.execute')}
        </button>
      );

    case 'modal-select':
    case 'modal-color':
    case 'modal-range':
    case 'modal-number':
    case 'modal-gradient':
      return (
        <>
          <button
            type="button"
            className="plugin-value-button editable-reading"
            aria-label={`${label}：${valueLabel ?? formatFieldValue(value, field.format, i18n.t)}，点击编辑`}
            disabled={!writable}
            onClick={() => {
              invoke('device_refresh_quick').catch(() => {});
              setEditing(true);
            }}
          >
            {(field.editor === 'modal-color' || field.format === 'color') && typeof value === 'string' && <i style={{ '--light-color': value } as React.CSSProperties} />}
            <FormattedValue value={value} format={field.format} label={valueLabel} />
          </button>
          {editing && (
            <FieldEditModal
              field={field}
              device={device}
              writeBusy={writeBusy}
              onClose={() => setEditing(false)}
              onApply={(nextValue) => {
                apply(nextValue);
                setEditing(false);
              }}
            />
          )}
        </>
      );

    case 'inline-value':
    case 'static-readonly':
    default:
      return <FormattedValue value={value} format={field.format} label={valueLabel} className="plugin-current-value" />;
  }
}

function GenericCapabilityControl({ capability, device, writeBusy, runMutation }: {
  capability: PluginCapability;
  device: DeviceState;
  writeBusy: boolean;
  runMutation: (mutation: string, params: Record<string, unknown>) => Promise<void>;
}) {
  const fields = (capability.metadata.fields ?? []).filter((field) => resolveVisibleWhen(field.visibleWhen, device));
  const label = resolveLabelKey(capability.labelKey, device.pluginId);

  return (
    <div className="control-reading mode-reading plugin-control-reading">
      <UserCircle weight="regular" />
      <span>{label}</span>
      {fields.map((field) => (
        <GenericFieldControl
          key={field.id}
          field={field}
          device={device}
          writeBusy={writeBusy}
          runMutation={runMutation}
        />
      ))}
      <CapabilitySummary capability={capability} device={device} />
    </div>
  );
}

/// DPI 分档布局。读取 stageLayout 声明渲染档位点与值按钮。
function StageLayout({ capability, device, writeBusy, runMutation }: {
  capability: PluginCapability;
  device: DeviceState;
  writeBusy: boolean;
  runMutation: (mutation: string, params: Record<string, unknown>) => Promise<void>;
}) {
  const layout = resolveStageLayout(capability);
  const [editingStage, setEditingStage] = useState<number | null>(null);
  const runtimePending = capabilityRuntimePending(capability);

  if (!layout) return null;

  const allStages = (readPath(device, layout.dotsSource) as DpiStage[] | undefined) ?? [];
  const stages = allStages.filter((stage) => stage.enabled);
  const displayedStages = stages.slice(0, 8);
  const current = stages.find((stage) => stage.active);
  const currentStageNumber = Math.max(1, stages.findIndex((stage) => stage.active) + 1);
  const activeDpi = current?.value ?? stages[0]?.value ?? 0;

  const selectMutation = resolveMutation(layout.selectMutation, device.writableMutations);
  const setMutation = resolveMutation(layout.setMutation, device.writableMutations);
  const selectWritable = Boolean(selectMutation);
  const setWritable = Boolean(setMutation);
  const range: RangeSpec = layout.range;

  const stageField: PluginField = {
    id: 'stage-value',
    source: layout.valueSource,
    mutation: setMutation,
    param: layout.valueParam ?? 'value',
    editor: 'modal-number',
    range,
    labelKey: 'dashboard.dpiValue',
    editLabelKey: 'dashboard.dpiValue',
  };

  return (
    <>
      <button
        type="button"
        className="primary-reading editable-reading"
        aria-label={activeDpi ? i18n.t('dashboard.currentDpiEdit', { value: activeDpi }) : i18n.t('dashboard.dpiNotReported')}
        disabled={writeBusy || !setWritable || !activeDpi}
        onClick={() => {
          if (!activeDpi) return;
          invoke('device_refresh_quick').catch(() => {});
          setEditingStage(currentStageNumber);
        }}
      >
        <LiveValue text={String(activeDpi || (runtimePending ? '—' : i18n.t('common.notReported')))} /><em>DPI</em>
      </button>
      <div className={`dpi-scale ${runtimePending ? 'is-pending' : 'is-ready'}`} aria-label={i18n.t('dashboard.dpiStages')} style={{ '--stage-count': Math.max(displayedStages.length, 1) } as React.CSSProperties}>
        <div className="dpi-stage-placeholders" aria-hidden="true">
          {Array.from({ length: 5 }, (_, index) => <span key={index} className="dpi-stage-placeholder" />)}
        </div>
        <div className="dpi-stage-values">
          {displayedStages.map((stage, index) => {
            const stageNumber = index + 1;
            return (
              <div
                key={`${index}-${stage.value}`}
                className="dpi-stage-item"
                style={{ '--dpi-stage-delay': `${60 + index * 26}ms` } as React.CSSProperties}
              >
              <button
                type="button"
                className={`dpi-stage-dot ${stage.active ? 'active' : ''}`}
                aria-pressed={stage.active}
                disabled={writeBusy || !selectWritable}
                onClick={() => selectMutation && runMutation(selectMutation, { [layout.selectParam ?? 'value']: stageNumber })}
                aria-label={i18n.t('dashboard.switchToStage', { stage: stageNumber })}
              >
                <i style={{ '--stage-source-color': pastelDisplayColor(stage.color) } as React.CSSProperties} />
              </button>
              <button
                type="button"
                className="dpi-stage-value"
                disabled={writeBusy || !setWritable}
                onClick={() => {
                  invoke('device_refresh_quick').catch(() => {});
                  setEditingStage(stageNumber);
                }}
                aria-label={i18n.t('dashboard.editStageDpi', { stage: stageNumber })}
              >
                {stage.value}
              </button>
              </div>
            );
          })}
        </div>
      </div>
      {!runtimePending && displayedStages.length === 0 && <p className="setting-hint">{i18n.t('dashboard.noDpiStages')}</p>}
      {!setWritable && displayedStages.length > 0 && <p className="setting-hint">{i18n.t('dashboard.dpiWriteUnavailable')}</p>}
      {editingStage !== null && (
        <FieldEditModal
          field={stageField}
          device={device}
          writeBusy={writeBusy}
          title={i18n.t('dashboard.editStageDpi', { stage: editingStage })}
          currentValue={stages[editingStage - 1]?.value ?? activeDpi}
          onClose={() => setEditingStage(null)}
          onApply={(value) => {
            if (setMutation) {
              void runMutation(setMutation, {
                [layout.stageParam ?? 'stage']: editingStage,
                [layout.valueParam ?? 'value']: value,
              });
            }
            setEditingStage(null);
          }}
        />
      )}
    </>
  );
}

/// 灯光区域渲染。多区域时显示子标签页，单区域时直接渲染字段网格。
function ZoneRenderer({ capability, device, writeBusy, runMutation }: {
  capability: PluginCapability;
  device: DeviceState;
  writeBusy: boolean;
  runMutation: (mutation: string, params: Record<string, unknown>) => Promise<void>;
}) {
  const zones = resolveZones(capability, device);
  const [activeZoneId, setActiveZoneId] = useState<string>('');
  const [editingColorZoneId, setEditingColorZoneId] = useState<string>();

  // 灯光区域标题（鼠标灯光/接收器灯光）的淡入淡出状态机。
  // Hooks 必须在条件返回之前调用，所以 activeZone 在此安全派生。
  const activeZone = zones.length > 0 ? (zones.find((z) => z.id === activeZoneId) ?? zones[0]) : undefined;
  const currentLabel = activeZone ? resolveLabelKey(activeZone.labelKey, device.pluginId) : '';
  const [displayedLabel, setDisplayedLabel] = useState(currentLabel);
  const [titlePhase, setTitlePhase] = useState<'in' | 'out' | 'waiting'>('waiting');
  const previousLabelRef = useRef<string | undefined>(undefined);

  useEffect(() => {
    const prev = previousLabelRef.current;
    previousLabelRef.current = currentLabel;
    if (prev === undefined || prev === currentLabel) {
      // 初次挂载（含 StrictMode 重复挂载）：在子块淡入动画的末尾阶段淡入标题
      //（220ms 开始，340ms 完成，早于子块完成时间 345~390ms，让标题在子块
      // 淡入播放完前就出现）。StrictMode 下 useEffect 执行两次，第一次会把
      // previousLabelRef 设为 currentLabel 并被清理掉定时器，第二次 prev ===
      // currentLabel 也走此分支，保证定时器被重新建立。
      setDisplayedLabel(currentLabel);
      setTitlePhase('waiting');
      const inTimer = window.setTimeout(() => setTitlePhase('in'), 220);
      return () => window.clearTimeout(inTimer);
    }
    // 切换区域：先在子块淡出的中间点淡出（90ms ≈ 子块淡入时长 180ms 的一半），
    // 然后在子块淡入动画的末尾阶段（220ms）淡入新标题，340ms 完成。
    setTitlePhase('out');
    const switchTimer = window.setTimeout(() => {
      setDisplayedLabel(currentLabel);
      setTitlePhase('waiting');
    }, 90);
    const inTimer = window.setTimeout(() => setTitlePhase('in'), 220);
    return () => {
      window.clearTimeout(switchTimer);
      window.clearTimeout(inTimer);
    };
  }, [currentLabel]);

  if (zones.length === 0 || !activeZone) return null;

  const activeZoneIndex = Math.max(zones.findIndex((zone) => zone.id === activeZone.id), 0);
  const multipleZones = zones.length > 1;

  const colorField = activeZone.fields.find((f) => f.editor === 'modal-color')
    ?? activeZone.fields.find((f) => f.format === 'color');
  const zoneColor = colorField ? readPath(device, colorField.source) as string | undefined : undefined;
  const colorMutation = colorField?.editor === 'modal-color'
    ? resolveMutation(colorField.mutation, device.writableMutations)
    : undefined;
  const colorWritable = Boolean(
    colorField
    && resolveVisibleWhen(colorField.visibleWhen, device)
    && colorMutation
    && !writeBusy,
  );
  const colorLabel = colorField
    ? resolveFieldLabel(colorField, device, device.pluginId)
    : i18n.t('common.color');
  // 主题来源区域继续沿用全局主题色；附属灯光区域则只在当前分段滑块内
  // 使用自己的灯光颜色。判断依据来自插件声明，不依赖鼠标/接收器 id。
  const usesThemeAccent = capability.metadata.accentSource
    ? colorField?.source === capability.metadata.accentSource
    : activeZone.id === zones[0].id;
  const tabAccent = usesThemeAccent ? 'var(--accent)' : zoneColor ?? 'var(--accent)';

  const visibleFields = activeZone.fields.filter((f) => resolveVisibleWhen(f.visibleWhen, device));
  // 条件显示的次级区域通常是接收器等附属对象；字段较多时使用与旧界面一致
  // 的紧凑密度。这里仅依赖 zone 的声明形态，不依赖 zone id。
  const compactDetailGrid = Boolean(activeZone.visibleWhen) && visibleFields.length >= 5;

  return (
    <>
      {multipleZones && (
        <div
          className="lighting-sub-tabs segmented-slider"
          role="tablist"
          aria-label={i18n.t('dashboard.lightingTarget')}
          data-active-index={activeZoneIndex}
          style={{
            gridTemplateColumns: `repeat(${zones.length}, minmax(0, 1fr))`,
            ...segmentedIndicatorStyle(zones.length, activeZoneIndex, { accent: tabAccent, gap: 3, padding: 3 }),
          } as React.CSSProperties}
        >
          {zones.map((zone) => (
            <button
              key={zone.id}
              role="tab"
              aria-selected={activeZone.id === zone.id}
              className={activeZone.id === zone.id ? 'active' : ''}
              onClick={() => setActiveZoneId(zone.id)}
            >{resolveLabelKey(zone.labelKey, device.pluginId)}</button>
          ))}
        </div>
      )}
      <button
        type="button"
        className="lighting-swatch"
        style={{ '--light-color': zoneColor ?? '#b87ab0' } as React.CSSProperties}
        aria-label={colorLabel}
        title={colorLabel}
        disabled={!colorWritable}
        onClick={() => {
          invoke('device_refresh_quick').catch(() => {});
          setEditingColorZoneId(activeZone.id);
        }}
      />
      <div className="lighting-sections" aria-label={i18n.t('dashboard.lightingGroups')}>
        <div className={`lighting-group lighting-group-${activeZone.id}${compactDetailGrid ? ' is-compact' : ''}`}>
          <p className="lighting-group-title" data-title-phase={titlePhase}>{displayedLabel}</p>
          <div
            className={`lighting-rows${compactDetailGrid ? ' is-compact' : ''}`}
            style={{ gridTemplateColumns: `repeat(${Math.max(visibleFields.length, 1)}, minmax(0, 1fr))` }}
          >
            {visibleFields.map((field) => (
              <div
                key={`${activeZone.id}:${field.id}`}
                className="lighting-row-slot secondary-control-item"
                style={secondaryRevealStyle(`${capability.id}:${activeZone.id}:${field.id}`)}
              >
                <FieldRenderer
                  field={field}
                  device={device}
                  writeBusy={writeBusy}
                  runMutation={runMutation}
                />
              </div>
            ))}
          </div>
        </div>
      </div>
      {colorField && colorMutation && editingColorZoneId === activeZone.id && (
        <FieldEditModal
          key={`${activeZone.id}:${colorField.id}`}
          field={colorField}
          device={device}
          writeBusy={writeBusy}
          onClose={() => setEditingColorZoneId(undefined)}
          onApply={(value) => {
            void runMutation(colorMutation, resolveFieldMutationParams(colorField, device, value));
            setEditingColorZoneId(undefined);
          }}
        />
      )}
    </>
  );
}

/// 状态栏条目。读取 statusDisplay 声明渲染图标+标签+值。
function StatusItem({ capability, device, placement, onClick }: {
  capability: PluginCapability;
  device: DeviceState;
  placement: PluginCapabilityPlacement;
  onClick: (() => void) | undefined;
}) {
  const display = resolveStatusDisplay(capability);
  if (!display) return null;

  const requestedField = resolveStatusField(capability, display.onClickField, device);
  const preferredField = display.onClickField
    ? ([...(capability.metadata.fields ?? []), ...(capability.metadata.zones ?? []).flatMap((zone) => zone.fields)]
      .find((field) => field.id === display.onClickField))
    : undefined;
  const valueSource = requestedField && requestedField !== preferredField
    ? requestedField.source
    : display.valueSource;
  const value = readPath(device, valueSource);
  const capabilityLabel = resolveLabelKey(capability.labelKey, device.pluginId);
  const fieldLabel = requestedField ? resolveFieldLabel(requestedField, device, device.pluginId) : '';
  const label = display.labelKey
    ? resolveLabelKey(display.labelKey, device.pluginId)
    : fieldLabel || capabilityLabel;

  let valueText: string;
  if (display.valueOptions) {
    const option = display.valueOptions.find((opt) => opt.value === value);
    valueText = option ? resolveLabelKey(option.labelKey, device.pluginId) : formatFieldValue(value, display.valueFormat, i18n.t);
  } else {
    valueText = formatFieldValue(value, display.valueFormat, i18n.t);
  }
  if (capabilityRuntimePending(capability) && (value === null || value === undefined || value === '')) {
    valueText = '…';
  }

  const isColor = display.valueFormat === 'color' || valueLooksColor(value);

  const content = (
    <>
      <PluginIconView name={placement.icon} device={device} />
      <span>{label}<LiveValue text={valueText} /></span>
      {isColor && typeof value === 'string' && <i style={{ '--light-color': value } as React.CSSProperties} />}
    </>
  );

  return onClick
    ? <button type="button" onClick={onClick}>{content}</button>
    : <div>{content}</div>;
}

/// capability.control 组件级分派。这是唯一允许 capability.control === 判断的地方。
function CapabilityRouter({ capability, device, writeBusy, runMutation }: {
  capability: PluginCapability;
  device: DeviceState;
  writeBusy: boolean;
  runMutation: (mutation: string, params: Record<string, unknown>) => Promise<void>;
}) {
  if (capability.control === 'DpiStages') {
    return (
      <div className="control-reading dpi-reading">
        <StageLayout capability={capability} device={device} writeBusy={writeBusy} runMutation={runMutation} />
        <CapabilitySummary capability={capability} device={device} />
      </div>
    );
  }
  if (capability.control === 'LightingZone') {
    return (
      <div className="control-reading mode-reading lighting-reading">
        <ZoneRenderer capability={capability} device={device} writeBusy={writeBusy} runMutation={runMutation} />
        <CapabilitySummary capability={capability} device={device} />
      </div>
    );
  }
  const fields = (capability.metadata.fields ?? []).filter((f) => resolveVisibleWhen(f.visibleWhen, device));
  const metricField = fields.length === 1 && fields[0].format === 'hertz' ? fields[0] : undefined;
  if (metricField) {
    return <MetricField capability={capability} field={metricField} device={device} writeBusy={writeBusy} runMutation={runMutation} />;
  }
  return <GenericCapabilityControl capability={capability} device={device} writeBusy={writeBusy} runMutation={runMutation} />;
}

function DeviceDetails({ device, onClose }: { device: DeviceState; onClose: () => void }) {
  const detailOrder = new Map<string, number>();
  for (const capability of device.pluginCapabilities) {
    const placement = placementsFor(capability, 'details')[0];
    if (placement) {
      detailOrder.set(capability.id, placement.order);
    }
  }
  const groups = Object.entries(device.capabilities)
    .filter(([, fields]) => fields && Object.keys(fields).length > 0)
    .sort(([a], [b]) => (detailOrder.get(a) ?? 10_000) - (detailOrder.get(b) ?? 10_000));
  return (
    <Modal
      open
      title={i18n.t('dashboard.allReadInfo')}
      size="medium"
      className="device-details"
      backdropClassName="details-backdrop"
      onClose={onClose}
    >
      <section aria-labelledby="device-details-title">
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
                {Object.entries(fields).map(([key, value]) => {
                  const valueLabel = resolveDetailValueLabel(group, key, device);
                  return (
                    <div key={key}>
                      <dt>{capabilityFieldLabel(key)}</dt>
                      <dd><FormattedValue value={value} label={valueLabel} /></dd>
                    </div>
                  );
                })}
              </dl>
            </section>
          )) : <p className="setting-hint">{i18n.t('dashboard.noCapabilities')}</p>}
        </div>
      </section>
    </Modal>
  );
}

type SharedControlMetric = {
  label: string;
  targetSelector: string;
  text: string;
  unit: string;
  variant: 'dpi' | 'hertz';
};

type SharedControlSurface = {
  kind: 'summary' | 'lighting';
  targetSelector: string;
};

function sharedControlMetric(capabilities: PluginCapability[], device: DeviceState): SharedControlMetric | undefined {
  for (const capability of capabilities) {
    const layout = resolveStageLayout(capability);
    if (layout) {
      const stages = ((readPath(device, layout.dotsSource) as DpiStage[] | undefined) ?? [])
        .filter((stage) => stage.enabled);
      const activeDpi = stages.find((stage) => stage.active)?.value ?? stages[0]?.value;
      const text = activeDpi
        ? String(activeDpi)
        : capabilityRuntimePending(capability) ? '—' : i18n.t('common.notReported');
      return {
        label: activeDpi
          ? i18n.t('dashboard.currentDpiEdit', { value: activeDpi })
          : i18n.t('dashboard.dpiNotReported'),
        targetSelector: '.dpi-reading > .primary-reading',
        text,
        unit: 'DPI',
        variant: 'dpi',
      };
    }

    const fields = (capability.metadata.fields ?? [])
      .filter((field) => resolveVisibleWhen(field.visibleWhen, device));
    const metricField = fields.length === 1 && fields[0].format === 'hertz' ? fields[0] : undefined;
    if (metricField) {
      const value = readPath(device, metricField.source);
      const hasHertzValue = typeof value === 'number';
      const text = hasHertzValue ? String(value) : formatFieldValue(value, metricField.format, i18n.t);
      return {
        label: hasHertzValue
          ? i18n.t('dashboard.currentPollingRateEdit', { value: text })
          : i18n.t('dashboard.pollingRateNotReportedEdit'),
        targetSelector: '.metric-reading > .metric-reading-value',
        text,
        unit: hasHertzValue ? 'Hz' : '',
        variant: 'hertz',
      };
    }
  }
  return undefined;
}

function sharedControlSurface(capabilities: PluginCapability[], device: DeviceState): SharedControlSurface | undefined {
  for (const capability of capabilities) {
    if (resolveZones(capability, device).length > 0) {
      return { kind: 'lighting', targetSelector: '.lighting-reading .lighting-group' };
    }
    if ((capability.metadata.summary ?? []).length > 0) {
      return { kind: 'summary', targetSelector: '.metric-reading > .capability-summary' };
    }
  }
  return undefined;
}

function useControlTargetPosition(
  stageRef: React.RefObject<HTMLElement | null>,
  targetSelector: string | undefined,
  layerRef: React.RefObject<HTMLElement | null>,
  transitionMode: 'morph' | 'snap',
) {
  const previousTargetRef = useRef<string | undefined>(undefined);

  useLayoutEffect(() => {
    const previousTarget = previousTargetRef.current;
    previousTargetRef.current = targetSelector;
    if (!targetSelector) return;

    const stage = stageRef.current;
    const target = stage?.querySelector<HTMLElement>(targetSelector);
    const layer = layerRef.current;
    if (!stage || !target || !layer) return;

    const shouldSnap = layer.dataset.positioned !== 'true'
      || (transitionMode === 'snap' && previousTarget === undefined);
    let revealFrame = 0;

    const measure = () => {
      const stageRect = stage.getBoundingClientRect();
      const targetRect = target.getBoundingClientRect();
      layer.style.width = `${targetRect.width}px`;
      layer.style.height = `${targetRect.height}px`;
      layer.style.transform = `translate3d(${targetRect.left - stageRect.left}px, ${targetRect.top - stageRect.top}px, 0)`;
      layer.dataset.positioned = 'true';
    };

    if (shouldSnap) {
      layer.dataset.geometryReady = 'false';
      layer.dataset.repositioning = 'true';
      layer.getBoundingClientRect();
    } else {
      layer.dataset.geometryReady = 'true';
    }
    measure();
    if (shouldSnap) {
      layer.getBoundingClientRect();
      revealFrame = window.requestAnimationFrame(() => {
        layer.dataset.geometryReady = 'true';
        layer.dataset.repositioning = 'false';
      });
    }

    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(measure) : undefined;
    observer?.observe(stage);
    observer?.observe(target);
    window.addEventListener('resize', measure);
    return () => {
      window.cancelAnimationFrame(revealFrame);
      observer?.disconnect();
      window.removeEventListener('resize', measure);
      layer.dataset.repositioning = 'false';
    };
  }, [layerRef, stageRef, targetSelector, transitionMode]);
}

function SharedControlMetricLayer({
  contextKey,
  layerRef,
  metric,
  sync,
}: {
  contextKey: string;
  layerRef: React.RefObject<HTMLDivElement | null>;
  metric: SharedControlMetric | undefined;
  sync: 'metric' | 'surface';
}) {
  return (
    <div
      ref={layerRef}
      className="shared-control-metric"
      data-sync={sync}
      data-variant={metric?.variant ?? 'hertz'}
      data-visible={metric ? 'true' : 'false'}
      data-positioned="false"
      aria-hidden="true"
    >
      <MorphingMetricValue
        active={Boolean(metric)}
        contextKey={contextKey}
        text={metric?.text ?? ''}
        unit={metric?.unit ?? ''}
        variant={metric?.variant ?? 'hertz'}
      />
    </div>
  );
}

function SharedControlSurfaceLayer({
  layerRef,
  surface,
}: {
  layerRef: React.RefObject<HTMLDivElement | null>;
  surface: SharedControlSurface | undefined;
}) {
  return (
    <div
      ref={layerRef}
      className="shared-control-surface"
      data-kind={surface?.kind ?? 'summary'}
      data-visible={surface ? 'true' : 'false'}
      data-positioned="false"
      aria-hidden="true"
    />
  );
}

function SharedPollingContextLayer({
  layerRef,
  sync,
  visible,
}: {
  layerRef: React.RefObject<HTMLDivElement | null>;
  sync: 'metric' | 'surface';
  visible: boolean;
}) {
  return (
    <div
      ref={layerRef}
      className="shared-control-context"
      data-positioned="false"
      data-sync={sync}
      data-visible={visible ? 'true' : 'false'}
      aria-hidden="true"
    >
      <div className="shared-control-context-content">
        <WaveSine weight="regular" />
        <span>{i18n.t('dashboard.currentPollingRate')}</span>
      </div>
    </div>
  );
}

function resolveContextMotionSync(
  currentHasMetric: boolean,
  currentHasSurface: boolean,
  targetHasMetric: boolean,
  targetHasSurface: boolean,
): 'metric' | 'surface' {
  if (currentHasMetric && targetHasMetric) return 'metric';
  if (currentHasSurface || targetHasSurface) return 'surface';
  return 'metric';
}

function Dashboard({
  device,
  deviceEntries,
  onDeviceChange,
  onDeviceSelect,
  onOpenBatteryUsage,
  pluginLocaleRevision,
  demoMode,
}: {
  device: DeviceState;
  deviceEntries: DeviceSnapshotEntry[];
  onDeviceChange: (device: DeviceState) => void;
  onDeviceSelect: (deviceKey: string) => void;
  onOpenBatteryUsage: () => void;
  pluginLocaleRevision: number;
  demoMode: boolean;
}) {
  const { t } = useTranslation();
  const [mode, setMode] = useState<ControlMode>('');
  const [contextMotionSync, setContextMotionSync] = useState<'metric' | 'surface'>('metric');
  const [previewMessage, setPreviewMessage] = useState('');
  const [showDetails, setShowDetails] = useState(false);
  const [showBatteries, setShowBatteries] = useState(false);
  const [showDeviceSwitcher, setShowDeviceSwitcher] = useState(false);
  const batteryControlRef = useRef<HTMLDivElement>(null);
  const deviceSwitcherRef = useRef<HTMLDivElement>(null);
  const controlStageLayersRef = useRef<HTMLDivElement>(null);
  const sharedContextLayerRef = useRef<HTMLDivElement>(null);
  const sharedMetricLayerRef = useRef<HTMLDivElement>(null);
  const sharedSurfaceLayerRef = useRef<HTMLDivElement>(null);
  const [writeBusy, setWriteBusy] = useState(false);
  const [editingField, setEditingField] = useState<{ capability: PluginCapability; field: PluginField } | null>(null);

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
      if (demoMode) {
        // 演示模式：直接在前端模拟写入，不调用 Tauri device_mutate。
        // 参数变化立即反映在 UI 上，并保留「搞定啦」成功通知。
        const nextDevice = simulateDemoMutation(device, mutation, params);
        onDeviceChange(nextDevice);
        setPreviewMessage('');
        notifySuccess(i18n.t('dashboard.writeConfirmed'));
        return;
      }
      const snapshot = await invoke<DeviceSnapshot>('device_mutate', { mutation, params });
      onDeviceChange(snapshotToState(snapshot));
      setPreviewMessage('');
      notifySuccess(i18n.t('dashboard.writeConfirmed'));
    } catch (error) {
      setPreviewMessage('');
      const errorString = String(error);
      if (errorString.includes('is not available on this device')) {
        notifyError(
          i18n.t('notification.mutationUnavailable'),
          i18n.t('notification.mutationUnavailableBody'),
        );
      } else {
        notifyError(i18n.t('notification.writeFailed'), i18n.t('notification.writeFailedBody', { error: errorString }));
      }
      invoke('device_refresh').catch(() => {});
    } finally {
      setWriteBusy(false);
    }
  };

  const controls = useMemo(() => {
    const controlPlacements = device.pluginCapabilities
      .filter(capabilityAvailable)
      .flatMap((capability) => placementsFor(capability, 'control').map((placement) => ({ capability, placement })))
      .filter(({ capability }) => resolveVisibleWhen(capability.metadata.visibleWhen, device))
      .sort((a, b) => a.placement.order - b.placement.order);
    const groups = new Map<string, { id: string; label: string; icon: string | undefined; capabilities: PluginCapability[] }>();
    for (const { capability, placement } of controlPlacements) {
      const id = placement.group || capability.id;
      const existing = groups.get(id);
      if (existing) {
        existing.capabilities.push(capability);
      } else {
        groups.set(id, {
          id,
          label: resolveLabelKey(capability.labelKey, device.pluginId),
          icon: placement.icon,
          capabilities: [capability],
        });
      }
    }
    return [...groups.values()]
      .slice(0, MAX_CONTROL_GROUPS)
      .map((group) => ({
        ...group,
        hasMetric: Boolean(sharedControlMetric(group.capabilities, device)),
        hasSurface: Boolean(sharedControlSurface(group.capabilities, device)),
      }));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [device, pluginLocaleRevision]);

  const activeMode = controls.some((c) => c.id === mode) ? mode : controls[0]?.id ?? '';
  const activeGroup = controls.find((c) => c.id === activeMode);
  const activeCapabilities = activeGroup?.capabilities ?? [];
  const metricPresentation = sharedControlMetric(activeCapabilities, device);
  const surfacePresentation = sharedControlSurface(activeCapabilities, device);
  const activeHasMetric = activeGroup?.hasMetric ?? false;
  const activeHasSurface = activeGroup?.hasSurface ?? false;
  const pollingContextTarget = metricPresentation?.variant === 'hertz'
    ? '.metric-reading > .metric-reading-heading'
    : undefined;
  useControlTargetPosition(controlStageLayersRef, pollingContextTarget, sharedContextLayerRef, 'snap');
  useControlTargetPosition(controlStageLayersRef, metricPresentation?.targetSelector, sharedMetricLayerRef, 'morph');
  useControlTargetPosition(controlStageLayersRef, surfacePresentation?.targetSelector, sharedSurfaceLayerRef, 'snap');

  const switchMode = (targetMode: string, sync: 'metric' | 'surface') => {
    if (!targetMode || targetMode === activeMode) return;
    setContextMotionSync(sync);
    setMode(targetMode);
    setPreviewMessage('');
  };

  const statusItems = (() => {
    const items: { capability: PluginCapability; placement: PluginCapabilityPlacement; onClick: (() => void) | undefined }[] = [];
    for (const capability of device.pluginCapabilities) {
      if (!capabilityAvailable(capability)) continue;
      if (!resolveVisibleWhen(capability.metadata.visibleWhen, device)) continue;
      const display = resolveStatusDisplay(capability);
      if (!display) continue;
      const placements = placementsFor(capability, 'status');
      for (const placement of placements) {
        let onClick: (() => void) | undefined;
        if (display.onClickField) {
          const field = resolveStatusField(capability, display.onClickField, device);
          if (field) {
            const isWritable = Boolean(resolveMutation(field.mutation, device.writableMutations));
            if (isWritable) {
              onClick = () => {
                invoke('device_refresh_quick').catch(() => {});
                setEditingField({ capability, field });
              };
            }
          }
        } else {
          const controlPlacement = placementsFor(capability, 'control')[0];
          if (controlPlacement) {
            const target = controlPlacement.group || capability.id;
            const targetControl = controls.find((control) => control.id === target);
            if (targetControl) {
              const sync = resolveContextMotionSync(
                activeHasMetric,
                activeHasSurface,
                targetControl.hasMetric,
                targetControl.hasSurface,
              );
              onClick = () => switchMode(target, sync);
            }
          }
        }
        items.push({ capability, placement, onClick });
      }
    }
    return items.sort((a, b) => a.placement.order - b.placement.order).slice(0, MAX_STATUS_ITEMS);
  })();

  const selectedEntry = selectedDeviceEntry(deviceEntries);
  const multipleDevices = deviceEntries.length > 1;
  const runtimePending = deviceRuntimePending(device);

  return (
    <main className={`dashboard ${runtimePending ? 'is-initializing' : 'is-ready'}`} aria-busy={runtimePending}>
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
                onClick={() => {
                  if (!showBatteries) invoke('device_refresh_battery').catch(() => {});
                  setShowBatteries((visible) => !visible);
                }}
              >
                <BatteryLevelIcon percentage={device.batteries[0].percentage} charging={device.batteries[0].charging} />
                {device.batteries[0].percentage}%
                {device.batteries[0].charging ? ` · ${t('common.charging')}` : ''}
                <span className="battery-count">{t('dashboard.deviceCount', { count: device.batteries.length })}</span>
              </button>
              <section id="device-batteries" className="battery-popover" aria-label={t('dashboard.deviceBattery')}>
                <div className="battery-popover-header">
                  <span>{t('dashboard.deviceBattery')}</span>
                  <strong>{t('dashboard.deviceCount', { count: device.batteries.length })}</strong>
                </div>
                <div className="battery-device-list">
                  {device.batteries.map((battery) => {
                    const batteryLevel = Math.max(0, Math.min(100, battery.percentage));
                    const batteryTone = battery.charging ? 'charging' : batteryLevel <= 20 ? 'low' : 'normal';
                    return (
                      <div key={battery.id} className={`battery-device ${batteryTone}`}>
	                        <div className="battery-device-main">
	                          <span className="battery-device-label">
	                            <BatteryLevelIcon percentage={battery.percentage} charging={battery.charging} />
	                            <span>{t(battery.label, { defaultValue: battery.label })}</span>
	                          </span>
                          <span className="battery-device-value">
                            <strong>{battery.percentage}%</strong>
                            {battery.charging && <small>{t('common.charging')}</small>}
                          </span>
                        </div>
                        <span className="battery-meter" aria-hidden="true">
                          <span className="battery-meter-fill" style={{ width: `${batteryLevel}%` }} />
                        </span>
                      </div>
                    );
                  })}
                </div>
                <button
                  type="button"
                  className="battery-usage-entry"
                  onClick={() => {
                    setShowBatteries(false);
                    onOpenBatteryUsage();
                  }}
                >
                  <ChartBar weight="regular" />
                  <span>{t('batteryUsage.viewTrend')}</span>
                </button>
              </section>
            </div>
            )}
          </div>
        </div>
        <DeviceAura color={declaredAccentColor(device)} />
      </section>

      <div
        className="control-tabs segmented-slider"
        role="tablist"
        aria-label={t('dashboard.deviceControl')}
        data-active-index={Math.max(controls.findIndex(({ id }) => activeMode === id), 0)}
        style={{
          gridTemplateColumns: `repeat(${Math.max(controls.length, 1)}, minmax(0, 1fr))`,
          width: `min(92%, ${Math.max(220, controls.length * 104)}px)`,
          ...segmentedIndicatorStyle(
            controls.length,
            Math.max(controls.findIndex(({ id }) => activeMode === id), 0),
            { gap: 3, padding: 4 },
          ),
        }}
      >
        {controls.map(({ id, label, icon, hasMetric, hasSurface }) => {
          const sync = resolveContextMotionSync(
            activeHasMetric,
            activeHasSurface,
            hasMetric,
            hasSurface,
          );
          return (
            <button
              key={id}
              role="tab"
              aria-selected={activeMode === id}
              className={activeMode === id ? 'active' : ''}
              onClick={() => {
                invoke('device_refresh_quick').catch(() => {});
                switchMode(id, sync);
              }}
            >
              <PluginIconView name={icon} device={device} />
              <span>{label}</span>
            </button>
          );
        })}
      </div>

      <section
        className={[
          'control-stage',
          previewMessage ? 'has-preview-message' : '',
          pollingContextTarget ? 'has-shared-context' : '',
          metricPresentation ? 'has-shared-metric' : '',
          surfacePresentation ? 'has-shared-surface' : '',
        ].filter(Boolean).join(' ')}
        aria-live="polite"
        data-control-mode={activeMode}
      >
        <div ref={controlStageLayersRef} className="control-stage-layers">
          <SharedControlSurfaceLayer layerRef={sharedSurfaceLayerRef} surface={surfacePresentation} />
          <div className="control-stage-content">
            {activeCapabilities.map((capability) => (
              <CapabilityRouter
                key={capability.id}
                capability={capability}
                device={device}
                writeBusy={writeBusy}
                runMutation={runMutation}
              />
            ))}
          </div>
          <SharedPollingContextLayer
            layerRef={sharedContextLayerRef}
            sync={contextMotionSync}
            visible={Boolean(pollingContextTarget)}
          />
          <SharedControlMetricLayer
            contextKey={activeMode}
            layerRef={sharedMetricLayerRef}
            metric={metricPresentation}
            sync={contextMotionSync}
          />
        </div>
        {previewMessage && <p className="preview-message">{previewMessage}</p>}
        {editingField && (
          <FieldEditModal
            field={editingField.field}
            device={device}
            writeBusy={writeBusy}
            onClose={() => setEditingField(null)}
            onApply={(value) => {
              const field = editingField.field;
              const mutation = resolveMutation(field.mutation, device.writableMutations);
              if (mutation) {
                void runMutation(mutation, resolveFieldMutationParams(field, device, value));
              }
              setEditingField(null);
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
        {statusItems.map(({ capability, placement, onClick }) => (
          <StatusItem
            key={capability.id}
            capability={capability}
            device={device}
            placement={placement}
            onClick={onClick}
          />
        ))}
      </section>
      )}
      <div className="dashboard-meta">
        <span>{t('dashboard.lastUpdate', { time: device.updatedAt })}</span>
        <button className="details-button" onClick={() => { invoke('device_refresh').catch(() => {}); setShowDetails(true); }}><ReadCvLogo weight="regular" />{t('dashboard.allReadInfo')}</button>
      </div>
      {showDetails && <DeviceDetails device={device} onClose={() => setShowDetails(false)} />}
    </main>
  );
}

export default function App() {
  const { t } = useTranslation();
  const pureWeb = isPureWebPreview();
  const [device, setDevice] = useState<DeviceState | undefined>(pureWeb ? MOCK_DEVICE : undefined);
  const [deviceEntries, setDeviceEntries] = useState<DeviceSnapshotEntry[]>(pureWeb ? MOCK_DEVICE_ENTRIES : []);
  const deviceEntriesRef = useRef<DeviceSnapshotEntry[]>(pureWeb ? MOCK_DEVICE_ENTRIES : []);
  // device-updated 可能由初始化或用户按需读取连续触发；用 startTransition
  // 将 Dashboard 渲染标记为低优先级。writeBusy 等用户交互状态保持同步。
  const [, startTransition] = useTransition();
  const [theme, setTheme] = useState<ThemeMode>('system');
  const [themeLoaded, setThemeLoaded] = useState(pureWeb);
  const [view, setView] = useState<View>('dashboard');
  const [aboutFocusToken, setAboutFocusToken] = useState(0);
  const [settingsPluginFocusToken, setSettingsPluginFocusToken] = useState(0);
  const [demoMode, setDemoMode] = useState(pureWeb);
  const [refreshNonce, setRefreshNonce] = useState(0);
  const [appNotification, setAppNotification] = useState<AppNotification>();
  const [showBatteryUsage, setShowBatteryUsage] = useState(false);
  const [batteryUsageSession, setBatteryUsageSession] = useState(0);
  const [batteryUsageSettings, setBatteryUsageSettings] = useState<{ batteryHistoryEnabled: boolean; aiAnalysisEnabled: boolean } | undefined>(
    pureWeb ? { batteryHistoryEnabled: true, aiAnalysisEnabled: false } : undefined,
  );
  const [pluginLocaleRevision, setPluginLocaleRevision] = useState(0);
  const windowsPlatform = isWindowsPlatform();
  const macPlatform = isMacPlatform();
  const windowsWebPreview = isWindowsWebPreview();
  const fallbackPlatform = !pureWeb && !windowsPlatform && !macPlatform;
  // Modal 打开期间禁用通知的跳转 / 打开行为（通知本身仍可见、可关闭）。
  const modalOpen = useHasOpenModal();
  const notificationActionEnabled = !modalOpen;
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
  const openBatteryUsage = useCallback(() => {
    setBatteryUsageSession((value) => value + 1);
    setShowBatteryUsage(true);
  }, []);
  const syncBatteryUsageSettings = useCallback((settings: { batteryHistoryEnabled: boolean; aiAnalysisEnabled: boolean }) => {
    setBatteryUsageSettings(settings);
  }, []);
  const reloadPluginLocales = useCallback(() => {
    void loadPluginLocales().then((loaded) => {
      if (loaded) setPluginLocaleRevision((value) => value + 1);
    });
  }, []);

  useEffect(() => onAppNotification(setAppNotification), []);

  useEffect(() => {
    if (pureWeb) return;
    let unlisten: (() => void) | undefined;
    let unlistenResume: (() => void) | undefined;
    let unlistenFocus: (() => void) | undefined;
    let unlistenBatteryUsage: (() => void) | undefined;
    let unlistenPluginLocales: (() => void) | undefined;
    listen('navigate-about-update', () => openAboutUpdate())
      .then((un) => { unlisten = un; })
      .catch(() => {});
    let unlistenPluginUpdate: (() => void) | undefined;
    listen('navigate-plugin-update', () => openSettingsPluginUpdate())
      .then((un) => { unlistenPluginUpdate = un; })
      .catch(() => {});
    listen('open-battery-usage', () => openBatteryUsage())
      .then((un) => { unlistenBatteryUsage = un; })
      .catch(() => {});
    listen('plugin-locales-updated', () => reloadPluginLocales())
      .then((un) => { unlistenPluginLocales = un; })
      .catch(() => {});
    listen('window-resumed', () => {
      setRefreshNonce((value) => value + 1);
    }).then((un) => { unlistenResume = un; })
      .catch(() => {});
    // macOS 原生通知不暴露点击回调：发通知时将跳转动作写入 pending action，
    // 窗口聚焦时取走并执行。Windows/Linux 由 `navigate-about-update` 事件直接处理，
    // 此处返回 null 不影响。
    try {
      getCurrentWindow().onFocusChanged(({ payload: focused }) => {
        if (!focused) return;
        invoke<string | null>('take_pending_notification_action')
          .then((action) => {
            if (action === 'about-update') openAboutUpdate();
            if (action === 'settings-plugin-update') openSettingsPluginUpdate();
            if (action === 'battery-usage') openBatteryUsage();
          })
          .catch(() => {});
      }).then((un) => { unlistenFocus = un; }).catch(() => {});
    } catch {
      // 非 Tauri 环境忽略
    }
    return () => {
      if (unlisten) unlisten();
      if (unlistenPluginUpdate) unlistenPluginUpdate();
      if (unlistenResume) unlistenResume();
      if (unlistenFocus) unlistenFocus();
      if (unlistenBatteryUsage) unlistenBatteryUsage();
      if (unlistenPluginLocales) unlistenPluginLocales();
    };
  }, [openAboutUpdate, openBatteryUsage, openSettingsPluginUpdate, pureWeb, reloadPluginLocales]);

  // 加载插件 locale，注册为 i18n namespace（以插件 ID 命名）。
  // 异步加载完成后刷新插件标签 memo，加载前使用 host 回退标签。
  useEffect(() => {
    reloadPluginLocales();
  }, [reloadPluginLocales]);

  useEffect(() => {
    if (!appNotification) return;
    const timeout = window.setTimeout(() => setAppNotification(undefined), 6000);
    return () => window.clearTimeout(timeout);
  }, [appNotification]);

  useEffect(() => {
    if (pureWeb) return;
    invoke<AppSettings>('settings_get')
      .then((settings) => {
        setTheme(settings.theme as ThemeMode);
        setThemeLoaded(true);
        syncBatteryUsageSettings({
          batteryHistoryEnabled: settings.batteryHistoryEnabled ?? true,
          aiAnalysisEnabled: localAiFeatureEnabled(settings, LOCAL_AI_FEATURE.batteryUsage),
        });
        applyLanguage(settings.language ?? 'auto');
        if (settings.automaticUpdateChecks) {
          void invoke<AboutInfo>('about_info')
            .then((info) => {
              if (info.updaterActive) return startAutomaticAppUpdateCheck(true, settings.automaticUpdateInstall);
            })
            .catch(() => { /* Pre-release and offline builds skip automatic application checks. */ });
        }
        void startAutomaticPluginUpdateCheck(settings.automaticPluginUpdateChecks);
      })
      .catch(() => setThemeLoaded(true));
  }, [pureWeb, syncBatteryUsageSettings]);

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

  const themeColor = device ? declaredAccentColor(device) : undefined;
  const selectedBatteryUsageTarget = batteryUsageTarget(selectedDeviceEntry(deviceEntries));
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
    {view === 'dashboard' && (device ? <Dashboard device={device} deviceEntries={deviceEntries} onDeviceChange={setDevice} onDeviceSelect={selectDevice} onOpenBatteryUsage={openBatteryUsage} pluginLocaleRevision={pluginLocaleRevision} demoMode={demoMode} /> : <EmptyState onRefresh={() => { setDemoMode(false); setDevice(undefined); setDeviceEntries([]); deviceEntriesRef.current = []; setRefreshNonce((value) => value + 1); invoke('device_refresh').catch(() => {}); }} onDemo={() => { setDemoMode(true); setDevice(MOCK_DEVICE); setDeviceEntries(MOCK_DEVICE_ENTRIES); deviceEntriesRef.current = MOCK_DEVICE_ENTRIES; }} onOpenSettings={() => setView('settings')} />)}
    {view === 'settings' && <SettingsPage previewMode={pureWeb} focusPluginUpdateToken={settingsPluginFocusToken} onNavigateAbout={() => setView('about')} onOpenBatteryUsage={openBatteryUsage} onBatteryUsageSettingsChange={syncBatteryUsageSettings} onThemeChange={setTheme} pluginCapabilities={device?.pluginCapabilities ?? []} writableMutations={device?.writableMutations ?? []} />}
    {view === 'about' && <AboutPage previewMode={pureWeb} focusUpdateToken={aboutFocusToken} onBack={() => setView('settings')} />}
    <BatteryUsageModal
      key={batteryUsageSession}
      open={showBatteryUsage}
      onClose={() => setShowBatteryUsage(false)}
      hasBattery={(device?.batteries.length ?? 0) > 0}
      batteryHistoryEnabled={batteryUsageSettings?.batteryHistoryEnabled}
      aiAnalysisEnabled={batteryUsageSettings?.aiAnalysisEnabled}
      preferredDeviceName={selectedBatteryUsageTarget?.name}
      preferredComponentId={selectedBatteryUsageTarget?.componentId}
    />
    {appNotification && (
      <OverlayPortal>
        <aside
          className={`app-notification ${appNotification.kind} ${appNotification.action && notificationActionEnabled ? 'actionable' : ''}`}
          role={appNotification.kind === 'error' ? 'alert' : 'status'}
          aria-live={appNotification.kind === 'error' ? 'assertive' : 'polite'}
          data-action-disabled={!notificationActionEnabled ? 'true' : undefined}
          onClick={
            appNotification.action && notificationActionEnabled
              ? appNotification.action === 'about-update'
                ? openAboutUpdate
                : appNotification.action === 'settings-plugin-update'
                  ? openSettingsPluginUpdate
                  : appNotification.action === 'battery-usage'
                    ? openBatteryUsage
                    : appNotification.action === 'relaunch'
                      ? () => void relaunchAfterUpdate()
                      : undefined
              : undefined
          }
        >
          <div><strong>{appNotification.title}</strong>{appNotification.body && <p>{appNotification.body}</p>}</div>
          <button type="button" onClick={(event) => { event.stopPropagation(); setAppNotification(undefined); }} aria-label={t('dashboard.closeNotification')}><X weight="bold" /></button>
        </aside>
      </OverlayPortal>
    )}
  </div>;
}
