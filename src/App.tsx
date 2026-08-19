// SPDX-License-Identifier: AGPL-3.0-or-later
import { lazy, Suspense, useCallback, useEffect, useEffectEvent, useLayoutEffect, useMemo, useRef, useState, useTransition } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  BatteryHigh,
  CaretDown,
  ChartBar,
  Cpu,
  Crosshair,
  Gauge,
  Gear,
  Info,
  Lightbulb,
  MagnifyingGlass,
  Minus,
  Mouse,
  ReadCvLogo,
  SignOut,
  SlidersHorizontal,
  Stack,
  Timer,
  WaveSine,
  X,
} from '@phosphor-icons/react';
import { useTranslation } from 'react-i18next';
import { MOCK_DEVICE, MOCK_DEVICE_ENTRIES } from './mock';
import { applyTheme, pastelDisplayColor } from './theme';
import i18n, { applyLanguage, loadPluginLocales, resolveLabelKey } from './i18n';
import type { SettingsTab } from './Settings';
import { AboutPageSkeleton, LogPageSkeleton, SettingsPageSkeleton } from './RuntimePageSkeleton';
import type { BatteryUsageConnectedTarget } from './BatteryUsage';
import { BatteryLevelIcon } from './BatteryLevelIcon';
import type { BatteryChargingEstimate, DeviceSnapshot, DeviceSnapshotEntry, DeviceState, DpiStage, PluginCapability, PluginCapabilityPlacement, PluginChargingEstimatePolicy, PluginField, PluginFieldFormat, PluginSummaryItem, PluginZone, RangeSpec, ReadStatus, ThemeMode } from './types';
import { DetailValue } from './DetailValue';
import {
  placementsFor,
  selectDashboardControls,
  selectDashboardStatus,
  fieldHasReportedValue,
  readPath,
  resolveDetailValueLabel,
  resolveFieldInteraction,
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
  resolveStatusDisplayVariant,
  resolveSwitchState,
  resolveSwitchNextValue,
  resolveVisibleWhen,
  resolveZones,
  selectLightingSubblocks,
  selectSummarySubblocks,
  summaryMaxForCapability,
  simulateDemoMutation,
} from './pluginAdapter';
import { onAppNotification, notifyError, notifySuccess, type AppNotification } from './notify';
import {
  ATTENTION_PRIORITY,
  AttentionBeamLayer,
  AttentionBusController,
  AttentionSurface,
  attentionAppRestartKey,
  attentionAppUpdateKey,
  attentionDesaturatedAccent,
  attentionLocalAiUpdateKey,
  attentionPluginUpdateKey,
  clearPendingLightingAttention,
  confirmPendingLightingAttention,
  detectAttentionVisualSupport,
  peekPendingLightingAttention,
  registerLightingAttention,
  resolveUpdateAttentionTarget,
  useAttentionFeedback,
  type AttentionBeamRequest,
  type ZoneLightingState,
} from './attention';
import { useScrollFadeState } from './useScrollOverflow';
import { appUpdateState, relaunchAfterUpdate, startAutomaticAppUpdateCheck, recordUpdateReminderDismissed, recordUpdateReminderIgnored, remindInstalledUpdateOnShown } from './updater';
import { pluginUpdateState, startAutomaticPluginUpdateCheck } from './plugin-updater';
import { localAiUpdateState, startAutomaticLocalAiUpdateCheck } from './local-ai-updater';
import { initUpdatePriorityCoordinator } from './update-priority';
import { LOCAL_AI_FEATURE, localAiFeatureEnabled } from './localAi';
import { segmentedIndicatorStyle } from './segmentedControl';
import { invalidateAboutInfo, loadAboutInfo, loadAppSettings } from './runtime-data-cache';
import {
  dismissTransientSurfaces,
  Modal,
  OverlayPortal,
  subscribeTransientSurfaceDismiss,
  useHasOpenModal,
} from './overlay';
import { MiraActivityButton, MiraInlineActivity, MiraActivityOverlay } from './activity';
import {
  isSoftwareDpiLayout,
  loadSoftwareDpiStages,
  saveSoftwareDpiStages,
  softwareDpiCurrentValue,
  softwareDpiStorage,
  softwareDpiStageKey,
  softwareDpiStages,
  type SoftwareDpiStageState,
} from './softwareDpiStages';
import './styles.css';

// Dashboard stays in the initial chunk; secondary pages and the analysis-heavy
// battery modal are fetched only when the user opens them.
const SettingsPage = lazy(() => import('./Settings').then((module) => ({ default: module.SettingsPage })));
const AboutPage = lazy(() => import('./About').then((module) => ({ default: module.AboutPage })));
const LogPage = lazy(() => import('./logs/LogPage').then((module) => ({ default: module.LogPage })));
const BatteryUsageModal = lazy(() => import('./BatteryUsage').then((module) => ({ default: module.BatteryUsageModal })));

type View = 'dashboard' | 'settings' | 'about' | 'logs';
type TitledView = Exclude<View, 'dashboard'>;
type ControlMode = string;

type ControlPageKind = 'dpi' | 'segmented' | 'standard';

type ControlStageTransition = {
  id: number;
  fromMode: string;
  fromKind: ControlPageKind;
  fromCapabilities: PluginCapability[];
  preserveOutgoing: boolean;
  toMode: string;
  toKind: ControlPageKind;
};

function controlPageKind(capabilities: PluginCapability[]): ControlPageKind {
  if (capabilities.some((capability) => Boolean(resolveStageLayout(capability)))) return 'dpi';
  if (capabilities.some((capability) => (
    (capability.metadata.fields ?? []).some((field) => field.editor === 'inline-segmented')
  ))) return 'segmented';
  return 'standard';
}

