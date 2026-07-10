// SPDX-License-Identifier: AGPL-3.0-or-later
import { useCallback, useEffect, useMemo, useRef, useState, useTransition } from 'react';
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
import type { AboutInfo, AppSettings, DeviceCapabilities, DeviceSnapshot, DeviceSnapshotEntry, DeviceState, DpiStage, PluginCapability, PluginCapabilityPlacement, PluginField, PluginFieldFormat, PluginUpdateInfo, RangeSpec, ThemeMode } from './types';
import {
  MAX_CONTROL_GROUPS,
  MAX_STATUS_ITEMS,
  readPath,
  resolveMutation,
  resolveFieldLabel,
  resolveFieldOptions,
  resolveFieldRange,
  resolveStageLayout,
  resolveStateMapping,
  resolveStatusDisplay,
  resolveSwitchState,
  resolveVisibleWhen,
  resolveZones,
} from './pluginAdapter';
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

function ColorValue({ value, fallback, className }: { value: unknown; fallback?: string; className?: string }) {
  const label = typeof value === 'string' && value ? value : fallback ?? i18n.t('common.notReported');
  const style = colorValueStyle(value);
  const classes = [className, style ? 'color-value' : undefined].filter(Boolean).join(' ') || undefined;
  return <strong className={classes} style={style}>{label}</strong>;
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
    : <strong className={className}>{text}</strong>;
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

/** 从插件声明中取得用于宿主装饰的颜色，不依赖任何厂商状态字段名。 */
function declaredAccentColor(device: DeviceState): string | undefined {
  for (const capability of device.pluginCapabilities) {
    const zones = capability.metadata.zones ?? [];
    for (const zone of zones) {
      const color = zone.fields.find((field) => field.format === 'color' || field.editor === 'modal-color');
      if (color) {
        const value = readPath(device, color.source);
        if (typeof value === 'string') return value;
      }
    }
    const layout = capability.metadata.stageLayout;
    if (layout) {
      const stages = readPath(device, layout.colorSource ?? layout.dotsSource) as DpiStage[] | undefined;
      const active = stages?.find((stage) => stage.enabled && stage.active) ?? stages?.find((stage) => stage.enabled);
      if (active?.color) return active.color;
    }
  }
  return undefined;
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
  const resolvedTitle = title ?? resolveFieldLabel(field, device, device.pluginId);
  const initialValue = currentValue ?? readPath(device, field.source);
  const range = resolveFieldRange(field);
  const options = resolveFieldOptions(field, device);

  const [draft, setDraft] = useState<unknown>(() => {
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
  });

  const submitDisabled = useMemo(() => {
    if (writeBusy) return true;
    return draft === initialValue;
  }, [writeBusy, draft, initialValue]);

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
            <span>{resolvedTitle}</span>
            <select
              autoFocus
              aria-label={resolvedTitle}
              value={String(draft ?? '')}
              disabled={writeBusy}
              onChange={(event) => setDraft(event.target.value)}
            >
              {options.map((option) => (
                <option key={String(option.value)} value={String(option.value)}>{resolveLabelKey(option.labelKey, device.pluginId)}</option>
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
              onChange={(event) => setDraft(event.target.value)}
            />
          </label>
        );
      case 'modal-range':
        return (
          <label className="edit-field range-field">
            <span>{resolvedTitle}</span>
            <input
              type="range"
              autoFocus
              aria-label={resolvedTitle}
              value={typeof draft === 'number' ? draft : Number(draft ?? 0)}
              min={range?.min}
              max={range?.max}
              step={range?.step}
              disabled={writeBusy}
              onChange={(event) => setDraft(Number(event.target.value))}
            />
            <span className="range-value">{typeof draft === 'number' ? draft : Number(draft ?? 0)}</span>
          </label>
        );
      case 'modal-number':
        return (
          <label className="edit-field">
            <span>{resolvedTitle}</span>
            <input
              type="number"
              autoFocus
              aria-label={resolvedTitle}
              value={typeof draft === 'number' ? draft : Number(draft ?? 0)}
              min={range?.min}
              max={range?.max}
              step={range?.step}
              disabled={writeBusy}
              onChange={(event) => setDraft(Number(event.target.value))}
            />
          </label>
        );
      case 'modal-gradient':
        return (
          <label className="edit-field">
            <span>{resolvedTitle}</span>
            <input
              type="text"
              autoFocus
              aria-label={resolvedTitle}
              value={typeof draft === 'string' ? draft : String(draft ?? '')}
              disabled={writeBusy}
              onChange={(event) => setDraft(event.target.value)}
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
    const param = field.param ?? 'value';
    if (isOn) {
      void runMutation(mutation, { ...field.params, [param]: sw.offValue });
    } else {
      let restoreValue = restoreRef.current;
      if (restoreValue === undefined && field.options) {
        const nonOff = field.options.find((opt) => opt.value !== sw.offValue);
        restoreValue = nonOff?.value;
      }
      if (restoreValue !== undefined) {
        void runMutation(mutation, { ...field.params, [param]: restoreValue });
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
      <strong>{isOn ? i18n.t('common.on') : i18n.t('common.off')}</strong>
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
            onClick={() => mutation && applyMutation(mutation, { ...field.params, [field.param ?? 'value']: value !== true })}
          >{value === true ? i18n.t('common.on') : i18n.t('common.off')}</button>
        </>
      );

    case 'inline-segmented': {
      const options = resolveFieldOptions(field, device);
      return (
        <>
          <span>{label}</span>
          <div
            className="plugin-segmented"
            role="group"
            aria-label={label}
            style={{ gridTemplateColumns: `repeat(${options.length}, minmax(0, 1fr))` }}
          >
            {options.map((option) => (
              <button
                key={String(option.value)}
                type="button"
                className={value === option.value ? 'active' : ''}
                aria-pressed={value === option.value}
                disabled={!writable}
                onClick={() => mutation && applyMutation(mutation, { ...field.params, [field.param ?? 'value']: option.value })}
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
          <FormattedValue value={value} format={field.format} className="plugin-current-value" />
        </>
      );

    case 'inline-action':
      return (
        <button
          type="button"
          className="plugin-action"
          disabled={!writable}
          onClick={() => mutation && applyMutation(mutation, field.params ?? {})}
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
            onClick={() => setEditing(true)}
          >
            <span>{label}</span>
            <FormattedValue value={value} format={field.format} />
          </button>
          {editing && (
            <FieldEditModal
              field={field}
              device={device}
              writeBusy={writeBusy}
              onClose={() => setEditing(false)}
              onApply={(v) => {
                if (mutation) applyMutation(mutation, { ...field.params, [field.param ?? 'value']: v });
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
          <FormattedValue value={value} format={field.format} className="plugin-current-value" />
        </>
      );

    default:
      console.warn('Unknown field editor:', field.editor);
      return (
        <>
          <span>{label}</span>
          <FormattedValue value={value} format={field.format} className="plugin-current-value" />
        </>
      );
  }
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
  };

  return (
    <>
      <button
        type="button"
        className="primary-reading editable-reading"
        aria-label={activeDpi ? i18n.t('dashboard.currentDpiEdit', { value: activeDpi }) : i18n.t('dashboard.dpiNotReported')}
        disabled={writeBusy || !setWritable || !activeDpi}
        onClick={() => activeDpi && setEditingStage(currentStageNumber)}
      >
        <strong>{activeDpi || i18n.t('common.notReported')}</strong><em>DPI</em>
      </button>
      <div className="dpi-scale" aria-label={i18n.t('dashboard.dpiStages')} style={{ '--stage-count': Math.max(displayedStages.length, 1) } as React.CSSProperties}>
        {displayedStages.map((stage, index) => {
          const stageNumber = index + 1;
          return (
            <div key={`${index}-${stage.value}`} className="dpi-stage-item">
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
                onClick={() => setEditingStage(stageNumber)}
                aria-label={i18n.t('dashboard.editStageDpi', { stage: stageNumber })}
              >
                {stage.value}
              </button>
            </div>
          );
        })}
      </div>
      {displayedStages.length === 0 && <p className="setting-hint">{i18n.t('dashboard.noDpiStages')}</p>}
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

  if (zones.length === 0) return null;

  const activeZone = zones.find((z) => z.id === activeZoneId) ?? zones[0];
  const multipleZones = zones.length > 1;

  const colorField = activeZone.fields.find((f) => f.editor === 'modal-color' || f.format === 'color');
  const zoneColor = colorField ? readPath(device, colorField.source) as string | undefined : undefined;

  const visibleFields = activeZone.fields.filter((f) => resolveVisibleWhen(f.visibleWhen, device));

  return (
    <>
      {multipleZones && (
        <div
          className="lighting-sub-tabs"
          role="tablist"
          aria-label={i18n.t('dashboard.lightingTarget')}
          style={{ gridTemplateColumns: `repeat(${zones.length}, minmax(0, 1fr))` }}
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
      <div className="lighting-swatch" style={{ '--light-color': zoneColor ?? '#b87ab0' } as React.CSSProperties} />
      <div className="lighting-sections" aria-label={i18n.t('dashboard.lightingGroups')}>
        <div className={`lighting-group lighting-group-${activeZone.id}`}>
          {multipleZones && <p className="lighting-group-title">{resolveLabelKey(activeZone.labelKey, device.pluginId)}</p>}
          <div
            className="lighting-rows"
            style={{ gridTemplateColumns: `repeat(${Math.max(visibleFields.length, 1)}, minmax(0, 1fr))` }}
          >
            {visibleFields.map((field) => (
              <FieldRenderer
                key={field.id}
                field={field}
                device={device}
                writeBusy={writeBusy}
                runMutation={runMutation}
              />
            ))}
          </div>
        </div>
      </div>
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

  const value = readPath(device, display.valueSource);
  const label = resolveLabelKey(capability.labelKey, device.pluginId);

  let valueText: string;
  if (display.valueOptions) {
    const option = display.valueOptions.find((opt) => opt.value === value);
    valueText = option ? resolveLabelKey(option.labelKey, device.pluginId) : formatFieldValue(value, display.valueFormat, i18n.t);
  } else {
    valueText = formatFieldValue(value, display.valueFormat, i18n.t);
  }

  const isColor = display.valueFormat === 'color' || valueLooksColor(value);

  const content = (
    <>
      <PluginIconView name={placement.icon} device={device} />
      <span>{label}<strong>{valueText}</strong></span>
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
      </div>
    );
  }
  if (capability.control === 'LightingZone') {
    return (
      <div className="control-reading mode-reading lighting-reading">
        <ZoneRenderer capability={capability} device={device} writeBusy={writeBusy} runMutation={runMutation} />
      </div>
    );
  }
  const fields = (capability.metadata.fields ?? []).filter((f) => resolveVisibleWhen(f.visibleWhen, device));
  return (
    <div className="control-reading mode-reading plugin-control-reading">
      {fields.map((field) => (
        <FieldRenderer
          key={field.id}
          field={field}
          device={device}
          writeBusy={writeBusy}
          runMutation={runMutation}
        />
      ))}
    </div>
  );
}

/// 在 capability 的 fields 和 zones[].fields 中查找指定 ID 的字段。
function findField(capability: PluginCapability, fieldId: string): PluginField | undefined {
  const fields = capability.metadata.fields;
  if (fields) {
    const found = fields.find((f) => f.id === fieldId);
    if (found) return found;
  }
  const zones = capability.metadata.zones;
  if (zones) {
    for (const zone of zones) {
      const found = zone.fields.find((f) => f.id === fieldId);
      if (found) return found;
    }
  }
  return undefined;
}

function DeviceDetails({ capabilities, pluginCapabilities, onClose }: { capabilities: DeviceCapabilities; pluginCapabilities: PluginCapability[]; onClose: () => void }) {
  const detailOrder = new Map<string, number>();
  for (const capability of pluginCapabilities) {
    const placement = placementsFor(capability, 'details')[0];
    if (placement) {
      detailOrder.set(capability.id, placement.order);
    }
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
                    <dd><FormattedValue value={value} /></dd>
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

function Dashboard({
  device,
  deviceEntries,
  onDeviceChange,
  onDeviceSelect,
  onOpenBatteryUsage,
  pluginLocaleRevision,
}: {
  device: DeviceState;
  deviceEntries: DeviceSnapshotEntry[];
  onDeviceChange: (device: DeviceState) => void;
  onDeviceSelect: (deviceKey: string) => void;
  onOpenBatteryUsage: () => void;
  pluginLocaleRevision: number;
}) {
  const { t } = useTranslation();
  const [mode, setMode] = useState<ControlMode>('');
  const [previewMessage, setPreviewMessage] = useState('');
  const [showDetails, setShowDetails] = useState(false);
  const [showBatteries, setShowBatteries] = useState(false);
  const [showDeviceSwitcher, setShowDeviceSwitcher] = useState(false);
  const batteryControlRef = useRef<HTMLDivElement>(null);
  const deviceSwitcherRef = useRef<HTMLDivElement>(null);
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
    return [...groups.values()].slice(0, MAX_CONTROL_GROUPS);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [device, pluginLocaleRevision]);

  const activeMode = controls.some((c) => c.id === mode) ? mode : controls[0]?.id ?? '';
  const activeGroup = controls.find((c) => c.id === activeMode);
  const activeCapabilities = activeGroup?.capabilities ?? [];

  const statusItems = useMemo(() => {
    const items: { capability: PluginCapability; placement: PluginCapabilityPlacement; onClick: (() => void) | undefined }[] = [];
    for (const capability of device.pluginCapabilities) {
      if (!resolveVisibleWhen(capability.metadata.visibleWhen, device)) continue;
      const display = resolveStatusDisplay(capability);
      if (!display) continue;
      const placements = placementsFor(capability, 'status');
      for (const placement of placements) {
        let onClick: (() => void) | undefined;
        if (display.onClickField) {
          const field = findField(capability, display.onClickField);
          if (field) {
            const isWritable = Boolean(resolveMutation(field.mutation, device.writableMutations));
            if (isWritable) {
              onClick = () => setEditingField({ capability, field });
            }
          }
        } else {
          const controlPlacement = placementsFor(capability, 'control')[0];
          if (controlPlacement) {
            const target = controlPlacement.group || capability.id;
            if (controls.some((c) => c.id === target)) {
              onClick = () => setMode(target);
            }
          }
        }
        items.push({ capability, placement, onClick });
      }
    }
    return items.sort((a, b) => a.placement.order - b.placement.order).slice(0, MAX_STATUS_ITEMS);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [device, controls, pluginLocaleRevision]);

  const selectedEntry = selectedDeviceEntry(deviceEntries);
  const multipleDevices = deviceEntries.length > 1;

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
        className="control-tabs"
        role="tablist"
        aria-label={t('dashboard.deviceControl')}
        style={{
          gridTemplateColumns: `repeat(${Math.max(controls.length, 1)}, minmax(0, 1fr))`,
          width: `min(92%, ${Math.max(220, controls.length * 104)}px)`,
        }}
      >
        {controls.map(({ id, label, icon }) => (
          <button
            key={id}
            role="tab"
            aria-selected={activeMode === id}
            className={activeMode === id ? 'active' : ''}
            onClick={() => { setMode(id); setPreviewMessage(''); }}
          >
            <PluginIconView name={icon} device={device} />
            <span>{label}</span>
          </button>
        ))}
      </div>

      <section className={`control-stage ${previewMessage ? 'has-preview-message' : ''}`} aria-live="polite">
        {activeCapabilities.map((capability) => (
          <CapabilityRouter
            key={capability.id}
            capability={capability}
            device={device}
            writeBusy={writeBusy}
            runMutation={runMutation}
          />
        ))}
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
                void runMutation(mutation, { ...field.params, [field.param ?? 'value']: value });
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
        <button className="details-button" onClick={() => setShowDetails(true)}><ReadCvLogo weight="regular" />{t('dashboard.allReadInfo')}</button>
      </div>
      {showDetails && <DeviceDetails capabilities={device.capabilities} pluginCapabilities={device.pluginCapabilities} onClose={() => setShowDetails(false)} />}
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
  const [showBatteryUsage, setShowBatteryUsage] = useState(false);
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
  const openBatteryUsage = useCallback(() => {
    setShowBatteryUsage(true);
  }, []);

  useEffect(() => onAppNotification(setAppNotification), []);

  useEffect(() => {
    if (pureWeb) return;
    let unlisten: (() => void) | undefined;
    let unlistenResume: (() => void) | undefined;
    let unlistenFocus: (() => void) | undefined;
    let unlistenBatteryUsage: (() => void) | undefined;
    listen('navigate-about-update', () => openAboutUpdate())
      .then((un) => { unlisten = un; })
      .catch(() => {});
    listen('open-battery-usage', () => openBatteryUsage())
      .then((un) => { unlistenBatteryUsage = un; })
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
            if (action === 'battery-usage') openBatteryUsage();
          })
          .catch(() => {});
      }).then((un) => { unlistenFocus = un; }).catch(() => {});
    } catch {
      // 非 Tauri 环境忽略
    }
    return () => {
      if (unlisten) unlisten();
      if (unlistenResume) unlistenResume();
      if (unlistenFocus) unlistenFocus();
      if (unlistenBatteryUsage) unlistenBatteryUsage();
    };
  }, [openAboutUpdate, openBatteryUsage, pureWeb]);

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

  const themeColor = device ? declaredAccentColor(device) : undefined;
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
    {view === 'dashboard' && (device ? <Dashboard device={device} deviceEntries={deviceEntries} onDeviceChange={setDevice} onDeviceSelect={selectDevice} onOpenBatteryUsage={openBatteryUsage} pluginLocaleRevision={pluginLocaleRevision} /> : <EmptyState onRefresh={() => { setDemoMode(false); setDevice(undefined); setDeviceEntries([]); deviceEntriesRef.current = []; setRefreshNonce((value) => value + 1); invoke('device_refresh').catch(() => {}); }} onDemo={() => { setDemoMode(true); setDevice(MOCK_DEVICE); setDeviceEntries(MOCK_DEVICE_ENTRIES); deviceEntriesRef.current = MOCK_DEVICE_ENTRIES; }} onOpenSettings={() => setView('settings')} />)}
    {view === 'settings' && <SettingsPage previewMode={pureWeb} focusPluginUpdateToken={settingsPluginFocusToken} onNavigateAbout={() => setView('about')} onOpenBatteryUsage={openBatteryUsage} onThemeChange={setTheme} pluginCapabilities={device?.pluginCapabilities ?? []} writableMutations={device?.writableMutations ?? []} />}
    {view === 'about' && <AboutPage previewMode={pureWeb} focusUpdateToken={aboutFocusToken} onBack={() => setView('settings')} />}
    <BatteryUsageModal
      open={showBatteryUsage}
      onClose={() => setShowBatteryUsage(false)}
      hasBattery={(device?.batteries.length ?? 0) > 0}
    />
    {appNotification && (
      <aside
        className={`app-notification ${appNotification.kind} ${appNotification.action ? 'actionable' : ''}`}
        role={appNotification.kind === 'error' ? 'alert' : 'status'}
        aria-live={appNotification.kind === 'error' ? 'assertive' : 'polite'}
        onClick={appNotification.action === 'about-update' ? openAboutUpdate : appNotification.action === 'settings-plugin-update' ? openSettingsPluginUpdate : appNotification.action === 'battery-usage' ? openBatteryUsage : appNotification.action === 'relaunch' ? () => void relaunchAfterUpdate() : undefined}
      >
        <div><strong>{appNotification.title}</strong>{appNotification.body && <p>{appNotification.body}</p>}</div>
        <button type="button" onClick={(event) => { event.stopPropagation(); setAppNotification(undefined); }} aria-label={t('dashboard.closeNotification')}><X weight="bold" /></button>
      </aside>
    )}
  </div>;
}