function isWindowsPlatform(): boolean {
  const previewPlatform = new URLSearchParams(window.location.search).get('platform');
  // 显式指定 platform 时以参数为准，不依赖 userAgent
  if (previewPlatform !== null) return previewPlatform === 'windows';
  return navigator.userAgent.includes('Windows');
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
  // sleep -1 表示“从不休眠”（AM35 协议使用 65535 → -1 映射）。
  if (seconds === -1) return i18n.t('common.never');
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

type PersistentTitleFace = { key: string; title: string };

/** 设置 / 关于 / 日志共用的固定标题槽。新旧标题在同一几何位置直接交叉，
 * 新标题由轮廓过渡到实心，旧标题保持实心淡出；短时间反向时复用仍在场的 face。 */
function PersistentPageTitle({ view, title }: { view: TitledView; title: string }) {
  const incomingKey = `${view}::${title}`;
  const [faces, setFaces] = useState<PersistentTitleFace[]>(() => [{ key: incomingKey, title }]);
  const [activeKey, setActiveKey] = useState(incomingKey);
  const facesRef = useRef(faces);
  const activeKeyRef = useRef(activeKey);
  const transitionIdRef = useRef(0);

  useEffect(() => {
    const transitionId = transitionIdRef.current + 1;
    transitionIdRef.current = transitionId;
    const incomingFace = { key: incomingKey, title };
    let prepareFrame = 0;
    let activateFrame = 0;
    let settleTimeout = 0;

    if (activeKeyRef.current === incomingKey) {
      return undefined;
    }

    const activateIncoming = () => {
      if (transitionIdRef.current !== transitionId) return;
      activeKeyRef.current = incomingKey;
      setActiveKey(incomingKey);
      settleTimeout = window.setTimeout(() => {
        if (transitionIdRef.current !== transitionId) return;
        facesRef.current = [incomingFace];
        setFaces([incomingFace]);
      }, 260);
    };

    prepareFrame = window.requestAnimationFrame(() => {
      if (transitionIdRef.current !== transitionId) return;
      const faceAlreadyMounted = facesRef.current.some((face) => face.key === incomingKey);
      if (faceAlreadyMounted) {
        activateIncoming();
        return;
      }

      const nextFaces = [...facesRef.current, incomingFace];
      facesRef.current = nextFaces;
      setFaces(nextFaces);
      activateFrame = window.requestAnimationFrame(activateIncoming);
    });

    return () => {
      window.cancelAnimationFrame(prepareFrame);
      window.cancelAnimationFrame(activateFrame);
      window.clearTimeout(settleTimeout);
    };
  }, [incomingKey, title]);

  return (
    <h1 className="page-persistent-title" aria-hidden="true">
      {faces.map((face) => (
        <span
          key={face.key}
          className={`page-persistent-title-face${
            face.key === activeKey
              ? ` is-active${faces.length > 1 ? ' is-forming' : ''}`
              : face.key === incomingKey
                ? ' is-entering'
                : ' is-exiting'
          }`}
          data-title={face.title}
          aria-hidden="true"
        >
          {face.title}
        </span>
      ))}
    </h1>
  );
}

type MetricFlipValue = {
  contextKey: string;
  text: string;
  unit: string;
  variant: SharedControlMetric['variant'];
};

type MetricDigitFlip = {
  targetText: string;
  targetLength: number;
  duration: number;
  slots: Array<{
    position: number;
    frames: Array<{
      character: string;
      delay: number;
      kind: 'static' | 'initial' | 'cycle' | 'final';
      terminal?: boolean;
    }>;
  }>;
};

const METRIC_DIGIT_STAGGER = 42;
const METRIC_DIGIT_STEP = 52;
const METRIC_DIGIT_FINAL_DURATION = 92;

function metricDigitFlip(fromText: string, targetText: string, force = false): MetricDigitFlip | undefined {
  if (!/^\d+$/.test(fromText) || !/^\d+$/.test(targetText)) return undefined;

  const slotCount = Math.max(fromText.length, targetText.length);
  const targetLength = targetText.length;
  const leadingSlots = slotCount - targetLength;
  const fromDigits = fromText.padStart(slotCount, ' ');
  const targetDigits = targetText.padStart(slotCount, ' ');
  const slots: MetricDigitFlip['slots'] = Array.from({ length: slotCount }, (_, index) => {
    const fromCharacter = fromDigits[index];
    const targetCharacter = targetDigits[index];
    const baseDelay = index * METRIC_DIGIT_STAGGER;

    if (fromCharacter === targetCharacter && !force) {
      return {
        position: index - leadingSlots,
        frames: [{ character: targetCharacter, delay: baseDelay, kind: 'static' }],
      };
    }

    let intermediateCharacters: string[];
    if (fromCharacter === targetCharacter) {
      intermediateCharacters = [];
    } else if (fromCharacter === ' ') {
      const targetDigit = Number(targetCharacter);
      intermediateCharacters = Array.from({ length: targetDigit }, (_, step) => String(step));
    } else if (targetCharacter === ' ') {
      const fromDigit = Number(fromCharacter);
      intermediateCharacters = Array.from({ length: fromDigit }, (_, step) => String(fromDigit - step - 1));
    } else {
      const fromDigit = Number(fromCharacter);
      const targetDigit = Number(targetCharacter);
      const direction = targetDigit > fromDigit ? 1 : -1;
      intermediateCharacters = Array.from(
        { length: Math.max(0, Math.abs(targetDigit - fromDigit) - 1) },
        (_, step) => String(fromDigit + direction * (step + 1)),
      );
    }

    return {
      position: index - leadingSlots,
      frames: [
        { character: fromCharacter, delay: baseDelay, kind: 'initial' },
        ...intermediateCharacters.map((character, frameIndex) => ({
          character,
          delay: baseDelay + (frameIndex + 1) * METRIC_DIGIT_STEP,
          kind: 'cycle' as const,
        })),
        {
          character: targetCharacter,
          delay: baseDelay + (intermediateCharacters.length + 1) * METRIC_DIGIT_STEP,
          kind: 'final' as const,
        },
      ],
    };
  });

  let terminalFrame: MetricDigitFlip['slots'][number]['frames'][number] | undefined;
  for (const slot of slots) {
    const finalFrame = slot.frames.find((frame) => frame.kind === 'final');
    if (finalFrame && (!terminalFrame || finalFrame.delay >= terminalFrame.delay)) {
      terminalFrame = finalFrame;
    }
  }
  if (terminalFrame) terminalFrame.terminal = true;

  return {
    targetText,
    targetLength,
    duration: (terminalFrame?.delay ?? 0) + METRIC_DIGIT_FINAL_DURATION,
    slots,
  };
}

function metricDigitFlipDuration(fromText: string, targetText: string, force = false): number | undefined {
  return metricDigitFlip(fromText, targetText, force)?.duration;
}

function MorphingMetricValue({
  active,
  contextKey,
  text,
  unit,
  variant,
  duration = 320,
  contextTransitionDelay = 50,
}: MetricFlipValue & { active: boolean; duration?: number; contextTransitionDelay?: number }) {
  const [currentValue, setCurrentValue] = useState<MetricFlipValue>(() => ({
    contextKey,
    text,
    unit,
    variant,
  }));
  const [nextValue, setNextValue] = useState<MetricFlipValue>();
  const [transitioning, setTransitioning] = useState(false);
  const currentValueRef = useRef(currentValue);
  const transitionIdRef = useRef(0);

  useEffect(() => {
    const transitionId = transitionIdRef.current + 1;
    transitionIdRef.current = transitionId;
    let prepareFrame = 0;
    let transitionFrame = 0;
    let commitFrame = 0;
    let delayTimeout = 0;
    let fallbackTimeout = 0;
    let resetFrame = 0;
    const displayedValue = currentValueRef.current;
    const incomingValue = {
      contextKey,
      text,
      unit,
      variant,
    };
    const alreadyCurrent = text === displayedValue.text
      && unit === displayedValue.unit
      && variant === displayedValue.variant
      && contextKey === displayedValue.contextKey;

    // Every prop change cancels the previous animation. Clear any abandoned next
    // face even when the user switches back to the currently displayed metric.
    // Otherwise a fast DPI -> polling -> DPI sequence can leave a stale face and
    // the following polling selection still renders the DPI value.
    if (!active || alreadyCurrent) {
      resetFrame = window.requestAnimationFrame(() => {
        setNextValue(undefined);
        setTransitioning(false);
      });
      return () => window.cancelAnimationFrame(resetFrame);
    }

    const contextChanged = contextKey !== displayedValue.contextKey;
    const transitionDuration = metricDigitFlipDuration(
      displayedValue.text,
      incomingValue.text,
      contextChanged,
    ) ?? duration;
    const prepareTransition = () => {
      prepareFrame = window.requestAnimationFrame(() => {
        setNextValue(incomingValue);
        setTransitioning(false);
        transitionFrame = window.requestAnimationFrame(() => {
          setTransitioning(true);
          fallbackTimeout = window.setTimeout(() => {
            if (transitionIdRef.current !== transitionId) return;
            commitFrame = window.requestAnimationFrame(() => {
              if (transitionIdRef.current !== transitionId) return;
              currentValueRef.current = incomingValue;
              setCurrentValue(incomingValue);
              setNextValue(undefined);
              setTransitioning(false);
            });
          }, transitionDuration + 80);
        });
      });
    };

    // DPI 与回报率共用同一个数字层。切换上下文时让数字翻牌与几何变形
    // （缩放）重叠。缩放使用 cubic-bezier(.32, .72, 0, 1) 缓动曲线，该曲线
    // 极其激进：24.5% 时间（83ms）即完成 77% 视觉进度。50ms 对应视觉约
    // 60%，翻牌在缩放早期就启动，与缩放剩余的大部分时段并行，入场自然
    // 融入缩放过程，避免"等缩放完全结束才开始翻牌"的串行感知。
    if (contextChanged) {
      delayTimeout = window.setTimeout(prepareTransition, contextTransitionDelay);
    } else {
      prepareTransition();
    }

    return () => {
      window.cancelAnimationFrame(prepareFrame);
      window.cancelAnimationFrame(transitionFrame);
      window.cancelAnimationFrame(commitFrame);
      window.cancelAnimationFrame(resetFrame);
      window.clearTimeout(delayTimeout);
      window.clearTimeout(fallbackTimeout);
    };
  }, [active, contextKey, contextTransitionDelay, duration, text, unit, variant]);

  const commitNextValue = () => {
    if (!nextValue) return;
    transitionIdRef.current += 1;
    currentValueRef.current = nextValue;
    setCurrentValue(nextValue);
    setNextValue(undefined);
    setTransitioning(false);
  };

  const finishFlipTransition = (event: React.TransitionEvent<HTMLSpanElement>) => {
    if (
      event.target !== event.currentTarget
      || event.propertyName !== 'opacity'
    ) return;

    commitNextValue();
  };

  const finishDigitFlip = (event: React.AnimationEvent<HTMLSpanElement>) => {
    const target = event.target as HTMLElement;
    if (
      target.dataset.flipLast !== 'true'
    ) return;

    commitNextValue();
  };

  const renderFace = (value: MetricFlipValue, className: string) => {
    const digitFlip = className === 'is-next'
      ? metricDigitFlip(
          currentValue.text,
          value.text,
          currentValue.contextKey !== value.contextKey,
        )
      : undefined;
    return (
      <span
        key={`${value.contextKey}\u0000${value.variant}\u0000${value.text}\u0000${value.unit}`}
        className={`shared-control-metric-face ${className}`}
        data-variant={value.variant}
        onTransitionEnd={className === 'is-next' && !digitFlip ? finishFlipTransition : undefined}
      >
        <strong className="shared-control-metric-text">
          {digitFlip ? (
            <span
              className="metric-flip-digits"
              style={{ '--metric-target-digits': digitFlip.targetLength } as React.CSSProperties}
            >
              <span className="metric-flip-sizer" aria-hidden="true">{digitFlip.targetText}</span>
              {digitFlip.slots.map((slot) => (
                <span
                  key={slot.position}
                  className="metric-flip-digit"
                  style={{ '--metric-digit-position': slot.position } as React.CSSProperties}
                >
                  {slot.frames.map((frame, frameIndex) => (
                    <span
                      key={`${frame.kind}:${frameIndex}:${frame.character}`}
                      className={`metric-flip-digit-face is-${frame.kind}`}
                      data-flip-last={frame.terminal ? 'true' : undefined}
                      style={{ '--metric-digit-delay': `${frame.delay}ms` } as React.CSSProperties}
                      onAnimationEnd={frame.terminal ? finishDigitFlip : undefined}
                      aria-hidden="true"
                    >{frame.character || '\u00a0'}</span>
                  ))}
                </span>
              ))}
            </span>
          ) : value.text}
        </strong>
        {value.unit && <em>{value.unit}</em>}
      </span>
    );
  };

  return (
    <span
      className={`shared-control-metric-value${transitioning ? ' is-transitioning' : ''}`}
      data-transition="flip"
      aria-label={`${(nextValue ?? currentValue).text}${(nextValue ?? currentValue).unit ? ` ${(nextValue ?? currentValue).unit}` : ''}`}
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

function resolveSummaryValue(item: PluginSummaryItem, device: DeviceState): {
  source: string;
  value: unknown;
} {
  const sources = [item.source, ...(item.sourceFallbacks ?? [])];
  for (const source of sources) {
    const value = readPath(device, source);
    if (value !== undefined && value !== null && value !== '') return { source, value };
  }
  return { source: item.source, value: readPath(device, item.source) };
}

function CapabilitySummary({ capability, device }: { capability: PluginCapability; device: DeviceState }) {
  // summary 上限只作用于 polling capability（fixedSlot=2 或 group=polling）。
  // 非 polling capability 的 summary 不被截断。
  const allItems = capability.metadata.summary ?? [];
  const reportedItems = allItems.filter((item) => {
    const { value } = resolveSummaryValue(item, device);
    return value !== undefined && value !== null && value !== '';
  });
  const max = summaryMaxForCapability(capability);
  const { selected: items } = selectSummarySubblocks(reportedItems, max);
  if (items.length === 0) return null;
  return (
    <div
      className="capability-summary"
      aria-label={i18n.t('dashboard.deviceSummary')}
      style={{ gridTemplateColumns: `repeat(${items.length}, minmax(0, 1fr))` }}
    >
      {items.map((item) => {
        const { source, value } = resolveSummaryValue(item, device);
        const option = item.options?.find((candidate) => candidate.value === value);
        const valueLabel = option
          ? resolveLabelKey(option.labelKey, device.pluginId)
          : `${formatFieldValue(value, item.format, i18n.t)}${item.unit ? ` ${item.unit}` : ''}`;
        const label = item.labelKey
          ? resolveLabelKey(item.labelKey, device.pluginId)
          : item.label ?? item.source;
        return (
          <span
            key={`${label}:${source}`}
            className="secondary-control-item"
            style={secondaryRevealStyle(`${capability.id}:${source}:${label}`)}
          >
            {label}
            <FormattedValue value={value} format={item.format} label={valueLabel} />
          </span>
        );
      })}
    </div>
  );
}

function capabilityGroupLabel(group: string, pluginId?: string): string {
  return resolveLabelKey(`capability.group.${group}`, pluginId);
}

function capabilityFieldLabel(key: string, pluginId?: string): string {
  return resolveLabelKey(`capability.field.${key}`, pluginId);
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
    historyIdentity: snapshot.historyIdentity,
    updatedAt: now,
    readStatuses: snapshot.readStatuses,
    mouseReady: snapshot.mouseReady,
    family: snapshot.family,
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
    componentId: battery?.id ?? (snapshot.batteryPercent !== undefined ? 'mouse' : undefined),
  };
}

function connectedBatteryUsageTargets(
  entries: DeviceSnapshotEntry[],
  lowBatteryThreshold: number,
): BatteryUsageConnectedTarget[] {
  const targets: BatteryUsageConnectedTarget[] = [];
  const seen = new Set<string>();
  // entries 只在真实设备快照变化时更新；把这一刻作为即时状态的观测时间，
  // 避免弹窗继续展示历史记录的旧时间戳。
  const latestAt = new Date().toISOString();

  for (const { snapshot } of entries) {
    const deviceName = snapshot.historyIdentity?.displayName ?? snapshot.displayName;
    const batteries = snapshot.batteries?.length
      ? snapshot.batteries
      : snapshot.batteryPercent !== undefined
        ? [{
            id: 'mouse',
            label: '',
            percentage: snapshot.batteryPercent,
            charging: snapshot.charging,
          }]
        : [];

    for (const battery of batteries) {
      const componentId = battery.id;
      const key = `${deviceName.trim().replace(/\s+/g, ' ').toLocaleLowerCase()}\u0000${componentId}`;
      if (seen.has(key)) continue;
      seen.add(key);
      targets.push({
        deviceName,
        componentId,
        latestPercentage: battery.percentage,
        latestCharging: battery.charging,
        latestAt,
        lowBattery: battery.percentage <= lowBatteryThreshold,
      });
    }
  }

  return targets;
}

const auraTimelineStartedAt = typeof performance === 'undefined' ? 0 : performance.now();

function auraPhaseOffset(): string {
  if (typeof performance === 'undefined') return '0ms';
  return `${-Math.max(0, Math.round(performance.now() - auraTimelineStartedAt))}ms`;
}

/** 页面退场层使用静态 HTML 快照。把仍在运动的页头、卡片与 Aura 当前合成帧
 * 写入快照，避免快照层禁用动画后突然跳到关键帧起点或终点。 */
function pageSnapshotHtml(root: HTMLElement): string {
  const clone = root.cloneNode(true) as HTMLElement;
  const selector = [
    '[data-animation="realtime-deformation"] .aura-cloud',
    '[data-animation="realtime-deformation"] .aura-stars',
    '[data-animation="realtime-deformation"] .aura-star',
    '.settings-page > header',
    '.settings-page > .sub-nav',
    '.about-page > header',
    '.log-page > header',
    '.log-page > .log-toolbar',
    '.settings-scroll-content > .card',
    '.settings-scroll-content > section',
    '.log-page > .log-list-wrapper',
  ].join(', ');
  const sources = root.querySelectorAll<HTMLElement>(selector);
  const targets = clone.querySelectorAll<HTMLElement>(selector);

  sources.forEach((source, index) => {
    const target = targets[index];
    if (!target) return;
    const frame = window.getComputedStyle(source);
    target.style.animation = 'none';
    target.style.transform = frame.transform;
    target.style.opacity = frame.opacity;
    target.style.filter = frame.filter;
    target.style.borderRadius = frame.borderRadius;
    target.style.willChange = 'auto';
  });

  return clone.innerHTML;
}

function DeviceAura({ color }: { color?: string }) {
  const [paused, setPaused] = useState(false);
  const [phaseOffset] = useState(auraPhaseOffset);

  useEffect(() => {
    if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
    let disposed = false;
    let syncId = 0;
    const unlisteners: Array<() => void> = [];
    try {
      const win = getCurrentWindow();
      const syncPaused = () => {
        const currentSyncId = ++syncId;
        if (document.hidden) {
          setPaused(true);
          return;
        }
        Promise.all([win.isVisible(), win.isMinimized()])
          .then(([visible, minimized]) => {
            if (!disposed && currentSyncId === syncId) setPaused(!visible || minimized);
          })
          .catch(() => {});
      };
      const register = (promise: Promise<() => void>) => {
        promise.then((unlisten) => {
          if (disposed) unlisten();
          else unlisteners.push(unlisten);
        }).catch(() => {});
      };
      const onVisibilityChange = () => {
        if (document.hidden) {
          ++syncId;
          setPaused(true);
        } else {
          syncPaused();
        }
      };

      syncPaused();
      register(win.onFocusChanged(() => syncPaused()));
      // 最小化不等于不可见；macOS 尤其会保持 isVisible() === true。
      // resize 是 focus 事件早于最小化状态落定时的第二个可靠同步点。
      register(win.onResized(() => syncPaused()));
      document.addEventListener('visibilitychange', onVisibilityChange);

      return () => {
        disposed = true;
        ++syncId;
        document.removeEventListener('visibilitychange', onVisibilityChange);
        unlisteners.splice(0).forEach((unlisten) => unlisten());
      };
    } catch {
      // 非 Tauri 环境忽略
    }
    return undefined;
  }, []);

  return (
    <div className={`device-aura${paused ? ' is-paused' : ''}`} data-animation="realtime-deformation" style={{ '--device-color': color ?? '#b87ab0', '--aura-phase-offset': phaseOffset } as React.CSSProperties} aria-hidden="true">
      <div className="aura-cloud aura-cloud-1" />
      <div className="aura-cloud aura-cloud-2" />
      <div className="aura-cloud aura-cloud-3" />
      <div className="aura-cloud aura-cloud-4" />
      <div className="aura-cloud aura-cloud-5" />
      <div className="aura-stars">
        <div className="aura-star aura-star-1" />
        <div className="aura-star aura-star-2" />
        <div className="aura-star aura-star-3" />
        <div className="aura-star aura-star-4" />
        <div className="aura-star aura-star-5" />
        <div className="aura-star aura-star-6" />
      </div>
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

/** 接收器已连接但鼠标未就位时的等待提示。
 *  复用 EmptyState 的布局风格，但文案明确告知用户接收器已被识别，
 *  正在等待鼠标就位。后端在此期间会高频检测 mouseOnline。 */
function AwaitingMouseState({ deviceName, onRefresh, onOpenSettings }: { deviceName: string; onRefresh: () => void; onOpenSettings: () => void }) {
  const { t } = useTranslation();
  return (
    <main className="empty">
      <DeviceAura />
      <p className="eyebrow">{t('dashboard.awaitingMouse.eyebrow')}</p>
      <h1>{t('dashboard.awaitingMouse.title')}</h1>
      <p>{t('dashboard.awaitingMouse.hint', { name: deviceName })}</p>
      <div className="actions">
        <button onClick={onRefresh}>{t('common.refresh')}</button>
        <button className="secondary" onClick={onOpenSettings}>{t('dashboard.deviceAndDiagnostics')}</button>
      </div>
    </main>
  );
}

function capabilityAvailable(capability: PluginCapability): boolean {
  return capability.available !== false;
}

/** 判断灯光 zone 是否处于开启状态。 */
function zoneLightingEnabled(zone: PluginZone, device: DeviceState): boolean {
  if (!resolveVisibleWhen(zone.visibleWhen, device)) return false;
  // 优先用 switch 字段（inline-toggle 开关）判断。
  const switchField = zone.fields.find((field) => field.switch);
  if (switchField) return resolveSwitchState(switchField, device);
  // 无 switch 时用 effect 字段：值非 0 视为开启。
  const effectField = zone.fields.find((field) => field.lightingRole === 'effect');
  if (effectField) {
    const value = readPath(device, effectField.source);
    return value !== undefined && value !== null && value !== 0;
  }
  // 无开关信息时默认开启。
  return true;
}

/** 读取 zone 中 primary-color 字段的颜色值。 */
function zonePrimaryColor(zone: PluginZone, device: DeviceState): string | undefined {
  const colorField = zone.fields.find(
    (field) => field.lightingRole === 'primary-color' && (field.format === 'color' || field.editor === 'modal-color'),
  ) ?? zone.fields.find((field) => field.format === 'color' || field.editor === 'modal-color');
  if (!colorField) return undefined;
  const value = readPath(device, colorField.source);
  return typeof value === 'string' ? value : undefined;
}

/**
 * 从插件声明中取得用于宿主装饰的颜色，不依赖任何厂商状态字段名。
 * 主题色规则：accentSource 指向主灯光（鼠标）；主灯光开 → 主灯光色；
 * 主灯光关 + 其他灯光（接收器）开 → 其他灯光色；都关 → 主灯光色（默认）。
 */
function declaredAccentColor(device: DeviceState): string | undefined {
  for (const capability of device.pluginCapabilities.filter(capabilityAvailable)) {
    const source = capability.metadata.accentSource;
    if (!source) continue;
    const zones = capability.metadata.zones ?? [];
    // 找到 accentSource 对应的主 zone。
    const primaryZone = zones.find(
      (zone) => resolveVisibleWhen(zone.visibleWhen, device)
        && zone.fields.some((field) => field.source === source && (field.format === 'color' || field.editor === 'modal-color')),
    );
    if (!primaryZone) {
      const value = readPath(device, source);
      if (typeof value === 'string') return value;
      continue;
    }
    // 主灯光开 → 主灯光色。
    if (zoneLightingEnabled(primaryZone, device)) {
      const value = readPath(device, source);
      if (typeof value === 'string') return value;
    }
    // 主灯光关：找其他开启的 zone。
    for (const zone of zones) {
      if (zone === primaryZone) continue;
      if (!resolveVisibleWhen(zone.visibleWhen, device)) continue;
      if (zoneLightingEnabled(zone, device)) {
        const color = zonePrimaryColor(zone, device);
        if (color) return color;
      }
    }
    // 都关 → 主灯光色（默认）。
    const value = readPath(device, source);
    if (typeof value === 'string') return value;
    return undefined;
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
      const colorPath = layout.colorSource ?? layout.dotsSource;
      const stages = colorPath ? readPath(device, colorPath) as DpiStage[] | undefined : undefined;
      const active = stages?.find((stage) => stage.enabled && stage.active) ?? stages?.find((stage) => stage.enabled);
      if (active?.color) return active.color;
    }
  }
  return undefined;
}

/** 按 zone id 从设备插件声明中定位 Zone（跨 capability 查找）。 */
function findZoneById(deviceState: DeviceState, zoneId: string): PluginZone | undefined {
  for (const capability of deviceState.pluginCapabilities) {
    const zone = (capability.metadata.zones ?? []).find((candidate) => candidate.id === zoneId);
    if (zone) return zone;
  }
  return undefined;
}

/** 提取 Zone 的灯光可读状态，供 Mutation 成功后的目标值确认使用。 */
function extractZoneLightingState(zone: PluginZone, deviceState: DeviceState): ZoneLightingState {
  const effectField = zone.fields.find((field) => field.lightingRole === 'effect');
  return {
    enabled: zoneLightingEnabled(zone, deviceState),
    color: zonePrimaryColor(zone, deviceState),
    effectValue: effectField ? readPath(deviceState, effectField.source) : undefined,
  };
}

/** 灯光 Mutation 成功后的确认入口：pending 存在时校验目标值并播放（P1-2）。 */
function confirmLightingMutation(attentionId: number | undefined, before: DeviceState, after: DeviceState): void {
  if (attentionId === undefined) return;
  const pending = peekPendingLightingAttention(attentionId);
  if (!pending) return;
  const zoneStateOf = (deviceState: DeviceState): ZoneLightingState | undefined => {
    const zone = findZoneById(deviceState, pending.zoneId);
    return zone ? extractZoneLightingState(zone, deviceState) : undefined;
  };
  confirmPendingLightingAttention(attentionId, {
    before: zoneStateOf(before),
    after: zoneStateOf(after),
  });
}

function capabilityRuntimePending(capability: PluginCapability): boolean {
  return capability.metadata._miraRuntimePending === true;
}

function usesSoftwareDpiLayout(layout: NonNullable<PluginCapability['metadata']['stageLayout']>, device: DeviceState): boolean {
  if (isSoftwareDpiLayout(layout)) return true;
  if (layout.mode !== 'auto') return false;
  const reportedStages = layout.dotsSource
    ? (readPath(device, layout.dotsSource) as DpiStage[] | undefined) ?? []
    : [];
  const enabledCount = reportedStages.filter((stage) => stage.enabled).length;
  return enabledCount < 2 || !resolveMutation(layout.selectMutation, device.writableMutations);
}

function deviceRuntimePending(device: DeviceState): boolean {
  return device.pluginCapabilities.some(capabilityRuntimePending);
}

function capabilityHasControlContent(capability: PluginCapability, device: DeviceState): boolean {
  if (capabilityRuntimePending(capability)) return true;

  const layout = resolveStageLayout(capability);
  if (layout) {
    if (usesSoftwareDpiLayout(layout, device)) {
      return softwareDpiCurrentValue(layout, device) !== undefined
        && (layout.defaultValues?.length ?? 0) >= 2;
    }
    if (!layout.dotsSource) return false;
    const stages = readPath(device, layout.dotsSource);
    return Array.isArray(stages) && stages.some((stage) => (
      stage && typeof stage === 'object' && (stage as DpiStage).enabled
    ));
  }

  if (capability.metadata.zones) {
    return resolveZones(capability, device).some((zone) => (
      zone.fields.some((field) => fieldHasReportedValue(field, device))
    ));
  }

  return (capability.metadata.fields ?? [])
    .some((field) => fieldHasReportedValue(field, device));
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
      return <Gauge weight="regular" data-plugin-icon="gauge" />;
    case 'lightbulb':
      return <Lightbulb weight="regular" data-plugin-icon="lightbulb" />;
    case 'profile':
      return <SlidersHorizontal weight="regular" data-plugin-icon="profile" />;
    case 'settings':
      return <Gear weight="regular" data-plugin-icon="settings" />;
    case 'timer':
      return <Timer weight="regular" data-plugin-icon="timer" />;
    case 'wave':
      return <WaveSine weight="regular" data-plugin-icon="wave" />;
    case 'info':
    default:
      return <Info weight="regular" data-plugin-icon="info" />;
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
  const range = resolveFieldRange(field, device);
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

/// 统一字段写入口。第三个可选参数是灯光 Mutation 的 pending 登记 id（P1-2），
/// 成功返回快照后由 runMutation 统一校验并播放灯光 Beam。
type RunMutation = (mutation: string, params: Record<string, unknown>, attentionId?: number) => Promise<void>;

/// 开关字段（inline-toggle + field.switch）。跟踪上次非 off 值用于恢复。
function SwitchField({ field, device, writeBusy, runMutation, attentionZoneId }: {
  field: PluginField;
  device: DeviceState;
  writeBusy: boolean;
  runMutation: RunMutation;
  attentionZoneId?: string;
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
    const nextValue = resolveSwitchNextValue(field, device, restoreRef.current);
    if (nextValue !== undefined) {
      // 灯光 Zone 的开关在「关 → 开」时登记 power-on 反馈；关闭不反馈。
      let attentionId: number | undefined;
      if (attentionZoneId && !resolveSwitchState(field, device)) {
        attentionId = registerLightingAttention(attentionZoneId, 'power-on', nextValue);
      }
      void runMutation(mutation, resolveFieldMutationParams(field, device, nextValue), attentionId);
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

/// inline-range 滑杆组件。本地预览 + debounced 提交，避免拖动时每个 onChange 都发送 HID 写请求。
/// 拖动期间只更新本地预览值（即时 UI 反馈），停止变化 150ms 后才提交一次 mutation。
function InlineRangeSlider({ range, value, disabled, format, onChange }: {
  range: RangeSpec;
  value: number;
  disabled: boolean;
  format: PluginField['format'];
  onChange: (numericValue: number) => void;
}) {
  const [pendingValue, setPendingValue] = useState<number>();
  const [lastSeenValue, setLastSeenValue] = useState(value);
  const debounceRef = useRef(0);

  // React 推荐的 "store previous props in state" 模式：prop 变化时同步本地 state。
  // 不使用 useEffect（避免 set-state-in-effect），不使用 ref during render（避免 refs 规则）。
  if (lastSeenValue !== value) {
    setLastSeenValue(value);
    setPendingValue(undefined);
  }

  // value 变化或卸载时取消 pending debounce，避免过期 onChange 调用。
  useEffect(() => () => window.clearTimeout(debounceRef.current), [value]);

  const displayedValue = pendingValue ?? value;
  const sliderPercent = ((displayedValue - range.min) / (range.max - range.min)) * 100;

  return (
    <div className="as-slider-wrap">
      <input
        type="range"
        className="as-slider"
        min={range.min}
        max={range.max}
        step={range.step ?? 1}
        value={displayedValue}
        disabled={disabled}
        style={{ '--slider-percent': `${sliderPercent}%` } as React.CSSProperties}
        onChange={(e) => {
          const numericValue = Number(e.target.value);
          setPendingValue(numericValue);
          window.clearTimeout(debounceRef.current);
          debounceRef.current = window.setTimeout(() => {
            // value 是闭包捕获的 prop（用户拖动时的值）。
            // 若 value 已通过外部更新变为 numericValue，effect 清理会取消此 timeout。
            if (numericValue !== value) {
              onChange(numericValue);
            }
          }, 150);
        }}
      />
      <span className="as-slider-value">{formatFieldValue(displayedValue, format, i18n.t)}</span>
    </div>
  );
}

/// 按 field.editor 渲染字段控件。声明式，不含字段级特殊分支。
/// attentionZoneId 存在时表示该字段属于灯光 Zone（P1-2：成功写入才反馈）。
function FieldRenderer({ field, device, writeBusy, runMutation, attentionZoneId }: {
  field: PluginField;
  device: DeviceState;
  writeBusy: boolean;
  runMutation: RunMutation;
  attentionZoneId?: string;
}) {
  const [editing, setEditing] = useState(false);

  if (!fieldHasReportedValue(field, device)) return null;

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
        return <SwitchField field={field} device={device} writeBusy={writeBusy} runMutation={runMutation} attentionZoneId={attentionZoneId} />;
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
                onClick={() => {
                  if (!mutation) return;
                  // 灯光 Zone 的 effect 字段：写入前登记，成功确认后反馈。
                  const attentionId = attentionZoneId && field.lightingRole === 'effect'
                    ? registerLightingAttention(attentionZoneId, 'effect-applied', option.value)
                    : undefined;
                  void runMutation(mutation, resolveFieldMutationParams(field, device, option.value), attentionId);
                }}
              >{resolveLabelKey(option.labelKey, device.pluginId)}</button>
            ))}
          </div>
        </>
      );
    }

    case 'inline-range': {
      const range = field.range;
      if (!range) {
        return (
          <>
            <span>{label}</span>
            <FormattedValue value={value} format={field.format} label={valueLabel} className="plugin-current-value" />
          </>
        );
      }
      const numericValue = typeof value === 'number' ? value : range.min;
      return (
        <>
          <span>{label}</span>
          <InlineRangeSlider
            range={range}
            value={numericValue}
            disabled={!writable}
            format={field.format}
            onChange={(v) => mutation && applyMutation(mutation, resolveFieldMutationParams(field, device, v))}
          />
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
  runMutation: RunMutation;
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
  runMutation: RunMutation;
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

  if (!fieldHasReportedValue(field, device)) return null;

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
            const nextValue = resolveSwitchNextValue(field, device, restoreRef.current);
            if (nextValue !== undefined) apply(nextValue);
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
  runMutation: RunMutation;
}) {
  const fields = (capability.metadata.fields ?? []).filter((field) => fieldHasReportedValue(field, device));
  const label = resolveLabelKey(capability.labelKey, device.pluginId);

  if (fields.length === 0) return null;

  return (
    <div className="control-reading mode-reading plugin-control-reading">
      <PluginIconView name={placementsFor(capability, 'control')[0]?.icon} device={device} />
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
  runMutation: RunMutation;
}) {
  const layout = resolveStageLayout(capability);
  const [editingStage, setEditingStage] = useState<number | null>(null);
  const runtimePending = capabilityRuntimePending(capability);
  const software = layout ? usesSoftwareDpiLayout(layout, device) : false;
  const currentSoftwareDpi = layout && software ? softwareDpiCurrentValue(layout, device) : undefined;
  const softwareKey = layout && software ? softwareDpiStageKey(device, capability.id) : '';
  const [softwareStates, setSoftwareStates] = useState<Record<string, SoftwareDpiStageState>>({});

  if (!layout) return null;

  const softwareState = software
    ? softwareStates[softwareKey] ?? loadSoftwareDpiStages(softwareDpiStorage(), softwareKey, layout, currentSoftwareDpi)
    : { values: [], selectedIndex: 0 };

  const allStages = software
    ? softwareDpiStages(softwareState, currentSoftwareDpi)
    : ((layout.dotsSource ? readPath(device, layout.dotsSource) : undefined) as DpiStage[] | undefined) ?? [];
  const stages = allStages.filter((stage) => stage.enabled);
  const displayedStages = stages.slice(0, 8);
  const current = stages.find((stage) => stage.active);
  const reportedSoftwareIndex = software && currentSoftwareDpi !== undefined
    ? softwareState.values.indexOf(currentSoftwareDpi)
    : -1;
  const effectiveSoftwareIndex = reportedSoftwareIndex >= 0
    ? reportedSoftwareIndex
    : softwareState.selectedIndex;
  const currentStageNumber = software
    ? Math.max(1, effectiveSoftwareIndex + 1)
    : Math.max(1, stages.findIndex((stage) => stage.active) + 1);
  const activeDpi = software ? currentSoftwareDpi ?? 0 : current?.value ?? stages[0]?.value ?? 0;

  const selectMutation = software
    ? resolveMutation(layout.setMutation, device.writableMutations)
    : resolveMutation(layout.selectMutation, device.writableMutations);
  const setMutation = resolveMutation(layout.setMutation, device.writableMutations);
  const selectWritable = Boolean(selectMutation);
  const setWritable = Boolean(setMutation);
  const range: RangeSpec = (() => {
    if (layout.rangeSource) {
      const raw = readPath(device, layout.rangeSource);
      if (typeof raw === 'number' && Number.isFinite(raw)) {
        const offset = layout.rangeMaxOffset ?? 0;
        return { min: layout.range.min, max: raw + offset, step: layout.range.step };
      }
    }
    return layout.range;
  })();

  const stageField: PluginField = {
    id: 'stage-value',
    source: layout.valueSource ?? layout.currentValueSource ?? '',
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
        {/* The shared metric layer owns the visible transition. Keep this hidden
            geometry anchor synchronous so ResizeObserver never follows a stale
            intermediate width after the visible number has already settled. */}
        <strong>{String(activeDpi || (runtimePending ? '—' : i18n.t('common.notReported')))}</strong><em>DPI</em>
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
                onClick={() => {
                  if (!selectMutation) return;
                  if (software) {
                    const next = { ...softwareState, selectedIndex: index };
                    setSoftwareStates((states) => ({ ...states, [softwareKey]: next }));
                    saveSoftwareDpiStages(softwareDpiStorage(), softwareKey, next);
                    void runMutation(selectMutation, {
                      ...(layout.stageParam ? { [layout.stageParam]: 1 } : {}),
                      [layout.valueParam ?? 'value']: stage.value,
                    });
                  } else {
                    void runMutation(selectMutation, { [layout.selectParam ?? 'value']: stageNumber });
                  }
                }}
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
              if (software) {
                const nextValues = softwareState.values.map((item, index) => index === editingStage - 1 ? Number(value) : item);
                const editingIndex = editingStage - 1;
                const writesCurrentStage = effectiveSoftwareIndex === editingIndex;
                const next = {
                  ...softwareState,
                  values: nextValues,
                  selectedIndex: writesCurrentStage ? editingIndex : softwareState.selectedIndex,
                };
                setSoftwareStates((states) => ({ ...states, [softwareKey]: next }));
                saveSoftwareDpiStages(softwareDpiStorage(), softwareKey, next);
                if (writesCurrentStage) {
                  void runMutation(setMutation, {
                    ...(layout.stageParam ? { [layout.stageParam]: 1 } : {}),
                    [layout.valueParam ?? 'value']: value,
                  });
                }
              } else {
                void runMutation(setMutation, {
                  [layout.stageParam ?? 'stage']: editingStage,
                  [layout.valueParam ?? 'value']: value,
                });
              }
            }
            setEditingStage(null);
          }}
        />
      )}
    </>
  );
}

/// Advanced Settings 条目类型。支持 field / stageLayout / zone / summary 四种形态，
/// 复用现有 FieldRenderer / StageLayout / ZoneRenderer 渲染。
/// polling overflow 以 summary 条目进入 Advanced Settings。
/// field entry 保留 zoneId/zoneLabelKey 以正确区分多 zone 同名字段。
type AdvancedSettingsEntry =
  | { type: 'field'; capability: PluginCapability; field: PluginField; zoneId?: string; zoneLabelKey?: string }
  | { type: 'stageLayout'; capability: PluginCapability }
  | { type: 'zone'; capability: PluginCapability }
  | { type: 'summary'; capability: PluginCapability; item: PluginSummaryItem };

/// Advanced Settings 模态窗口。展示未进入 Dashboard 首页的可写字段和 details 字段，
/// 按 advancedSection 分组。可编辑字段点击后打开 FieldEditModal。
function AdvancedSettingsModal({ groups, device, writeBusy, onClose, onEditField, runMutation }: {
  groups: { section: NonNullable<PluginField['advancedSection']>; entries: AdvancedSettingsEntry[] }[];
  device: DeviceState;
  writeBusy: boolean;
  onClose: () => void;
  onEditField: (capability: PluginCapability, field: PluginField) => void;
  runMutation: RunMutation;
}) {
  const sectionLabel = (section: NonNullable<PluginField['advancedSection']>): string => {
    const key = `advancedSettings.section.${section}`;
    return i18n.exists(key) ? i18n.t(key) : section;
  };

  // 分区图标：由 UI 根据分区语义选择展示样式（插件仍控制字段与行为）。
  const sectionIcon = (section: NonNullable<PluginField['advancedSection']>) => {
    switch (section) {
      case 'performance': return <Gauge weight="regular" />;
      case 'lighting-details': return <Lightbulb weight="regular" />;
      case 'profiles': return <Stack weight="regular" />;
      case 'buttons': return <Mouse weight="regular" />;
      case 'power': return <BatteryHigh weight="regular" />;
      case 'sensor': return <Crosshair weight="regular" />;
      case 'device': return <Cpu weight="regular" />;
    }
  };

  const entryLabel = (entry: AdvancedSettingsEntry): string => {
    if (entry.type === 'stageLayout' || entry.type === 'zone') {
      return resolveLabelKey(entry.capability.labelKey, device.pluginId);
    }
    if (entry.type === 'summary') {
      return entry.item.labelKey
        ? resolveLabelKey(entry.item.labelKey, device.pluginId)
        : (entry.item.label ?? entry.item.source);
    }
    const zonePrefix = entry.zoneLabelKey ? `${resolveLabelKey(entry.zoneLabelKey, device.pluginId)} ` : '';
    return `${zonePrefix}${resolveFieldLabel(entry.field, device, device.pluginId)}`;
  };

  const totalCount = groups.reduce((sum, group) => sum + group.entries.length, 0);

  const [query, setQuery] = useState('');
  const trimmedQuery = query.trim().toLowerCase();
  const filteredGroups = trimmedQuery
    ? groups
        .map((group) => ({ ...group, entries: group.entries.filter((entry) => entryLabel(entry).toLowerCase().includes(trimmedQuery)) }))
        .filter((group) => group.entries.length > 0)
    : groups;

  const bodyRef = useRef<HTMLDivElement>(null);
  const bodyContentRef = useRef<HTMLDivElement>(null);
  const { canScrollUp, canScrollDown } = useScrollFadeState(bodyRef, bodyContentRef);

  const renderEntry = (entry: AdvancedSettingsEntry, index: number) => {
    if (entry.type === 'stageLayout') {
      return (
        <li key={`${entry.capability.id}:stageLayout`} className="as-item as-item-stage-layout">
          <StageLayout capability={entry.capability} device={device} writeBusy={writeBusy} runMutation={runMutation} />
        </li>
      );
    }
    if (entry.type === 'zone') {
      return (
        <li key={`${entry.capability.id}:zone`} className="as-item as-item-zone">
          <ZoneRenderer capability={entry.capability} device={device} writeBusy={writeBusy} runMutation={runMutation} />
        </li>
      );
    }
    if (entry.type === 'summary') {
      // polling overflow 以只读 summary 条目展示。
      const { capability, item } = entry;
      const { value } = resolveSummaryValue(item, device);
      const option = item.options?.find((candidate) => candidate.value === value);
      const valueLabel = option
        ? resolveLabelKey(option.labelKey, device.pluginId)
        : `${formatFieldValue(value, item.format, i18n.t)}${item.unit ? ` ${item.unit}` : ''}`;
      const label = item.labelKey
        ? resolveLabelKey(item.labelKey, device.pluginId)
        : (item.label ?? item.source);
      return (
        <li key={`${capability.id}:${item.source}:${index}`} className="as-item">
          <span className="as-item-label">{label}</span>
          <span className="as-item-right">
            <span className="as-item-value">{valueLabel || i18n.t('common.notReported')}</span>
          </span>
        </li>
      );
    }
    // field entry。React key 包含 zoneId，label 包含 zone 前缀以区分多 zone 同名字段。
    const { capability, field, zoneId, zoneLabelKey } = entry;
    const label = resolveFieldLabel(field, device, device.pluginId);
    const mutation = resolveMutation(field.mutation, device.writableMutations);
    const value = readPath(device, field.source);
    const editable = Boolean(mutation) && !writeBusy;
    const itemKey = `${capability.id}:${zoneId ?? 'root'}:${field.id}:${index}`;
    const zoneLabelNode = zoneLabelKey
      ? <span className="as-item-label-zone">{resolveLabelKey(zoneLabelKey, device.pluginId)}</span>
      : null;

    // inline-toggle + switch：复用设置页 .toggle/.toggle-knob 开关样式，保持主题色一致。
    if (field.editor === 'inline-toggle' && field.switch) {
      const isOn = resolveSwitchState(field, device);
      const nextValue = resolveSwitchNextValue(field, device);
      return (
        <li key={itemKey} className="as-item">
          <span className="as-item-label">{zoneLabelNode}{label}</span>
          <span className="as-item-right">
            <button
              type="button"
              role="switch"
              aria-checked={isOn}
              aria-label={label}
              className={`toggle${isOn ? ' on' : ''}`}
              disabled={!editable}
              onClick={() => {
                if (mutation && nextValue !== undefined) {
                  void runMutation(mutation, resolveFieldMutationParams(field, device, nextValue));
                }
              }}
            >
              <span className="toggle-knob" />
            </button>
          </span>
        </li>
      );
    }

    // inline-range：滑杆（复用 InlineRangeSlider 组件，本地预览 + debounced 提交）。
    if (field.editor === 'inline-range') {
      const range = field.range;
      if (!range) {
        const valueText = formatFieldValue(value, field.format, i18n.t);
        return (
          <li key={itemKey} className="as-item">
            <span className="as-item-label">{zoneLabelNode}{label}</span>
            <span className="as-item-right">
              <span className="as-item-value readonly">{valueText || i18n.t('common.notReported')}</span>
            </span>
          </li>
        );
      }
      const numericValue = typeof value === 'number' ? value : range.min;
      return (
        <li key={itemKey} className="as-item">
          <span className="as-item-label">{zoneLabelNode}{label}</span>
          <span className="as-item-right">
            <InlineRangeSlider
              range={range}
              value={numericValue}
              disabled={!editable}
              format={field.format}
              onChange={(v) => mutation && void runMutation(mutation, resolveFieldMutationParams(field, device, v))}
            />
          </span>
        </li>
      );
    }

    // static-readonly：只读值；布尔值用徽章展示（如校准状态）。
    if (field.editor === 'static-readonly') {
      const isBoolean = typeof value === 'boolean';
      const valueText = formatFieldValue(value, field.format, i18n.t);
      return (
        <li key={itemKey} className="as-item">
          <span className="as-item-label">{zoneLabelNode}{label}</span>
          <span className="as-item-right">
            {isBoolean ? (
              <span className={`as-badge${value === true ? ' accent' : ''}`}>{valueText}</span>
            ) : (
              <span className="as-item-value readonly">{valueText || i18n.t('common.notReported')}</span>
            )}
          </span>
        </li>
      );
    }

    // 其他非 modal inline editor（inline-segmented / inline-value / inline-action / inline-toggle without switch）
    // → 复用 FieldRenderer，但外层用 as-item 结构承载布局。
    if (!field.editor.startsWith('modal-')) {
      return (
        <li key={itemKey} className="as-item as-item-inline">
          <FieldRenderer field={field} device={device} writeBusy={writeBusy} runMutation={runMutation} />
        </li>
      );
    }

    // modal-* editor：label-value 按钮，点击打开编辑模态框。
    const valueLabel = resolveFieldValueLabel(field, device, device.pluginId);
    const valueText = valueLabel ?? formatFieldValue(value, field.format, i18n.t);
    const isColor = field.format === 'color' || valueLooksColor(value);
    return (
      <li key={itemKey} className="as-item editable">
        <button
          type="button"
          className="as-item-btn"
          disabled={!editable}
          onClick={() => editable && onEditField(capability, field)}
        >
          <span className="as-item-label">{zoneLabelNode}{label}</span>
          <span className="as-item-right">
            {isColor && typeof value === 'string' ? (
              <>
                <span className="as-color-dot" style={{ background: value }} />
                <span className="as-item-value">{valueText}</span>
              </>
            ) : (
              <span className="as-item-value">{valueText || i18n.t('common.notReported')}</span>
            )}
          </span>
        </button>
      </li>
    );
  };

  return (
    <Modal
      open
      onClose={onClose}
      title={i18n.t('advancedSettings.title')}
      size="medium"
      className="advanced-settings-modal"
      backdropClassName="advanced-settings-backdrop"
    >
      <div className="as-header">
        <div className="as-header-icon"><SlidersHorizontal weight="regular" /></div>
        <div className="as-header-text">
          <h2 className="as-header-title">{i18n.t('advancedSettings.title')}</h2>
          <p className="as-header-sub">{device.name} · {i18n.t('advancedSettings.itemCount', { count: totalCount })}</p>
        </div>
        <button type="button" className="icon-button" aria-label={i18n.t('common.close')} onClick={onClose}>
          <X weight="regular" />
        </button>
      </div>
      <div className="as-search">
        <div className="as-search-input-wrap">
          <MagnifyingGlass weight="regular" />
          <input
            type="text"
            className="as-search-input"
            placeholder={i18n.t('advancedSettings.searchPlaceholder')}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
      </div>
      <div ref={bodyRef} className={`as-body${canScrollUp ? ' scroll-fade-top' : ''}${canScrollDown ? ' scroll-fade-bottom' : ''}`}>
        <div ref={bodyContentRef} className="as-body-content">
          {filteredGroups.length === 0 ? (
            <p className="as-empty">{i18n.t('advancedSettings.searchNoResults')}</p>
          ) : filteredGroups.map((group) => (
            <section key={group.section} className="advanced-settings-section" data-section={group.section}>
              <div className="as-section-header">
                <span className="as-section-icon">{sectionIcon(group.section)}</span>
                <span className="as-section-title">{sectionLabel(group.section)}</span>
              </div>
              {group.section === 'device' && (
                <div className="as-device-info">
                  <div className="as-device-info-icon"><Mouse weight="regular" /></div>
                  <div className="as-device-info-text">
                    <div className="as-device-info-name">{device.name}</div>
                    <div className="as-device-info-meta">
                      {[
                        typeof device.state.firmwareVersion === 'string' ? `${i18n.t('mock.firmware')} ${device.state.firmwareVersion}` : '',
                        typeof device.state.serialNumber === 'string' ? `SN ${device.state.serialNumber}` : '',
                      ].filter(Boolean).join(' · ')}
                    </div>
                  </div>
                </div>
              )}
              <ul className={`advanced-settings-list${group.section === 'device' ? ' as-device-list' : ''}`}>
                {group.entries.map((entry, index) => renderEntry(entry, index))}
              </ul>
            </section>
          ))}
        </div>
      </div>
      <div className="as-footer">
        <span className="as-footer-hint">{i18n.t('advancedSettings.changesImmediate')}</span>
        <button type="button" className="as-footer-btn secondary" disabled>
          {i18n.t('advancedSettings.reset')}
        </button>
      </div>
    </Modal>
  );
}

/// 灯光区域渲染。多区域时显示子标签页，单区域时直接渲染字段网格。
function ZoneRenderer({ capability, device, writeBusy, runMutation }: {
  capability: PluginCapability;
  device: DeviceState;
  writeBusy: boolean;
  runMutation: RunMutation;
}) {
  const zones = resolveZones(capability, device);
  const [activeZoneId, setActiveZoneId] = useState<string>('');
  const [editingColorZoneId, setEditingColorZoneId] = useState<string>();

  // 灯光区域标题（鼠标灯光/接收器灯光）的淡入淡出状态机。
  // Hooks 必须在条件返回之前调用，所以 activeZone 在此安全派生。
  const activeZone = zones.length > 0 ? (zones.find((z) => z.id === activeZoneId) ?? zones[0]) : undefined;

  // 灯光 Zone 的 Attention 表面：只渲染本 Zone 发生的真实状态迁移。
  const zoneAttention = useAttentionFeedback(`lighting:${activeZone?.id ?? ''}`);
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

  // 主颜色入口必须来自 selector 选出的最高优先级、
  // 当前可见的 primary-color 字段。禁止从 raw fields 中取第一个 modal-color，
  // 那会选中不可见的 Protocol A color 而不是 AM35 的 am35-color。
  // 先过滤 reported 和 presentation，再用 selectLightingSubblocks 选择。
  const lightingCandidates = activeZone.fields.filter((field) =>
    fieldHasReportedValue(field, device) && field.presentation !== 'details',
  );
  const lightingSelection = selectLightingSubblocks(lightingCandidates);
  // 主颜色入口必须来自 selector 选出的最高优先级
  // primary-color 字段，且必须是真正的颜色字段（modal-color 或 format=color）。
  // 非颜色字段（如 modal-select）即使声明了 lightingRole=primary-color 也保留在
  // visibleFields 中作为普通子块渲染，不提取为色板。
  const selectorPrimaryColor = lightingSelection.primaryColor;
  const isColorField = (field: PluginField | undefined): boolean =>
    Boolean(field) && (field!.editor === 'modal-color' || field!.format === 'color');
  const colorField = isColorField(selectorPrimaryColor) ? selectorPrimaryColor : undefined;
  const visibleFieldsRaw = lightingSelection.selected;
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

  // 顶部灯带与最右普通颜色子块并存。
  // - 顶部灯带继续使用 colorField 作为可点击入口（不参与 grid 列数与子块计数）；
  // - 普通 rows 直接使用 selector 的最终顺序（已包含 primaryColor 在最右），
  //   primaryColor 通过 FieldRenderer + modal-color + lighting-row-slot 渲染，
  //   保留普通颜色子块样式，不从 rows 中删除；
  // - 两处共享同一 colorField / colorMutation / zoneColor / device 状态，
  //   任意一处写入成功后另一处立即同步，写入失败时两处都保持原色。
  // - 顶部灯带不计入 6 个普通子块上限；grid 列数与 compact 阈值仅基于 visibleFields。
  const visibleFields = visibleFieldsRaw;
  // 条件显示的次级区域通常是接收器等附属对象；字段较多时使用与旧界面一致
  // 的紧凑密度。这里仅依赖 zone 的声明形态，不依赖 zone id。
  // 灯带不计入子块数量，compact 阈值仅基于真实子块数。
  const compactDetailGrid = Boolean(activeZone.visibleWhen) && visibleFields.length >= 5;
  const lightingColumnCount = Math.max(visibleFields.length, 1);

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
      {colorField && (
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
      )}
      <div className="lighting-sections" aria-label={i18n.t('dashboard.lightingGroups')}>
        <AttentionSurface
          className={`lighting-group lighting-group-${activeZone.id}${compactDetailGrid ? ' is-compact' : ''}`}
          beam={zoneAttention.beam}
        >
          <p className="lighting-group-title" data-title-phase={titlePhase}>{displayedLabel}</p>
          <div
            className={`lighting-rows${compactDetailGrid ? ' is-compact' : ''}`}
            style={{ gridTemplateColumns: `repeat(${lightingColumnCount}, minmax(0, 1fr))` }}
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
                  attentionZoneId={activeZone.id}
                />
              </div>
            ))}
          </div>
        </AttentionSurface>
      </div>
      {colorField && colorMutation && editingColorZoneId === activeZone.id && (
        <FieldEditModal
          key={`${activeZone.id}:${colorField.id}`}
          field={colorField}
          device={device}
          writeBusy={writeBusy}
          onClose={() => setEditingColorZoneId(undefined)}
          onApply={(value) => {
            // 灯光 Zone 的颜色写入：写入前登记，runMutation 成功并确认目标值后反馈。
            const attentionId = registerLightingAttention(activeZone.id, 'color-applied', value);
            void runMutation(colorMutation, resolveFieldMutationParams(colorField, device, value), attentionId);
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
  const baseDisplay = resolveStatusDisplay(capability);
  if (!baseDisplay) return null;
  // 解析 variants，获取当前设备状态下生效的显示来源。
  const display = resolveStatusDisplayVariant(baseDisplay, device);
  if (!display.valueSource) return null;

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
  runMutation: RunMutation;
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
  const fields = (capability.metadata.fields ?? []).filter((field) => fieldHasReportedValue(field, device));
  const metricField = fields.length === 1 && fields[0].format === 'hertz' ? fields[0] : undefined;
  if (metricField) {
    return <MetricField capability={capability} field={metricField} device={device} writeBusy={writeBusy} runMutation={runMutation} />;
  }
  return <GenericCapabilityControl capability={capability} device={device} writeBusy={writeBusy} runMutation={runMutation} />;
}

function isComplexValue(value: unknown): boolean {
  return value !== null && typeof value === 'object';
}

/** Extract the actual capability group names referenced by a capability's fields/zones/summary. */
function capabilitySourceGroups(capability: PluginCapability): string[] {
  const groups = new Set<string>();
  const sources: string[] = [];
  for (const field of capability.metadata.fields ?? []) {
    if (field.source) sources.push(field.source);
    if (field.labelSource) sources.push(field.labelSource);
  }
  for (const zone of capability.metadata.zones ?? []) {
    for (const field of zone.fields) {
      if (field.source) sources.push(field.source);
      if (field.labelSource) sources.push(field.labelSource);
    }
  }
  for (const item of capability.metadata.summary ?? []) {
    if (item.source) sources.push(item.source);
    for (const fallback of item.sourceFallbacks ?? []) sources.push(fallback);
  }
  for (const source of sources) {
    const match = source.match(/^capabilities\.([^.]+)$/);
    if (match) groups.add(match[1]);
  }
  return [...groups];
}

function ReadStatusBadge({ status }: { status?: ReadStatus }) {
  if (!status) return null;
  if (status === 'ok') return <span className="read-status-badge ok" title={i18n.t('dashboard.readOk')} aria-label={i18n.t('dashboard.readOk')} />;
  if (status === 'skipped') return <span className="read-status-badge skipped" title={i18n.t('dashboard.readSkipped')} aria-label={i18n.t('dashboard.readSkipped')} />;
  if (status === 'not-supported') return <span className="read-status-badge not-supported" title={i18n.t('dashboard.readNotSupported')} aria-label={i18n.t('dashboard.readNotSupported')} />;
  if (typeof status === 'object' && 'failed' in status) {
    return <span className="read-status-badge failed" title={status.failed} aria-label={i18n.t('dashboard.readFailed')} />;
  }
  return null;
}

function DeviceDetails({ device, deviceKey, onClose }: { device: DeviceState; deviceKey: string; onClose: () => void }) {
  const [copyState, setCopyState] = useState<'idle' | 'copied' | 'copying'>('idle');
  const [diagCopyState, setDiagCopyState] = useState<'idle' | 'copied' | 'copying'>('idle');
  const [refreshingDetails, setRefreshingDetails] = useState(false);
  const [protoDiagActive, setProtoDiagActive] = useState(false);
  const [protoDiagError, setProtoDiagError] = useState<string | null>(null);
  const [includePayload, setIncludePayload] = useState(false);
  const [diagFormat, setDiagFormat] = useState<'markdown' | 'json'>('markdown');
  const [logSessionId, setLogSessionId] = useState<string | null>(null);
  const capScrollRef = useRef<HTMLDivElement>(null);
  // capability-groups 是滚动容器；只观察容器本身即可，
  // MutationObserver 会捕获子节点（含两列内分组）变化触发重新测量。
  const { canScrollUp: capCanScrollUp, canScrollDown: capCanScrollDown } = useScrollFadeState(capScrollRef);
  const pluginId = device.pluginId;

  // Sync protocol diagnostic state with backend on mount: the session may
  // have been started in a previous DeviceDetails open or may have expired.
  useEffect(() => {
    let cancelled = false;
    invoke<{ protocolDiagnostic: { deviceKey: string } | null; sessionId: string }>('log_status')
      .then((status) => {
        if (cancelled) return;
        setLogSessionId(status.sessionId);
        const proto = status.protocolDiagnostic;
        setProtoDiagActive(proto !== null && proto.deviceKey === deviceKey);
      })
      .catch(() => {
        // log_status unavailable in web preview; leave defaults.
      });
    return () => { cancelled = true; };
  }, [deviceKey]);

  // Build group → placement order mapping by inspecting actual capability source paths.
  const detailOrder = new Map<string, number>();
  for (const capability of device.pluginCapabilities) {
    const placement = placementsFor(capability, 'details')[0];
    if (!placement) continue;
    const sourceGroups = capabilitySourceGroups(capability);
    for (const group of sourceGroups) {
      detailOrder.set(group, placement.order);
    }
    // Also map the capability id itself as a fallback.
    if (sourceGroups.length === 0) {
      detailOrder.set(capability.id, placement.order);
    }
  }

  const groups = Object.entries(device.capabilities)
    .filter(([, fields]) => fields && Object.keys(fields).length > 0)
    .sort(([a], [b]) => (detailOrder.get(a) ?? 10_000) - (detailOrder.get(b) ?? 10_000) || a.localeCompare(b));

  // 双列平衡分配：用字段数估算每个分组高度，贪心放到当前较短的一列，
  // 让块在两列间散落分布，避免一列很长、一列很空。估算高度由分组
  // padding(20) + h3(~17) + 每行字段(~18) 构成，足以驱动贪心决策。
  const { leftGroups, rightGroups } = useMemo(() => {
    let leftHeight = 0;
    let rightHeight = 0;
    const leftGroups: typeof groups = [];
    const rightGroups: typeof groups = [];
    for (const entry of groups) {
      const [, fields] = entry;
      const fieldCount = Object.keys(fields).length;
      const height = 20 + 17 + fieldCount * 18;
      if (leftHeight <= rightHeight) {
        leftGroups.push(entry);
        leftHeight += height;
      } else {
        rightGroups.push(entry);
        rightHeight += height;
      }
    }
    return { leftGroups, rightGroups };
  }, [groups]);

  const handleCopyAll = useCallback(() => {
    // Clipboard API 缺失时不得让状态永久停在 copying。
    if (!navigator.clipboard) {
      setCopyState('idle');
      return;
    }
    setCopyState('copying');
    const payload = {
      miraVersion: 'dev',
      pluginId: device.pluginId,
      evidence: device.evidence,
      connection: device.connection,
      updatedAt: device.updatedAt,
      capabilities: device.capabilities,
      readStatuses: device.readStatuses ?? {},
    };
    const text = JSON.stringify(payload, null, 2);
    navigator.clipboard.writeText(text).then(() => {
      setCopyState('copied');
      setTimeout(() => setCopyState('idle'), 2000);
    }).catch(() => setCopyState('idle'));
  }, [device]);

  const handleRefresh = useCallback(async () => {
    setRefreshingDetails(true);
    try {
      await invoke('device_refresh');
    } catch {
      // 保持原行为：刷新失败不覆盖已有读数，也不新增重复通知。
    } finally {
      setRefreshingDetails(false);
    }
  }, []);

  const handleCopyDiagnostics = useCallback(() => {
    setDiagCopyState('copying');
    setProtoDiagError(null);
    const input = {
      pluginId: pluginId ?? '',
      deviceKey,
      model: undefined,
      sessionId: logSessionId ?? undefined,
      readingsJson: JSON.stringify(device.capabilities ?? {}, null, 2),
      readStatusesJson: JSON.stringify(device.readStatuses ?? {}, null, 2),
      includeProtocolPayload: includePayload && protoDiagActive,
      format: diagFormat,
    };
    // path 为空 → 仅返回 content，不写文件。
    invoke<{ content: string }>('log_export_device_diagnostics', { input, path: '' })
      .then((outcome) => {
        if (!navigator.clipboard) throw new Error('clipboard unavailable');
        return navigator.clipboard.writeText(outcome.content);
      })
      .then(() => {
        setDiagCopyState('copied');
        setTimeout(() => setDiagCopyState('idle'), 2000);
      })
      .catch(() => {
        setDiagCopyState('idle');
        setProtoDiagError(i18n.t('dashboard.diagnosticsCopyFailed'));
      });
  }, [device, deviceKey, pluginId, includePayload, protoDiagActive, diagFormat, logSessionId]);

  const handleToggleProtocolDiagnostic = useCallback(() => {
    setProtoDiagError(null);
    if (protoDiagActive) {
      invoke('log_stop_protocol_diagnostic')
        .then(() => invoke('log_stop_diagnostic_session'))
        .then(() => setProtoDiagActive(false))
        .catch(() => setProtoDiagError(i18n.t('dashboard.protocolDiagnosticStopFailed')));
    } else {
      // 默认 10 分钟，自动到期。同时提升日志等级到 Trace 以采集 HID 交换事件。
      // 链式调用：如果 diagnostic_session 启动失败，回滚已启动的 protocol_diagnostic。
      invoke('log_start_protocol_diagnostic', { deviceKey, minutes: 10, autoExpire: true })
        .then(() =>
          invoke('log_start_diagnostic_session', { minutes: 10, level: 'trace', autoExpire: true })
            .then(() => setProtoDiagActive(true))
            .catch((sessionErr) => {
              // 回滚：diagnostic_session 失败时停止已启动的 protocol_diagnostic
              invoke('log_stop_protocol_diagnostic').catch(() => {});
              setProtoDiagError(i18n.t('dashboard.protocolDiagnosticStartFailed'));
              throw sessionErr;
            })
        )
        .catch(() => setProtoDiagError(i18n.t('dashboard.protocolDiagnosticStartFailed')));
    }
  }, [deviceKey, protoDiagActive]);

  return (
    <Modal
      open
      title={i18n.t('dashboard.allReadInfo')}
      size="medium"
      className="device-details"
      backdropClassName="details-backdrop"
      onClose={onClose}
    >
      <header>
        <div><p className="eyebrow">{i18n.t('dashboard.readonlyReport')}</p><h2 id="device-details-title">{i18n.t('dashboard.allReadInfo')}</h2></div>
        <button className="icon-button" onClick={onClose} aria-label={i18n.t('dashboard.closeDeviceDetails')}><X weight="regular" /></button>
      </header>
      <p className="details-note">{i18n.t('dashboard.detailsNote')}</p>
      <div className="details-actions">
        <MiraActivityButton
          className="details-action-btn"
          active={refreshingDetails}
          activity="refreshing-device-details"
          announce
          leading={<Timer weight="regular" />}
          onClick={() => void handleRefresh()}
          title={i18n.t('dashboard.refreshAll')}
        >
          {i18n.t('dashboard.refreshAll')}
        </MiraActivityButton>
        <MiraActivityButton
          className="details-action-btn"
          active={copyState === 'copying'}
          activity="copying-readings"
          announce
          leading={copyState === 'copied' ? <span aria-hidden="true">✓</span> : <ReadCvLogo weight="regular" />}
          onClick={handleCopyAll}
          title={i18n.t('dashboard.copyAllReadings')}
        >
          {i18n.t('dashboard.copyAllReadings')}
        </MiraActivityButton>
        <MiraActivityButton
          className="details-action-btn"
          active={diagCopyState === 'copying'}
          activity="copying-device-diagnostics"
          announce
          leading={diagCopyState === 'copied' ? <span aria-hidden="true">✓</span> : <Info weight="regular" />}
          onClick={handleCopyDiagnostics}
          title={i18n.t('dashboard.copyDeviceDiagnostics')}
        >
          {i18n.t('dashboard.copyDeviceDiagnostics')}
        </MiraActivityButton>
      </div>
      <div className="protocol-diagnostic-toggle">
        <label className="protocol-diagnostic-label">
          <input
            type="checkbox"
            checked={protoDiagActive}
            onChange={handleToggleProtocolDiagnostic}
          />
          <span>{i18n.t('dashboard.protocolDiagnostic')}</span>
        </label>
        <p className="protocol-diagnostic-hint">{i18n.t('dashboard.protocolDiagnosticHint')}</p>
        {protoDiagError && <p className="protocol-diagnostic-error">{protoDiagError}</p>}
        {protoDiagActive && (
          <label className="protocol-diagnostic-payload-label">
            <input
              type="checkbox"
              checked={includePayload}
              onChange={(e) => setIncludePayload(e.target.checked)}
            />
            <span>{i18n.t('dashboard.includeProtocolPayload')}</span>
          </label>
        )}
      </div>
      <div className="diagnostics-format-selector">
        <span className="diagnostics-format-label">{i18n.t('dashboard.diagnosticsFormat')}</span>
        <div className="diagnostics-format-options">
          <button
            type="button"
            className={diagFormat === 'markdown' ? 'active' : ''}
            onClick={() => setDiagFormat('markdown')}
          >
            {i18n.t('dashboard.diagnosticsFormatMarkdown')}
          </button>
          <button
            type="button"
            className={diagFormat === 'json' ? 'active' : ''}
            onClick={() => setDiagFormat('json')}
          >
            {i18n.t('dashboard.diagnosticsFormatJson')}
          </button>
        </div>
      </div>
      <div ref={capScrollRef} className={`capability-groups${capCanScrollUp ? ' scroll-fade-top' : ''}${capCanScrollDown ? ' scroll-fade-bottom' : ''}`}>
        {groups.length ? (
          <>
            <div className="capability-column">
              {leftGroups.map(([group, fields]) => (
                <section className="capability-group" key={group}>
                  <h3>
                    {capabilityGroupLabel(group, pluginId)}
                    <ReadStatusBadge status={device.readStatuses?.[group]} />
                  </h3>
                  <dl>
                    {Object.entries(fields).map(([key, value]) => {
                      const valueLabel = resolveDetailValueLabel(group, key, device);
                      const complex = isComplexValue(value);
                      return (
                        <div key={key}>
                          <dt>{capabilityFieldLabel(key, pluginId)}</dt>
                          <dd>
                            {complex
                              ? <DetailValue value={value} />
                              : <FormattedValue value={value} label={valueLabel} />}
                          </dd>
                        </div>
                      );
                    })}
                  </dl>
                </section>
              ))}
            </div>
            <div className="capability-column">
              {rightGroups.map(([group, fields]) => (
                <section className="capability-group" key={group}>
                  <h3>
                    {capabilityGroupLabel(group, pluginId)}
                    <ReadStatusBadge status={device.readStatuses?.[group]} />
                  </h3>
                  <dl>
                    {Object.entries(fields).map(([key, value]) => {
                      const valueLabel = resolveDetailValueLabel(group, key, device);
                      const complex = isComplexValue(value);
                      return (
                        <div key={key}>
                          <dt>{capabilityFieldLabel(key, pluginId)}</dt>
                          <dd>
                            {complex
                              ? <DetailValue value={value} />
                              : <FormattedValue value={value} label={valueLabel} />}
                          </dd>
                        </div>
                      );
                    })}
                  </dl>
                </section>
              ))}
            </div>
          </>
        ) : <p className="setting-hint">{i18n.t('dashboard.noCapabilities')}</p>}
      </div>
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
      const stages = layout.dotsSource
        ? ((readPath(device, layout.dotsSource) as DpiStage[] | undefined) ?? []).filter((stage) => stage.enabled)
        : [];
      const activeDpi = usesSoftwareDpiLayout(layout, device)
        ? softwareDpiCurrentValue(layout, device)
        : stages.find((stage) => stage.active)?.value ?? stages[0]?.value;
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
      .filter((field) => fieldHasReportedValue(field, device));
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
    if ((capability.metadata.summary ?? []).some((item) => {
      const { value } = resolveSummaryValue(item, device);
      return value !== undefined && value !== null && value !== '';
    })) {
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
  geometryScope?: string,
  geometryContent?: string,
) {
  const previousTargetRef = useRef<string | undefined>(undefined);
  const previousGeometryScopeRef = useRef<string | undefined>(undefined);
  const previousGeometryContentRef = useRef<string | undefined>(undefined);

  useLayoutEffect(() => {
    const previousTarget = previousTargetRef.current;
    const previousGeometryScope = previousGeometryScopeRef.current;
    const previousGeometryContent = previousGeometryContentRef.current;
    previousTargetRef.current = targetSelector;
    previousGeometryScopeRef.current = geometryScope;
    previousGeometryContentRef.current = geometryContent;
    if (!targetSelector) return;

    const stage = stageRef.current;
    const target = stage?.querySelector<HTMLElement>(targetSelector);
    const layer = layerRef.current;
    if (!stage || !target || !layer) return;

    const shouldHideWhileSnapping = layer.dataset.positioned !== 'true'
      || (transitionMode === 'snap' && previousTarget === undefined);
    const shouldSnapContentChange = layer.dataset.positioned === 'true'
      && previousTarget === targetSelector
      && previousGeometryScope === geometryScope
      && previousGeometryContent !== undefined
      && geometryContent !== undefined
      && previousGeometryContent !== geometryContent;
    const shouldDisableGeometryMotion = shouldHideWhileSnapping || shouldSnapContentChange;
    let revealFrame = 0;

    const measure = () => {
      const stageRect = stage.getBoundingClientRect();
      const targetRect = target.getBoundingClientRect();
      layer.style.width = `${targetRect.width}px`;
      layer.style.height = `${targetRect.height}px`;
      layer.style.transform = `translate3d(${targetRect.left - stageRect.left}px, ${targetRect.top - stageRect.top}px, 0)`;
      layer.dataset.positioned = 'true';
    };

    if (shouldDisableGeometryMotion) {
      layer.dataset.geometryReady = 'false';
      layer.dataset.geometrySnap = 'true';
      if (shouldHideWhileSnapping) layer.dataset.repositioning = 'true';
      layer.getBoundingClientRect();
    } else {
      layer.dataset.geometryReady = 'true';
    }
    measure();
    if (shouldDisableGeometryMotion) {
      layer.getBoundingClientRect();
      revealFrame = window.requestAnimationFrame(() => {
        layer.dataset.geometryReady = 'true';
        layer.dataset.geometrySnap = 'false';
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
      layer.dataset.geometrySnap = 'false';
      layer.dataset.repositioning = 'false';
    };
  }, [geometryContent, geometryScope, layerRef, stageRef, targetSelector, transitionMode]);
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
        contextTransitionDelay={sync === 'surface' ? 47 : 50}
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
  const [controlStageTransition, setControlStageTransition] = useState<ControlStageTransition>();
  const [previewMessage, setPreviewMessage] = useState('');
  const [showDetails, setShowDetails] = useState(false);
  const [showAdvancedSettings, setShowAdvancedSettings] = useState(false);
  const [showBatteries, setShowBatteries] = useState(false);
  const [showBatteryLearningInfo, setShowBatteryLearningInfo] = useState(false);
  const [chargingEstimate, setChargingEstimate] = useState<BatteryChargingEstimate>();
  const [showDeviceSwitcher, setShowDeviceSwitcher] = useState(false);
  const batteryControlRef = useRef<HTMLDivElement>(null);
  const batteryPopoverRef = useRef<HTMLElement>(null);
  const deviceSwitcherRef = useRef<HTMLDivElement>(null);
  const [batteryPopoverPosition, setBatteryPopoverPosition] = useState<{ left: number; top: number; width: number } | null>(null);
  const controlStageLayersRef = useRef<HTMLDivElement>(null);
  const sharedContextLayerRef = useRef<HTMLDivElement>(null);
  const sharedMetricLayerRef = useRef<HTMLDivElement>(null);
  const sharedSurfaceLayerRef = useRef<HTMLDivElement>(null);
  const [writeBusy, setWriteBusy] = useState(false);
  const [editingField, setEditingField] = useState<{ capability: PluginCapability; field: PluginField } | null>(null);
  const [statusSwitchRestoreValues, setStatusSwitchRestoreValues] = useState<Record<string, unknown>>({});
  const forcedPreviewMessage = demoMode
    && new URLSearchParams(window.location.search).get('preview') === 'writing'
    ? t('dashboard.writing')
    : '';
  const visiblePreviewMessage = previewMessage || forcedPreviewMessage;

  const chargingEstimatePolicy = useMemo<PluginChargingEstimatePolicy | undefined>(() => {
    const policy = device.pluginCapabilities
      .find((capability) => capability.id === 'battery' && capability.available !== false)
      ?.metadata.batteryHistory?.chargingEstimate;
    if (!policy || policy.mode !== 'local-learning' || !device.family) return undefined;
    return policy.families.includes(device.family) ? policy : undefined;
  }, [device.family, device.pluginCapabilities]);
  const chargingEstimateBattery = chargingEstimatePolicy
    ? device.batteries.find((battery) => (
      battery.id === chargingEstimatePolicy.componentId
      && battery.charging
      && battery.percentage > 0
      && battery.percentage < 100
    ))
    : undefined;

  useEffect(() => {
    if (!chargingEstimatePolicy || !chargingEstimateBattery || !device.historyIdentity?.group) {
      return undefined;
    }
    let cancelled = false;
    const rawPercentage = chargingEstimateBattery.percentage;
    invoke<BatteryChargingEstimate>('battery_charging_estimate_get', {
      identityGroup: device.historyIdentity.group,
      componentId: chargingEstimatePolicy.componentId,
      groundTruthComponentId: chargingEstimatePolicy.groundTruthComponentId,
      rawPercentage,
    }).then((estimate) => {
      if (!cancelled) setChargingEstimate(estimate);
    }).catch(() => {
      if (!cancelled) {
        setChargingEstimate({
          state: 'disabled',
          lowerPercentage: Math.max(0, rawPercentage - 25),
          upperPercentage: Math.min(100, rawPercentage + 25),
          calibrationCount: 0,
        });
      }
    });
    return () => { cancelled = true; };
  }, [
    chargingEstimateBattery,
    chargingEstimatePolicy,
    device.historyIdentity?.group,
  ]);

  const positionBatteryPopover = useCallback(() => {
    const anchor = batteryControlRef.current;
    if (!anchor) return;
    const rect = anchor.getBoundingClientRect();
    const viewportPadding = 16;
    const width = Math.min(238, Math.max(0, window.innerWidth - viewportPadding * 2));
    const maxLeft = Math.max(viewportPadding, window.innerWidth - viewportPadding - width);
    setBatteryPopoverPosition({
      left: Math.min(Math.max(viewportPadding, rect.left), maxLeft),
      top: rect.bottom + 6,
      width,
    });
  }, []);

  useLayoutEffect(() => {
    if (!showBatteries) return;
    positionBatteryPopover();
    const reposition = () => positionBatteryPopover();
    window.addEventListener('resize', reposition);
    window.addEventListener('scroll', reposition, true);
    return () => {
      window.removeEventListener('resize', reposition);
      window.removeEventListener('scroll', reposition, true);
    };
  }, [positionBatteryPopover, showBatteries]);

  useEffect(() => {
    const closeOnOutsideClick = (event: MouseEvent) => {
      const target = event.target as Node;
      if (!batteryControlRef.current?.contains(target) && !batteryPopoverRef.current?.contains(target)) {
        setShowBatteries(false);
      }
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

  useEffect(() => subscribeTransientSurfaceDismiss(() => {
    setShowBatteries(false);
    setShowDeviceSwitcher(false);
  }), []);

  const runMutation: RunMutation = async (
    mutation: string,
    params: Record<string, unknown>,
    attentionId?: number,
  ) => {
    const beforeDevice = device;
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
        confirmLightingMutation(attentionId, beforeDevice, nextDevice);
        return;
      }
      const snapshot = await invoke<DeviceSnapshot>('device_mutate', { mutation, params });
      const nextDevice = snapshotToState(snapshot);
      onDeviceChange(nextDevice);
      setPreviewMessage('');
      notifySuccess(i18n.t('dashboard.writeConfirmed'));
      confirmLightingMutation(attentionId, beforeDevice, nextDevice);
    } catch (error) {
      if (attentionId !== undefined) clearPendingLightingAttention(attentionId);
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

  const { groups: controlGroups, usedDedupeKeys: controlDedupeKeys, fallback: controlFallback } = useMemo(() => {
    // 使用纯函数选择器，输入 ReadonlySet<string>，返回新 Set。
    // 共享 dedupeKey 上下文，防止 control 与 status 区域出现重复入口。
    const { selected: controlCandidates, fallback, usedDedupeKeys } = selectDashboardControls(
      device.pluginCapabilities,
      device,
      capabilityAvailable,
      (capability) => capabilityHasControlContent(capability, device),
      new Set<string>(),
    );
    // 直接使用选择器返回的最终序列，无需按 fixedSlot/order 重新排序。
    // 选择器已保证 [leading → 核心(DPI→polling→lighting) → trailing] 顺序。
    // finalIndex 来自选择器序列的数组索引，用于保持 DOM 顺序与选择器语义一致。
    const groups = new Map<string, {
      id: string;
      label: string;
      accessibleLabel: string;
      icon: string | undefined;
      capabilities: PluginCapability[];
      finalIndex: number;
    }>();
    controlCandidates.forEach((candidate, finalIndex) => {
      const id = candidate.groupId;
      const existing = groups.get(id);
      if (existing) {
        existing.capabilities.push(candidate.capability);
      } else {
        const accessibleLabel = resolveLabelKey(candidate.capability.labelKey, device.pluginId);
        groups.set(id, {
          id,
          label: candidate.placement.compactLabelKey
            ? resolveLabelKey(candidate.placement.compactLabelKey, device.pluginId)
            : accessibleLabel,
          accessibleLabel,
          icon: candidate.placement.icon,
          capabilities: [candidate.capability],
          finalIndex,
        });
      }
    });
    // Map 保留插入顺序，无需 .sort()。选择器已限制最多 4 项，无需再次 slice。
    const result = [...groups.values()]
      .map((group) => ({
        ...group,
        hasMetric: Boolean(sharedControlMetric(group.capabilities, device)),
        hasSurface: Boolean(sharedControlSurface(group.capabilities, device)),
      }));
    return { groups: result, usedDedupeKeys, fallback };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [device, pluginLocaleRevision]);

  const controls = controlGroups;

  const activeMode = controls.some((c) => c.id === mode) ? mode : controls[0]?.id ?? '';
  const activeGroup = controls.find((c) => c.id === activeMode);
  const activeCapabilities = activeGroup?.capabilities ?? [];
  const metricPresentation = sharedControlMetric(activeCapabilities, device);
  const surfacePresentation = sharedControlSurface(activeCapabilities, device);
  const activeHasMetric = activeGroup?.hasMetric ?? false;
  const activeHasSurface = activeGroup?.hasSurface ?? false;
  const activePageKind = controlPageKind(activeCapabilities);
  const pollingContextTarget = metricPresentation?.variant === 'hertz'
    ? '.metric-reading > .metric-reading-heading'
    : undefined;
  const metricGeometryContent = metricPresentation
    ? `${metricPresentation.variant}\u0000${metricPresentation.text}\u0000${metricPresentation.unit}`
    : undefined;
  useControlTargetPosition(controlStageLayersRef, pollingContextTarget, sharedContextLayerRef, 'snap');
  useControlTargetPosition(
    controlStageLayersRef,
    metricPresentation?.targetSelector,
    sharedMetricLayerRef,
    'morph',
    activeMode,
    metricGeometryContent,
  );
  useControlTargetPosition(controlStageLayersRef, surfacePresentation?.targetSelector, sharedSurfaceLayerRef, 'snap');

  useEffect(() => {
    if (!controlStageTransition) return;
    const transitionId = controlStageTransition.id;
    const fallback = window.setTimeout(() => {
      setControlStageTransition((current) => current?.id === transitionId ? undefined : current);
    }, 380);
    return () => window.clearTimeout(fallback);
  }, [controlStageTransition]);

  const switchMode = (targetMode: string, sync: 'metric' | 'surface') => {
    if (!targetMode || targetMode === activeMode) return;
    const targetGroup = controls.find((control) => control.id === targetMode);
    const targetCapabilities = targetGroup?.capabilities ?? [];
    const targetPageKind = controlPageKind(targetCapabilities);
    const coordinatesDpiAndSegmented = (
      (activePageKind === 'dpi' && targetPageKind === 'segmented')
      || (activePageKind === 'segmented' && targetPageKind === 'dpi')
    );
    const entersSegmentedPage = targetPageKind === 'segmented';
    if (coordinatesDpiAndSegmented || entersSegmentedPage) {
      setControlStageTransition((current) => ({
        id: (current?.id ?? 0) + 1,
        fromMode: activeMode,
        fromKind: activePageKind,
        fromCapabilities: [...activeCapabilities],
        preserveOutgoing: coordinatesDpiAndSegmented,
        toMode: targetMode,
        toKind: targetPageKind,
      }));
    } else {
      setControlStageTransition(undefined);
    }
    setContextMotionSync(sync);
    setMode(targetMode);
    setPreviewMessage('');
  };

  const { items: statusItems, fallback: statusFallback } = useMemo(() => {
    // 纯函数选择器，消费 controlDedupeKeys 作为依赖。
    // 共享 dedupeKey 上下文，避免与上方控制区出现重复入口（如全部读数、电量）。
    const hasReportedStatus = (capability: PluginCapability): boolean => {
      const base = resolveStatusDisplay(capability);
      if (!base) return false;
      if (capabilityRuntimePending(capability)) return true;
      // 先解析 active variant，再检查 valueSource/onClickField。
      // 当 statusDisplay.variants 存在时，valueSource/onClickField 在 variant 上而非 base 上。
      const active = resolveStatusDisplayVariant(base, device);
      const requestedField = resolveStatusField(capability, active.onClickField, device);
      const displayedValue = active.valueSource ? readPath(device, active.valueSource) : undefined;
      const fallbackValue = requestedField ? readPath(device, requestedField.source) : undefined;
      if (
        (displayedValue === undefined || displayedValue === null || displayedValue === '')
        && (fallbackValue === undefined || fallbackValue === null || fallbackValue === '')
      ) return false;
      return true;
    };
    const { selected: statusCandidates, fallback: statusFallback } = selectDashboardStatus(
      device.pluginCapabilities,
      device,
      capabilityAvailable,
      hasReportedStatus,
      controlDedupeKeys,
    );

    const controlAction = (capability: PluginCapability): (() => void) | undefined => {
      const controlPlacement = placementsFor(capability, 'control')[0];
      if (!controlPlacement) return undefined;
      const target = controlPlacement.group || capability.id;
      const targetControl = controls.find((control) => control.id === target);
      if (!targetControl) return undefined;
      const sync = resolveContextMotionSync(
        activeHasMetric,
        activeHasSurface,
        targetControl.hasMetric,
        targetControl.hasSurface,
      );
      return () => switchMode(target, sync);
    };

    const items: { capability: PluginCapability; placement: PluginCapabilityPlacement; onClick: (() => void) | undefined }[] = [];
    for (const candidate of statusCandidates) {
      const { capability, placement } = candidate;
      const base = resolveStatusDisplay(capability);
      // 已通过 hasReportedStatus 校验，display 必然存在；保留防御性检查。
      if (!base) continue;
      // 使用 active variant 的 onClickField，而非 base。
      const display = resolveStatusDisplayVariant(base, device);
      let onClick: (() => void) | undefined;
      if (display.onClickField) {
        const field = resolveStatusField(capability, display.onClickField, device);
        if (field) {
          const interaction = resolveFieldInteraction(field);
          const mutation = resolveMutation(field.mutation, device.writableMutations);
          if (interaction === 'control') {
            onClick = controlAction(capability);
          } else if (mutation && !writeBusy) {
            if (interaction === 'modal') {
              onClick = () => {
                invoke('device_refresh_quick').catch(() => {});
                setEditingField({ capability, field });
              };
            } else if (interaction === 'action') {
              onClick = () => void runMutation(mutation, resolveFieldParams(field, device));
            } else {
              const restoreKey = `${device.name}:${capability.id}:${field.id}`;
              onClick = () => {
                const currentValue = readPath(device, field.switch?.source ?? field.source);
                const rememberedValue = field.switch
                  && currentValue !== field.switch.offValue
                  && currentValue != null
                  ? currentValue
                  : statusSwitchRestoreValues[restoreKey];
                if (rememberedValue === currentValue && currentValue != null) {
                  setStatusSwitchRestoreValues((current) => ({
                    ...current,
                    [restoreKey]: currentValue,
                  }));
                }
                const nextValue = resolveSwitchNextValue(
                  field,
                  device,
                  rememberedValue,
                );
                if (nextValue !== undefined) {
                  void runMutation(mutation, resolveFieldMutationParams(field, device, nextValue));
                }
              };
            }
          }
        }
      } else {
        onClick = controlAction(capability);
      }
      items.push({ capability, placement, onClick });
    }
    return { items, fallback: statusFallback };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [device, pluginLocaleRevision, controlDedupeKeys]);

  // Advanced Settings 分组。消费真实 selector fallback（非声明式）：
  // 1. selectDashboardControls(...).fallback（控制区未入选的候选）
  // 2. selectDashboardStatus(...).fallback（状态区未入选的候选）
  // 3. polling subblock fallback（回报率超过 3 项的溢出）
  // 4. lighting subblock fallback（灯光超过 6 项的溢出 + presentation=details 字段）
  // 5. placement.region=details 的 capability 字段
  // 6. 已在首页的 capability 中的 presentation=details 字段
  // inventory fallbackRegion 去 "全部读数"，不进 Advanced；hidden 不展示。
  // system.all-readings 不重复。
  const advancedFieldGroups = useMemo(() => {
    const sectionOrder: Array<NonNullable<PluginField['advancedSection']>> = [
      'performance', 'lighting-details', 'profiles', 'buttons', 'power', 'sensor', 'device',
    ];
    const groups = new Map<string, AdvancedSettingsEntry[]>();
    for (const section of sectionOrder) groups.set(section, []);

    const addEntry = (section: string | undefined, entry: AdvancedSettingsEntry) => {
      const target = section ?? 'device';
      if (!groups.has(target)) groups.set(target, []);
      groups.get(target)!.push(entry);
    };

    // 字段去重键：capabilityId:zoneId-or-root:fieldId（必须包含 zoneId）。
    const collectedFieldKeys = new Set<string>();
    const collectedSummaryKeys = new Set<string>();
    // 已处理的 capability（避免 fallback 与 details region 重复收集）。
    const seenCapabilityIds = new Set<string>();

    // 首页 capability ID 集合（用于判断是否只收集 presentation=details）。
    const homepageCapabilityIds = new Set<string>();
    for (const group of controlGroups) {
      for (const capability of group.capabilities) {
        homepageCapabilityIds.add(capability.id);
      }
    }

    // 按 zone 遍历，保留 zoneId/zoneLabelKey 上下文。
    // 禁止 zones.flatMap(zone => zone.fields) 后丢失 zone。
    // 顶层字段（capability.metadata.fields）使用 'root' 作为 zoneId。
    const collectAllFields = (capability: PluginCapability) => {
      // DpiStages with stageLayout：作为整体 entry 展示。
      if (
        capability.control === 'DpiStages'
        && capability.metadata.stageLayout
        && !capability.readOnly
      ) {
        addEntry('performance', { type: 'stageLayout', capability });
      }
      // 顶层字段（无 zone）
      for (const field of capability.metadata.fields ?? []) {
        if (!resolveVisibleWhen(field.visibleWhen, device)) continue;
        const mutation = resolveMutation(field.mutation, device.writableMutations);
        const isDetails = field.presentation === 'details';
        if (!mutation && !isDetails) continue;
        const key = `${capability.id}:root:${field.id}`;
        if (collectedFieldKeys.has(key)) continue;
        collectedFieldKeys.add(key);
        addEntry(field.advancedSection, { type: 'field', capability, field, zoneId: undefined, zoneLabelKey: undefined });
      }
      // zone 内字段——保留 zoneId 和 zoneLabelKey
      for (const zone of capability.metadata.zones ?? []) {
        for (const field of zone.fields) {
          if (!resolveVisibleWhen(field.visibleWhen, device)) continue;
          const mutation = resolveMutation(field.mutation, device.writableMutations);
          const isDetails = field.presentation === 'details';
          if (!mutation && !isDetails) continue;
          const key = `${capability.id}:${zone.id}:${field.id}`;
          if (collectedFieldKeys.has(key)) continue;
          collectedFieldKeys.add(key);
          addEntry(field.advancedSection, { type: 'field', capability, field, zoneId: zone.id, zoneLabelKey: zone.labelKey });
        }
      }
    };

    // 收集 homepage capability 中的 presentation=details 字段（字段级分层）。
    const collectDetailsFields = (capability: PluginCapability) => {
      for (const field of capability.metadata.fields ?? []) {
        if (field.presentation !== 'details') continue;
        if (!resolveVisibleWhen(field.visibleWhen, device)) continue;
        const key = `${capability.id}:root:${field.id}`;
        if (collectedFieldKeys.has(key)) continue;
        collectedFieldKeys.add(key);
        addEntry(field.advancedSection, { type: 'field', capability, field, zoneId: undefined, zoneLabelKey: undefined });
      }
      for (const zone of capability.metadata.zones ?? []) {
        for (const field of zone.fields) {
          if (field.presentation !== 'details') continue;
          if (!resolveVisibleWhen(field.visibleWhen, device)) continue;
          const key = `${capability.id}:${zone.id}:${field.id}`;
          if (collectedFieldKeys.has(key)) continue;
          collectedFieldKeys.add(key);
          addEntry(field.advancedSection, { type: 'field', capability, field, zoneId: zone.id, zoneLabelKey: zone.labelKey });
        }
      }
    };

    // 1. 消费 controlFallback（真实 selector fallback）。
    //    inventory fallbackRegion 去 "全部读数"，不进 Advanced；hidden 不展示。
    for (const candidate of controlFallback) {
      const { capability, placement } = candidate;
      if (seenCapabilityIds.has(capability.id)) continue;
      const region = placement.fallbackRegion ?? 'advanced';
      if (region !== 'advanced') continue; // inventory/hidden 不进 Advanced
      seenCapabilityIds.add(capability.id);
      collectAllFields(capability);
    }

    // 2. 消费 statusFallback（真实 selector fallback）。
    for (const candidate of statusFallback) {
      const { capability, placement } = candidate;
      if (seenCapabilityIds.has(capability.id)) continue;
      const region = placement.fallbackRegion ?? 'advanced';
      if (region !== 'advanced') continue;
      seenCapabilityIds.add(capability.id);
      collectAllFields(capability);
    }

    // 3. 消费 placement.region=details 的 capability（未被 fallback 覆盖的）。
    for (const capability of device.pluginCapabilities) {
      if (!capabilityAvailable(capability)) continue;
      if (seenCapabilityIds.has(capability.id)) continue;
      if (homepageCapabilityIds.has(capability.id)) continue;
      const hasDetailsPlacement = (capability.placements ?? []).some((p) => p.region === 'details');
      if (!hasDetailsPlacement) continue;
      seenCapabilityIds.add(capability.id);
      collectAllFields(capability);
    }

    // 4. polling overflow。对每个有 summary 的 capability，
    //    计算 selectSummarySubblocks fallback，溢出项进入 Advanced Settings。
    //    summary 上限只作用于 polling capability。
    for (const capability of device.pluginCapabilities) {
      if (!capabilityAvailable(capability)) continue;
      const summary = capability.metadata.summary;
      if (!summary || summary.length === 0) continue;
      const reportedItems = summary.filter((item) => {
        const { value } = resolveSummaryValue(item, device);
        return value !== undefined && value !== null && value !== '';
      });
      const max = summaryMaxForCapability(capability);
      const { fallback: summaryFallback } = selectSummarySubblocks(reportedItems, max);
      for (const item of summaryFallback) {
        const key = `${capability.id}:${item.source}`;
        if (collectedSummaryKeys.has(key)) continue;
        collectedSummaryKeys.add(key);
        addEntry('performance', { type: 'summary', capability, item });
      }
    }

    // 5. lighting overflow。对每个 LightingZone capability，
    //    计算 selectLightingSubblocks fallback + presentation=details 字段。
    //    去重键包含 zoneId，保留 zone 上下文。
    for (const capability of device.pluginCapabilities) {
      if (!capabilityAvailable(capability)) continue;
      if (capability.control !== 'LightingZone') continue;
      const zones = resolveZones(capability, device);
      for (const zone of zones) {
        const lightingCandidates = zone.fields.filter((field) =>
          fieldHasReportedValue(field, device) && field.presentation !== 'details',
        );
        const { fallback: lightingFallback } = selectLightingSubblocks(lightingCandidates);
        for (const field of lightingFallback) {
          const key = `${capability.id}:${zone.id}:${field.id}`;
          if (collectedFieldKeys.has(key)) continue;
          collectedFieldKeys.add(key);
          addEntry(field.advancedSection ?? 'lighting-details', { type: 'field', capability, field, zoneId: zone.id, zoneLabelKey: zone.labelKey });
        }
        // 同时收集 presentation=details 字段（如 AM35 的 color2/ratio2/color3/ratio3）。
        for (const field of zone.fields) {
          if (field.presentation !== 'details') continue;
          if (!resolveVisibleWhen(field.visibleWhen, device)) continue;
          const key = `${capability.id}:${zone.id}:${field.id}`;
          if (collectedFieldKeys.has(key)) continue;
          collectedFieldKeys.add(key);
          addEntry(field.advancedSection ?? 'lighting-details', { type: 'field', capability, field, zoneId: zone.id, zoneLabelKey: zone.labelKey });
        }
      }
    }

    // 6. 已在首页的 capability 中的 presentation=details 字段（字段级分层）。
    for (const group of controlGroups) {
      for (const capability of group.capabilities) {
        if (seenCapabilityIds.has(capability.id)) continue;
        collectDetailsFields(capability);
      }
    }

    const result: { section: NonNullable<PluginField['advancedSection']>; entries: AdvancedSettingsEntry[] }[] = [];
    for (const section of sectionOrder) {
      const entries = groups.get(section) ?? [];
      if (entries.length === 0) continue;
      entries.sort((a, b) => {
        const orderA = a.type === 'field' ? (a.field.advancedOrder ?? 100) : 50;
        const orderB = b.type === 'field' ? (b.field.advancedOrder ?? 100) : 50;
        return orderA - orderB;
      });
      result.push({ section, entries });
    }
    return result;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [device, pluginLocaleRevision, controlGroups, controlFallback, statusFallback]);

  const hasAdvancedSettings = advancedFieldGroups.length > 0;

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
            {device.batteries.length > 0 && (() => {
              // 优先显示鼠标电量，而非数组顺序的第一个：鼠标无线休眠时
              // batteries 可能只剩 receiver（后端 merge_batteries 已尽量保留
              // mouse 条目，这里做前端双保险），避免摘要按钮误显示接收器电量。
              const primaryBattery = device.batteries.find((b) => b.id === 'mouse') ?? device.batteries[0];
              return (
            <div ref={batteryControlRef} className={`battery-control ${showBatteries ? 'open' : ''}`}>
              <button
                className="battery-state"
                aria-expanded={showBatteries}
                aria-controls="device-batteries"
                onClick={() => {
                  if (!showBatteries) {
                    invoke('device_refresh_battery').catch(() => {});
                    positionBatteryPopover();
                  }
                  setShowBatteries((visible) => !visible);
                }}
              >
                <BatteryLevelIcon percentage={primaryBattery.percentage} charging={primaryBattery.charging} />
                {primaryBattery.percentage}%
                {primaryBattery.charging ? ` · ${t('common.charging')}` : ''}
                <span className="battery-count">{t('dashboard.deviceCount', { count: device.batteries.length })}</span>
              </button>
            </div>
              );
            })()}
          </div>
        </div>
        <DeviceAura color={declaredAccentColor(device)} />
      </section>
      {showBatteries && (
        <OverlayPortal>
          <section
            ref={batteryPopoverRef}
            id="device-batteries"
            className="battery-popover"
            aria-label={t('dashboard.deviceBattery')}
            data-positioned={batteryPopoverPosition ? 'true' : 'false'}
            style={batteryPopoverPosition ?? undefined}
          >
                <div className="battery-popover-header">
                  <span>{t('dashboard.deviceBattery')}</span>
                  <strong>{t('dashboard.deviceCount', { count: device.batteries.length })}</strong>
                </div>
                <div className="battery-device-list">
                  {device.batteries.map((battery) => {
                    const batteryLevel = Math.max(0, Math.min(100, battery.percentage));
                    const batteryTone = battery.charging ? 'charging' : batteryLevel <= 20 ? 'low' : 'normal';
                    const usesChargingEstimate = battery.id === chargingEstimatePolicy?.componentId
                      && battery.charging
                      && battery.percentage > 0
                      && battery.percentage < 100;
                    const estimateState = usesChargingEstimate ? chargingEstimate?.state ?? 'disabled' : undefined;
                    const estimateLower = usesChargingEstimate
                      ? chargingEstimate?.lowerPercentage ?? Math.max(0, batteryLevel - 25)
                      : batteryLevel;
                    const estimateUpper = usesChargingEstimate
                      ? chargingEstimate?.upperPercentage ?? Math.min(100, batteryLevel + 25)
                      : batteryLevel;
                    return (

                      <div
                        key={battery.id}
                        className={`battery-device ${batteryTone}${usesChargingEstimate ? ' estimated-charging' : ''}`}
                      >
                        <div className="battery-device-main">
                          <span className="battery-device-label">
                            <BatteryLevelIcon percentage={battery.percentage} charging={battery.charging} />
                            <span>{t(battery.label, { defaultValue: battery.label })}</span>
                          </span>
                          <span className="battery-device-value">
                            {usesChargingEstimate ? (
                              estimateState === 'learning' ? (
                                <button
                                  type="button"
                                  className="battery-learning-trigger"
                                  onClick={() => {
                                    setShowBatteries(false);
                                    setShowBatteryLearningInfo(true);
                                  }}
                                >
                                  {t('dashboard.batteryLearning.status')}
                                  <Info weight="bold" />
                                </button>
                              ) : (
                                <small>{t('common.charging')}</small>
                              )
                            ) : (
                              <>
                                <strong>{battery.percentage}%</strong>
                                {battery.charging && <small>{t('common.charging')}</small>}
                              </>
                            )}
                          </span>
                        </div>
                        <span className={`battery-meter${usesChargingEstimate ? ' estimated' : ''}`} aria-hidden="true">
                          {usesChargingEstimate ? (
                            <>
                              <span className="battery-meter-fill confirmed" style={{ width: `${estimateLower}%` }} />
                              <span
                                className="battery-meter-fill uncertain"
                                style={{ left: `${estimateLower}%`, width: `${Math.max(0, estimateUpper - estimateLower)}%` }}
                              />
                            </>
                          ) : (
                            <span className="battery-meter-fill" style={{ width: `${batteryLevel}%` }} />
                          )}
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
        </OverlayPortal>
      )}
      <Modal
        open={showBatteryLearningInfo}
        title={t('dashboard.batteryLearning.title')}
        size="small"
        className="battery-learning-modal"
        backdropClassName="edit-modal-backdrop"
        onClose={() => setShowBatteryLearningInfo(false)}
      >
        <section>
          <header><h3>{t('dashboard.batteryLearning.title')}</h3></header>
          <p>{t('dashboard.batteryLearning.description')}</p>
          <footer>
            <button type="button" onClick={() => setShowBatteryLearningInfo(false)}>
              {t('dashboard.batteryLearning.confirm')}
            </button>
          </footer>
        </section>
      </Modal>

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
        {controls.map(({ id, label, accessibleLabel, icon, hasMetric, hasSurface }) => {
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
              aria-label={accessibleLabel}
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
          visiblePreviewMessage ? 'has-preview-message' : '',
          pollingContextTarget ? 'has-shared-context' : '',
          metricPresentation ? 'has-shared-metric' : '',
          surfacePresentation ? 'has-shared-surface' : '',
        ].filter(Boolean).join(' ')}
        aria-live="polite"
        data-control-mode={activeMode}
        data-control-transition={controlStageTransition
          ? `${controlStageTransition.fromKind}-to-${controlStageTransition.toKind}`
          : undefined}
      >
        <div ref={controlStageLayersRef} className="control-stage-layers">
          <SharedControlSurfaceLayer layerRef={sharedSurfaceLayerRef} surface={surfacePresentation} />
          <div className="control-stage-content">
            {controlStageTransition?.toMode === activeMode && controlStageTransition.preserveOutgoing && (
              <div
                key={`leaving-${controlStageTransition.id}`}
                className="control-stage-page is-leaving"
                data-control-page={controlStageTransition.fromMode}
                data-page-kind={controlStageTransition.fromKind}
                aria-hidden="true"
                inert
              >
                {controlStageTransition.fromCapabilities.map((capability) => (
                  <CapabilityRouter
                    key={capability.id}
                    capability={capability}
                    device={device}
                    writeBusy={writeBusy}
                    runMutation={runMutation}
                  />
                ))}
              </div>
            )}
            <div
              key={activeMode}
              className={`control-stage-page${controlStageTransition?.toMode === activeMode ? ' is-entering' : ''}`}
              data-control-page={activeMode}
              data-page-kind={activePageKind}
              onAnimationEnd={(event) => {
                if (event.target !== event.currentTarget) return;
                const transitionId = controlStageTransition?.id;
                if (transitionId === undefined) return;
                setControlStageTransition((current) => current?.id === transitionId ? undefined : current);
              }}
            >
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
        {visiblePreviewMessage && (
          <p className="preview-message mira-process-message">
            <MiraInlineActivity
              active={writeBusy || Boolean(forcedPreviewMessage)}
              activity="applying-settings"
              delayMs={0}
              reserveSpace={false}
            />
            <span>{visiblePreviewMessage}</span>
          </p>
        )}
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
        {hasAdvancedSettings && (
          <button className="details-button" onClick={() => setShowAdvancedSettings(true)}><SlidersHorizontal weight="regular" />{t('advancedSettings.open')}</button>
        )}
      </div>
      {showDetails && <DeviceDetails device={device} deviceKey={selectedEntry?.deviceKey ?? ''} onClose={() => setShowDetails(false)} />}
      {showAdvancedSettings && (
        <AdvancedSettingsModal
          groups={advancedFieldGroups}
          device={device}
          writeBusy={writeBusy}
          onClose={() => setShowAdvancedSettings(false)}
          onEditField={(capability, field) => setEditingField({ capability, field })}
          runMutation={runMutation}
        />
      )}
    </main>
  );
}

/** 把「更新语义」的应用内通知映射为通知浮层上的 Attention 请求（§5.4~5.6）。
 *  目标归属由 resolveUpdateAttentionTarget 裁决：固定更新区域当前可见时返回
 *  undefined —— 由固定更新行播放（§11 仲裁），保证同一事件只有一个目标消费。 */
function resolveNotificationBeam(notification: AppNotification, currentView: View, currentSettingsTab: SettingsTab): AttentionBeamRequest | undefined {
  // 日志页没有固定更新区域；在通知归属上按普通非设置视图处理，避免沿用
  // 离开设置前的 plugins 标签误抑制插件 / 本地 AI 更新通知。
  const attentionView = currentView === 'logs' ? 'dashboard' : currentView;
  const target = (kind: 'app' | 'plugin' | 'local-ai') =>
    resolveUpdateAttentionTarget(kind, { view: attentionView, settingsTab: currentSettingsTab });

  switch (notification.action) {
    case 'about-update': {
      if (target('app') !== 'notification') return undefined;
      const state = appUpdateState();
      if (state.phase !== 'available' || !state.version) return undefined;
      return {
        eventKey: attentionAppUpdateKey(state.version),
        scope: 'notification:app',
        variant: 'line',
        color: attentionDesaturatedAccent(),
        durationMs: 1650,
        strength: 0.2,
        cycles: 1,
        delayMs: 200,
        priority: ATTENTION_PRIORITY['update-available'],
      };
    }
    case 'relaunch': {
      if (target('app') !== 'notification') return undefined;
      const state = appUpdateState();
      if (state.phase !== 'installed' || !state.version) return undefined;
      return {
        eventKey: attentionAppRestartKey(state.version),
        scope: 'notification:app',
        variant: 'pulse-inner',
        color: attentionDesaturatedAccent(),
        durationMs: 2400,
        strength: 0.16,
        cycles: 2,
        priority: ATTENTION_PRIORITY['restart-required'],
      };
    }
    case 'settings-plugin-update': {
      if (target('plugin') !== 'notification') return undefined;
      const info = pluginUpdateState().updates.find((item) => item.updateAvailable);
      if (!info?.availableVersion) return undefined;
      return {
        eventKey: attentionPluginUpdateKey(info.pluginId, info.availableVersion),
        scope: 'notification:app',
        variant: 'line',
        color: attentionDesaturatedAccent(),
        durationMs: 1600,
        strength: 0.18,
        cycles: 1,
        delayMs: 200,
        priority: ATTENTION_PRIORITY['update-available'],
      };
    }
    case 'settings-local-ai-update': {
      if (target('local-ai') !== 'notification') return undefined;
      const info = localAiUpdateState().updates.find((item) => item.updateAvailable);
      if (!info?.availableVersion) return undefined;
      return {
        eventKey: attentionLocalAiUpdateKey(info.component, info.availableVersion),
        scope: 'notification:app',
        variant: 'line',
        color: attentionDesaturatedAccent(),
        durationMs: 1600,
        strength: 0.18,
        cycles: 1,
        delayMs: 200,
        priority: ATTENTION_PRIORITY['update-available'],
      };
    }
    default:
      return undefined;
  }
}

function DeferredPageFallback({ view, onBack, settingsTab }: { view: View; onBack: () => void; settingsTab: SettingsTab }) {
  if (view === 'about') return <AboutPageSkeleton onBack={onBack} />;
  if (view === 'settings') return <SettingsPageSkeleton tab={settingsTab} />;
  return <LogPageSkeleton onBack={onBack} />;
}

function DeferredBatteryUsageFallback({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  return (
    <Modal
      open
      title={t('batteryUsage.title')}
      size="large"
      className="battery-usage-modal"
      backdropClassName="battery-usage-modal-overlay"
      onClose={onClose}
    >
      <div className="battery-usage-modal-layout">
        <div className="battery-usage-header">
          <div className="battery-usage-title-wrap"><h2>{t('batteryUsage.title')}</h2></div>
          <button className="battery-usage-close-icon" onClick={onClose} aria-label={t('batteryUsage.close')}>
            <X weight="regular" />
          </button>
        </div>
        <div className="battery-usage-scroll-region deferred-battery-loading" aria-busy="true">
          <div className="runtime-battery-frame" aria-hidden="true">
            <section className="battery-status-strip runtime-battery-status">
              <span className="runtime-skeleton runtime-battery-icon" />
              <div className="runtime-skeleton-lines">
                <span className="runtime-skeleton runtime-skeleton-short" />
                <span className="runtime-skeleton" />
              </div>
            </section>
            <section className="battery-summary runtime-battery-summary">
              {Array.from({ length: 3 }, (_, index) => (
                <div className="battery-summary-item" key={index}>
                  <span className="runtime-skeleton runtime-skeleton-short" />
                  <span className="runtime-skeleton" />
                </div>
              ))}
            </section>
            <section className="battery-chart-card runtime-battery-chart">
              <span className="runtime-skeleton runtime-skeleton-short" />
              <div className="runtime-battery-plot">
                {Array.from({ length: 8 }, (_, index) => <span key={index} />)}
              </div>
            </section>
            <section className="battery-insight-section runtime-battery-insights">
              <span className="runtime-skeleton runtime-skeleton-short" />
              <div className="battery-insight-cards">
                <div className="battery-insight-card"><div className="runtime-skeleton-lines"><span className="runtime-skeleton" /><span className="runtime-skeleton runtime-skeleton-short" /></div></div>
                <div className="battery-insight-card"><div className="runtime-skeleton-lines"><span className="runtime-skeleton" /><span className="runtime-skeleton runtime-skeleton-short" /></div></div>
              </div>
            </section>
          </div>
          <span className="visually-hidden" role="status">{t('about.loading')}</span>
        </div>
      </div>
    </Modal>
  );
}

export default function App() {
  const { t } = useTranslation();
  const pureWeb = isPureWebPreview();
  // 纯网页视觉夹具：复现真实 Presence → Full 之间的短暂初始化快照。
  // Tauri 运行时不读取该分支；打开/刷新 ?preview=initializing 即可观察
  // 玻璃卡的完整入场、稳定和退场，不需要伪造设备协议事件。
  const startupPreviewMode = pureWeb
    ? new URLSearchParams(window.location.search).get('preview')
    : null;
  const startupPreview = startupPreviewMode === 'initializing'
    || startupPreviewMode === 'initializing-fast';
  const previewDevice = useMemo(() => startupPreview
    ? {
        ...MOCK_DEVICE,
        pluginCapabilities: MOCK_DEVICE.pluginCapabilities.map((capability, index) => (
          index === 0
            ? {
                ...capability,
                metadata: { ...capability.metadata, _miraRuntimePending: true },
              }
            : capability
        )),
      }
    : MOCK_DEVICE, [startupPreview]);
  const [device, setDevice] = useState<DeviceState | undefined>(
    pureWeb && !startupPreview ? previewDevice : undefined,
  );
  useEffect(() => {
    if (!startupPreview) return;
    // 先让空状态真实显示，再模拟 Presence 快照抵达。慢夹具会超过
    // 识别卡门槛；快夹具在门槛前完成，用于证明卡片不会闪现。
    const detectedTimer = window.setTimeout(() => setDevice(previewDevice), 280);
    const readyTimer = window.setTimeout(
      () => setDevice(MOCK_DEVICE),
      startupPreviewMode === 'initializing-fast' ? 560 : 1280,
    );
    return () => {
      window.clearTimeout(detectedTimer);
      window.clearTimeout(readyTimer);
    };
  }, [previewDevice, startupPreview, startupPreviewMode]);
  const [deviceEntries, setDeviceEntries] = useState<DeviceSnapshotEntry[]>(pureWeb ? MOCK_DEVICE_ENTRIES : []);
  const deviceEntriesRef = useRef<DeviceSnapshotEntry[]>(pureWeb ? MOCK_DEVICE_ENTRIES : []);
  // device-updated 可能由初始化或用户按需读取连续触发；用 startTransition
  // 将 Dashboard 渲染标记为低优先级。writeBusy 等用户交互状态保持同步。
  const [, startTransition] = useTransition();
  const [theme, setTheme] = useState<ThemeMode>('system');
  const [themeLoaded, setThemeLoaded] = useState(pureWeb);
  const [view, setViewState] = useState<View>('dashboard');
  // 页面切换过渡：current 层始终渲染目标页 view；leaving 层仅在切换期间渲染
  // 旧页并整层淡出，与目标页淡入交叉，全程无透明空白，避免「闪现」。
  // leaving 层使用旧页的「静态 DOM 快照」而非活组件渲染，避免退场期间 SettingsPage
  // 等内部自带 IPC/effect 的页持续重渲染、重置容器淡出动画，造成刷新与闪烁。
  const [leavingView, setLeavingView] = useState<View | null>(null);
  const [leavingHTML, setLeavingHTML] = useState<string | null>(null);
  const [pageTransitionKind, setPageTransitionKind] = useState<'navigation' | 'device-arrival' | null>(null);
  const prevViewRef = useRef<View>('dashboard');
  const prevHasDeviceRef = useRef(device !== undefined);
  const transitionTimer = useRef<number | undefined>(undefined);
  // 当前层 DOM 节点引用：用于抓取旧页快照。
  const currentPageRef = useRef<HTMLDivElement | null>(null);
  // 最近一次「非过渡期」提交时抓取的当前页 HTML 快照；导航触发时再覆盖为
  // 含实时 Aura 合成帧的快照，保证退场层不把光团重置到关键帧起点。
  const pageSnapshotRef = useRef<string | null>(null);
  // 过渡进行中标记：置位期间暂停快照抓取，避免把新页误抓成旧页。
  const transitioningRef = useRef(false);
  const navigateTo = useCallback((nextView: View) => {
    if (nextView === view) return;
    const node = currentPageRef.current;
    if (node) pageSnapshotRef.current = pageSnapshotHtml(node);
    setViewState(nextView);
  }, [view]);
  // 设置页当前「实际可见」的标签（由 SettingsPage 在 130ms 切换过渡结束后上报，
  // 不是用户点击的目标标签）。通知与固定更新行的可见性仲裁都以此为准。
  const [visibleSettingsTab, setVisibleSettingsTab] = useState<SettingsTab>('general');
  const [aboutFocusToken, setAboutFocusToken] = useState(0);
  const [settingsPluginFocusToken, setSettingsPluginFocusToken] = useState(0);
  const [settingsLocalAiFocusToken, setSettingsLocalAiFocusToken] = useState(0);
  const [demoMode, setDemoMode] = useState(pureWeb);
  const [refreshNonce, setRefreshNonce] = useState(0);
  const [appNotification, setAppNotification] = useState<AppNotification>();
  // 与当前具体通知绑定的 Beam eventKey：只允许绑定「announce 真正入队/播放」
  // 的事件键，普通成功/错误通知不会有 Beam。通知替换时旧更新 Beam 不会
  // 套到新通知卡片上（P1-3）。
  const [appNotificationAttentionEventKey, setAppNotificationAttentionEventKey] = useState<string>();
  const [showBatteryUsage, setShowBatteryUsage] = useState(false);
  const [batteryUsageSession, setBatteryUsageSession] = useState(0);
  const [batteryUsageSettings, setBatteryUsageSettings] = useState<{
    batteryHistoryEnabled: boolean;
    aiAnalysisEnabled: boolean;
    lowBatteryThreshold: number;
  } | undefined>(
    pureWeb ? { batteryHistoryEnabled: true, aiAnalysisEnabled: false, lowBatteryThreshold: 20 } : undefined,
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
    navigateTo('dashboard');
    setRefreshNonce((value) => value + 1);
    invoke('device_refresh').catch(() => {});
  }, [navigateTo]);
  const openAboutUpdate = useCallback(() => {
    navigateTo('about');
    setAboutFocusToken((value) => value + 1);
  }, [navigateTo]);
  const openSettingsPluginUpdate = useCallback(() => {
    navigateTo('settings');
    setSettingsPluginFocusToken((value) => value + 1);
  }, [navigateTo]);
  const openSettingsLocalAiUpdate = useCallback(() => {
    navigateTo('settings');
    setSettingsLocalAiFocusToken((value) => value + 1);
  }, [navigateTo]);
  const openBatteryUsage = useCallback(() => {
    setBatteryUsageSession((value) => value + 1);
    setShowBatteryUsage(true);
  }, []);
  const syncBatteryUsageSettings = useCallback((settings: {
    batteryHistoryEnabled: boolean;
    aiAnalysisEnabled: boolean;
    lowBatteryThreshold: number;
  }) => {
    setBatteryUsageSettings(settings);
  }, []);
  const reloadPluginLocales = useCallback(() => {
    void loadPluginLocales().then((loaded) => {
      if (loaded) setPluginLocaleRevision((value) => value + 1);
    });
  }, []);

  const handleTauriEvent = useEffectEvent((eventName: string) => {
    switch (eventName) {
      case 'navigate-about-update':
        openAboutUpdate();
        break;
      case 'navigate-about':
        dismissTransientSurfaces();
        setShowBatteryUsage(false);
        navigateTo('about');
        break;
      case 'navigate-dashboard':
        dismissTransientSurfaces();
        setShowBatteryUsage(false);
        navigateTo('dashboard');
        break;
      case 'navigate-plugin-update':
        openSettingsPluginUpdate();
        break;
      case 'navigate-local-ai-update':
        openSettingsLocalAiUpdate();
        break;
      case 'open-battery-usage':
        dismissTransientSurfaces();
        openBatteryUsage();
        break;
      case 'plugin-locales-updated':
        invalidateAboutInfo();
        reloadPluginLocales();
        break;
      case 'window-resumed':
        setRefreshNonce((value) => value + 1);
        remindInstalledUpdateOnShown();
        break;
      default:
        break;
    }
  });

  // 设置页卸载时清除插件/AI 引擎更新聚焦 token，避免下次进入设置时重复跳转和重复显示"已更新至"标签。
  // token 仅在点击更新完成通知时递增，离开设置页即视为已消费。
  const handleSettingsExit = useCallback(() => {
    setSettingsPluginFocusToken(0);
    setSettingsLocalAiFocusToken(0);
  }, []);

  // ── Attention Beam：通知浮层表面 ──────────────────────────────────────
  // 只保留统一入口 onAppNotification(handleAppNotification)：每条通知只处理
  // 一次，Beam 判断、announce 结果与状态更新在同一个入口（P2-1 / P1-3）。
  const notificationAttention = useAttentionFeedback('notification:app');
  const notificationAnnounce = notificationAttention.announce;
  const handleAppNotification = useEffectEvent((nextNotification: AppNotification) => {
    const beam = resolveNotificationBeam(nextNotification, view, visibleSettingsTab);
    const accepted = beam ? notificationAnnounce(beam) : false;
    setAppNotification(nextNotification);
    setAppNotificationAttentionEventKey(accepted ? beam?.eventKey : undefined);
  });
  useEffect(() => onAppNotification(handleAppNotification), []);

  // 所有通知消失路径（自动超时 / 关闭按钮 / Relaunch 点击）统一清理，并同步
  // 解除 Beam eventKey 绑定，避免残留键把旧 Beam 渲染到后续通知上。
  const clearAppNotification = useCallback(() => {
    setAppNotification(undefined);
    setAppNotificationAttentionEventKey(undefined);
  }, []);

  // 渲染前再做一次 eventKey 匹配：总线里仍活跃的旧 Beam 不会显示在
  // 与它无关的新通知卡片上（P1-3）。
  const visibleNotificationBeam =
    notificationAttention.beam?.eventKey === appNotificationAttentionEventKey
      ? notificationAttention.beam
      : null;

  useEffect(() => {
    if (pureWeb) return;
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const eventNames = [
      'navigate-about-update',
      'navigate-about',
      'navigate-dashboard',
      'navigate-plugin-update',
      'navigate-local-ai-update',
      'open-battery-usage',
      'plugin-locales-updated',
      'window-resumed',
    ];
    for (const eventName of eventNames) {
      void listen(eventName, () => handleTauriEvent(eventName))
        .then((unlisten) => {
          if (disposed) unlisten();
          else unlisteners.push(unlisten);
        })
        .catch(() => {});
    }
    // macOS native 通知只提醒。
    // 不监听窗口聚焦事件，避免误把用户主动打开当作通知点击。Windows/Linux 由 navigate-* / open-battery-usage
    // 事件直接处理；macOS 系统通知仅显示 title/body，应用内 Toast 保留可点击入口。
    return () => {
      disposed = true;
      for (const unlisten of unlisteners) unlisten();
    };
  }, [pureWeb]);

  // 加载插件 locale，注册为 i18n namespace（以插件 ID 命名）。
  // 异步加载完成后刷新插件标签 memo，加载前使用 host 回退标签。
  useEffect(() => {
    reloadPluginLocales();
  }, [reloadPluginLocales]);

  useEffect(() => {
    if (!appNotification) return;
    const isRelaunchReminder = appNotification.action === 'relaunch';
    const timeout = window.setTimeout(() => {
      if (isRelaunchReminder) recordUpdateReminderIgnored();
      clearAppNotification();
    }, 6000);
    return () => window.clearTimeout(timeout);
  }, [appNotification, clearAppNotification]);

  useEffect(() => {
    if (pureWeb) return;
    loadAppSettings()
      .then((settings) => {
        setTheme(settings.theme as ThemeMode);
        setThemeLoaded(true);
        syncBatteryUsageSettings({
          batteryHistoryEnabled: settings.batteryHistoryEnabled ?? true,
          aiAnalysisEnabled: localAiFeatureEnabled(settings, LOCAL_AI_FEATURE.batteryUsage),
          lowBatteryThreshold: settings.lowBatteryThreshold,
        });
        applyLanguage(settings.language ?? 'auto');
        if (settings.automaticUpdateChecks) {
          void loadAboutInfo()
            .then((info) => {
              if (info.updaterActive) return startAutomaticAppUpdateCheck(true, settings.automaticUpdateInstall);
            })
            .catch(() => { /* Pre-release and offline builds skip automatic application checks. */ });
        }
        void startAutomaticPluginUpdateCheck(settings.automaticPluginUpdateChecks);
        void startAutomaticLocalAiUpdateCheck(settings.automaticLocalAiUpdateChecks ?? true);
      })
      .catch(() => setThemeLoaded(true));
  }, [pureWeb, syncBatteryUsageSettings]);

  // 主程序更新优先级协调：当主程序有可用更新/正在下载/已安装待重启时，
  // 抑制 AI 引擎与插件更新通知，避免用户被多个来源同时打断。
  useEffect(() => {
    if (pureWeb) return;
    initUpdatePriorityCoordinator();
  }, [pureWeb]);

  // 周期性从后端读取真实设备状态
  useEffect(() => {
    if (demoMode) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    let unlistenEntries: (() => void) | undefined;

    // 启动时预读缓存，避免首次渲染空白
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
  const batteryUsageConnectedTargets = useMemo(
    () => connectedBatteryUsageTargets(deviceEntries, batteryUsageSettings?.lowBatteryThreshold ?? 20),
    [batteryUsageSettings?.lowBatteryThreshold, deviceEntries],
  );
  // 全局 Orb 只表达 Dashboard 的设备近程状态。电量整理属于已打开的电量
  // Modal，由 Modal 内部的 embedded Orb 承担，避免玻璃浮层之上再叠玻璃卡。
  const globalDeviceActivity = view === 'dashboard' && device
    ? (device.mouseReady === false
        ? 'awaiting-mouse' as const
        : deviceRuntimePending(device)
          ? 'device-initializing' as const
          : null)
    : null;

  // 只检测一次：按 WebView 能力挂载根节点类，Attention Beam CSS 据此切换完整实现与降级。
  useEffect(() => {
    const support = detectAttentionVisualSupport();
    const root = document.documentElement;

    root.classList.toggle(
      'attention-full-line-supported',
      support.fullLineBeam,
    );

    root.classList.toggle(
      'attention-color-mix-supported',
      support.colorMix,
    );

    return () => {
      root.classList.remove(
        'attention-full-line-supported',
      );
      root.classList.remove(
        'attention-color-mix-supported',
      );
    };
  }, []);
  useEffect(() => {
    if (!themeLoaded) return;
    applyTheme(theme, themeColor);
  }, [themeLoaded, theme, themeColor]);

  // 页面切换过渡：view 变化时，把上一页（prevViewRef）作为退场页渲染在 leaving 层
  // 并整层淡出，目标页在 current 层直接淡入，两层交叉，避免内容瞬时消失造成「闪现」。
  // 用 useLayoutEffect 保证退场层在浏览器绘制前就位，避免中间一帧旧页已消失、新页直显。
  useLayoutEffect(() => {
    const prev = prevViewRef.current;
    prevViewRef.current = view;
    if (view === prev) return;
    window.clearTimeout(transitionTimer.current);
    // 把导航瞬间抓取的旧页快照交给 leaving 层，并用 transitioningRef 暂停后续快照抓取，
    // 直到过渡结束，避免退场期间把新页误抓成「旧页」。
    transitioningRef.current = true;
    setLeavingHTML(pageSnapshotRef.current);
    setLeavingView(prev);
    setPageTransitionKind('navigation');
    transitionTimer.current = window.setTimeout(() => {
      setLeavingView(null);
      setLeavingHTML(null);
      setPageTransitionKind(null);
      transitioningRef.current = false;
    }, 240);
    return () => {
      window.clearTimeout(transitionTimer.current);
      transitioningRef.current = false;
    };
  }, [view]);

  const hasDevice = device !== undefined;
  // 设备从“未找到”变为可渲染快照时，复用页面快照层保留旧空状态，
  // 与新 Dashboard 做一次短交叉过渡。旧层与新层始终重叠，不制造空白帧；
  // 初始即有设备时不会触发，避免重新引入启动内容延迟。
  useLayoutEffect(() => {
    const hadDevice = prevHasDeviceRef.current;
    prevHasDeviceRef.current = hasDevice;
    if (view !== 'dashboard' || hadDevice || !hasDevice) return;
    const snapshot = pageSnapshotRef.current;
    if (!snapshot?.includes('class="empty"')) return;

    window.clearTimeout(transitionTimer.current);
    transitioningRef.current = true;
    setLeavingHTML(snapshot);
    setLeavingView('dashboard');
    setPageTransitionKind('device-arrival');
    transitionTimer.current = window.setTimeout(() => {
      setLeavingView(null);
      setLeavingHTML(null);
      setPageTransitionKind(null);
      transitioningRef.current = false;
    }, 240);
    return () => {
      window.clearTimeout(transitionTimer.current);
      transitioningRef.current = false;
    };
  }, [hasDevice, view]);

  // 非过渡期持续抓取当前层的 DOM 快照，供下一次退场使用。
  // 用 useLayoutEffect 保证在绘制前拿到与屏幕一致的结构；过渡中跳过，避免误抓新页。
  useLayoutEffect(() => {
    if (transitioningRef.current) return;
    const node = currentPageRef.current;
    if (node) pageSnapshotRef.current = node.innerHTML;
  });

  // 渲染指定视图。current 层与 leaving 层共用，保证切换时两层内容一致。
  const renderView = (v: View) => {
    if (v === 'dashboard') {
      return !device
        ? <EmptyState onRefresh={() => { setDemoMode(false); setDevice(undefined); setDeviceEntries([]); deviceEntriesRef.current = []; setRefreshNonce((value) => value + 1); invoke('device_refresh').catch(() => {}); }} onDemo={() => { setDemoMode(true); setDevice(MOCK_DEVICE); setDeviceEntries(MOCK_DEVICE_ENTRIES); deviceEntriesRef.current = MOCK_DEVICE_ENTRIES; }} onOpenSettings={() => navigateTo('settings')} />
        : device.mouseReady === false
          ? <AwaitingMouseState deviceName={device.name} onRefresh={() => { setRefreshNonce((value) => value + 1); invoke('device_refresh').catch(() => {}); }} onOpenSettings={() => navigateTo('settings')} />
          : <Dashboard device={device} deviceEntries={deviceEntries} onDeviceChange={setDevice} onDeviceSelect={selectDevice} onOpenBatteryUsage={openBatteryUsage} pluginLocaleRevision={pluginLocaleRevision} demoMode={demoMode} />;
    }
    if (v === 'settings') {
      return <SettingsPage initialTab={visibleSettingsTab} onTabChange={setVisibleSettingsTab} previewMode={pureWeb} focusPluginUpdateToken={settingsPluginFocusToken} focusLocalAiUpdateToken={settingsLocalAiFocusToken} onSettingsExit={handleSettingsExit} onNavigateAbout={() => navigateTo('about')} onNavigateLogs={() => navigateTo('logs')} onOpenBatteryUsage={openBatteryUsage} onBatteryUsageSettingsChange={syncBatteryUsageSettings} onThemeChange={setTheme} pluginCapabilities={device?.pluginCapabilities ?? []} writableMutations={device?.writableMutations ?? []} />;
    }
    if (v === 'about') {
      return <AboutPage previewMode={pureWeb} focusUpdateToken={aboutFocusToken} onBack={() => navigateTo('settings')} />;
    }
    return <LogPage onBack={() => navigateTo('settings')} />;
  };
  const titledView = view === 'dashboard' ? undefined : view;
  const pageTitle = view === 'settings'
    ? t('settings.title')
    : view === 'about'
      ? t('about.title')
      : view === 'logs'
        ? t('logs.title')
        : undefined;

  return <div className={`app-shell ${pureWeb ? 'web-preview' : ''} ${windowsPlatform ? 'platform-windows' : ''} ${macPlatform ? 'platform-macos' : ''} ${fallbackPlatform ? 'platform-fallback' : ''} ${windowsWebPreview ? 'windows-web-preview' : ''}`}>
    <AttentionBusController />
    <MiraActivityOverlay activity={globalDeviceActivity} />
    {windowsWebPreview && <WindowsPreviewControls />}
    {windowsPlatform && !windowsWebPreview && !pureWeb && <WindowsWindowControls />}
    {windowsPlatform && !windowsWebPreview && !pureWeb && <div className="windows-drag-strip" data-tauri-drag-region />}
    <nav className="top-nav" data-tauri-drag-region />
    <div className="nav-links">
      <button className={`nav-link ${view === 'dashboard' ? 'active' : ''}`} onClick={() => navigateTo('dashboard')}>{t('nav.dashboard')}</button>
      <button className={`nav-link ${view === 'settings' || view === 'logs' ? 'active' : ''}`} onClick={() => navigateTo('settings')}>{t('nav.settings')}</button>
      <button className={`nav-link nav-about ${view === 'about' ? 'active' : ''}`} onClick={() => navigateTo('about')} aria-label={t('nav.about')}><Info weight="regular" /></button>
      {demoMode && !windowsPlatform && <button className="nav-link nav-exit" onClick={exitDemo} aria-label={t('nav.exitDemo')} title={t('nav.exitDemo')}><SignOut weight="regular" /></button>}
    </div>
    {windowsPlatform && demoMode && view === 'dashboard' && <button className="content-exit" onClick={exitDemo} aria-label={t('nav.exitDemo')} title={t('nav.exitDemo')}><SignOut weight="regular" /></button>}
    <div
      className={`page-swap${leavingView ? ' is-transitioning' : ''}`}
      data-page-view={view}
      data-page-transition={pageTransitionKind ?? undefined}
    >
      {/*
        设置 / 关于 / 日志共用固定眉题与标题槽，不参与 current / leaving
        页面层的整层交叉淡化；标题自身原地交叉，并由轮廓过渡到实心。
        始终渲染并给稳定 key，避免条件挂载导致 .page-layer-current 兄弟重排、
        被卸载重建而重启入场动画（首进设置页闪烁）。
      */}
      <div key="persistent-copy" className={`page-persistent-copy${titledView && pageTitle ? '' : ' is-hidden'}`}>
        {titledView && pageTitle && (
          <>
            <p className="eyebrow page-persistent-eyebrow">{t('about.eyebrow')}</p>
            <PersistentPageTitle view={titledView} title={pageTitle} />
          </>
        )}
      </div>
      {leavingView && (
        // leaving 层渲染旧页的静态快照（非活组件），退场期间冻结内容，
        // 避免 SettingsPage 等内部 IPC/effect 持续重渲染导致动画被重置。
        <div key="leaving" className="page-layer page-layer-leaving" aria-hidden="true" dangerouslySetInnerHTML={{ __html: leavingHTML ?? '' }} />
      )}
      <div key="current" ref={currentPageRef} className="page-layer page-layer-current">
        <Suspense fallback={<DeferredPageFallback view={view} settingsTab={visibleSettingsTab} onBack={() => navigateTo('settings')} />}>
          {renderView(view)}
        </Suspense>
      </div>
    </div>
    {showBatteryUsage && (
      <Suspense fallback={<DeferredBatteryUsageFallback onClose={() => setShowBatteryUsage(false)} />}>
        <BatteryUsageModal
          key={batteryUsageSession}
          open
          onClose={() => setShowBatteryUsage(false)}
          hasBattery={(device?.batteries.length ?? 0) > 0}
          batteryHistoryEnabled={batteryUsageSettings?.batteryHistoryEnabled}
          aiAnalysisEnabled={batteryUsageSettings?.aiAnalysisEnabled}
          connectedTargets={batteryUsageConnectedTargets}
          preferredDeviceName={selectedBatteryUsageTarget?.name}
          preferredComponentId={selectedBatteryUsageTarget?.componentId}
          demoMode={demoMode}
        />
      </Suspense>
    )}
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
                  : appNotification.action === 'settings-local-ai-update'
                    ? openSettingsLocalAiUpdate
                    : appNotification.action === 'battery-usage'
                      ? openBatteryUsage
                      : appNotification.action === 'relaunch'
                        ? () => {
                          clearAppNotification();
                          void relaunchAfterUpdate().catch((err) => {
                            notifyError(t('notification.relaunchFailed'), String(err));
                          });
                        }
                        : undefined
              : undefined
          }
        >
          {visibleNotificationBeam && (
            <AttentionBeamLayer
              active
              request={visibleNotificationBeam}
            />
          )}
          <div><strong>{appNotification.title}</strong>{appNotification.body && <p>{appNotification.body}</p>}</div>
          <button type="button" onClick={(event) => { event.stopPropagation(); if (appNotification.action === 'relaunch') recordUpdateReminderDismissed(); clearAppNotification(); }} aria-label={t('dashboard.closeNotification')}><X weight="bold" /></button>
        </aside>
      </OverlayPortal>
    )}
  </div>;
}
