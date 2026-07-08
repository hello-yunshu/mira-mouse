// SPDX-License-Identifier: AGPL-3.0-or-later
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import { Warning, X, Trash, Download, Clock, ChartBar, CaretUpDown, Plug, Gauge, ArrowsLeftRight, Lightbulb, TrendDown, BatteryCharging, BatteryLow } from '@phosphor-icons/react';
import type {
  BatteryHistoryRange,
  BatteryHistoryResponse,
  BatteryHistoryDevice,
  BatteryHistoryPoint,
  BatteryInsight,
} from './types';
import { MOCK_BATTERY_HISTORY_24H, MOCK_BATTERY_HISTORY_10D } from './mock';
import { notifyError, notifySuccess } from './notify';
import { BatteryLevelIcon } from './BatteryLevelIcon';

// ─── 工具函数 ───────────────────────────────────────────────────────────────

function isPureWebPreview(): boolean {
  return !('__TAURI_INTERNALS__' in window);
}

function formatRelativeTime(iso: string, t: (key: string) => string): string {
  const now = Date.now();
  const then = new Date(iso).getTime();
  const diff = now - then;
  if (diff < 60_000) return t('batteryUsage.lastUpdated');
  const minutes = Math.floor(diff / 60_000);
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h`;
  const days = Math.floor(hours / 24);
  return `${days} d`;
}

function formatInsightMessage(insight: BatteryInsight, t: (key: string, options?: Record<string, unknown>) => string): string {
  const message = insight.message;
  if (!message) return '';
  if (message === 'notEnoughData' || message === 'veryLowDrain') {
    return t(`batteryUsage.${message}`);
  }

  const [kind, ...parts] = message.split('|');
  switch (kind) {
    case 'remainingMinutes':
      return t('batteryUsage.remainingMinutes', { minutes: parts[0] ?? '?' });
    case 'remainingHours':
      return t('batteryUsage.remainingHours', { hours: parts[0] ?? '?' });
    case 'remainingDaysHours':
      return t('batteryUsage.remainingDaysHours', { days: parts[0] ?? '?', hours: parts[1] ?? '?' });
    case 'chargingHabitStartEnd':
      return t('batteryUsage.chargingHabitStartEnd', {
        start: parts[0] ?? '?',
        end: parts[1] ?? '?',
        count: parts[2] ?? '?',
      });
    case 'chargingHabitStartOnly':
      return t('batteryUsage.chargingHabitStartOnly', {
        start: parts[0] ?? '?',
        count: parts[1] ?? '?',
      });
    case 'abnormalDrain2h':
      return t('batteryUsage.abnormalDrain2h', { drop: parts[0] ?? '?' });
    case 'consistencyStable':
    case 'consistencyFaster':
    case 'consistencySlower':
      return t(`batteryUsage.${kind}`);
    case 'powerSavingTipLow':
      return t('batteryUsage.powerSavingTipLow', {
        component: t(parts[0] ?? '', { defaultValue: parts[0] ?? '' }),
      });
    case 'deviceComparisonDrain':
      return t('batteryUsage.deviceComparisonDrain', {
        fastest: t(parts[0] ?? '', { defaultValue: parts[0] ?? '' }),
        fastestRate: parts[1] ?? '?',
        slowest: t(parts[2] ?? '', { defaultValue: parts[2] ?? '' }),
        slowestRate: parts[3] ?? '?',
      });
    case 'averageDailyDrain':
      return t('batteryUsage.averageDailyDrainMsg', { percent: parts[0] ?? '?' });
    case 'chargingCount':
      return t('batteryUsage.chargingCountMsg', { count: parts[0] ?? '?' });
    case 'lowestLevel':
      return t('batteryUsage.lowestLevelMsg', { level: parts[0] ?? '?' });
    default:
      return t(`batteryUsage.${message}`, { defaultValue: message });
  }
}

// ─── 溢出文字弹窗 ───────────────────────────────────────────────────────────
// 检测单行/多行文字是否溢出容器，hover 时显示完整内容的毛玻璃弹窗。
function OverflowTip({ text, className, multiline }: { text: string; className?: string; multiline?: boolean }) {
  const ref = useRef<HTMLSpanElement>(null);
  const [show, setShow] = useState(false);

  const check = () => {
    const el = ref.current;
    if (!el) return false;
    return multiline ? el.scrollHeight > el.clientHeight : el.scrollWidth > el.clientWidth;
  };

  return (
    <span
      className={`overflow-tip-host${multiline ? ' multiline' : ''}`}
      onMouseEnter={() => { if (check()) setShow(true); }}
      onMouseLeave={() => setShow(false)}
    >
      <span ref={ref} className={className}>{text}</span>
      {show && <span className="overflow-tip" role="tooltip">{text}</span>}
    </span>
  );
}

// ─── SVG 图表 ───────────────────────────────────────────────────────────────

interface ChartProps {
  points: BatteryHistoryPoint[];
  range: BatteryHistoryRange;
}

function BatteryUsageChart({ points, range }: ChartProps) {
  const { t } = useTranslation();
  const [hoverIndex, setHoverIndex] = useState<number | null>(null);
  const [focusIndex, setFocusIndex] = useState<number | null>(null);

  const width = 520;
  const height = 136;
  const padding = { top: 8, right: 8, bottom: 20, left: 28 };
  const chartWidth = width - padding.left - padding.right;
  const chartHeight = height - padding.top - padding.bottom;
  const pointCount = Math.max(points.length, 1);
  const slotWidth = chartWidth / pointCount;
  // 24h 采用更高密度聚合（最多 96 个点），柱体保持细窄，交互命中区仍覆盖整个 slot。
  const visualBarWidth = Math.max(1.5, Math.min(range === '24h' ? slotWidth * 0.58 : slotWidth * 0.46, slotWidth - 1));

  const activeIndex = hoverIndex ?? focusIndex;
  const activePoint = activeIndex !== null ? points[activeIndex] : null;
  const tooltipStyle = activeIndex !== null
    ? ({ '--tooltip-x': `${((padding.left + activeIndex * slotWidth + slotWidth / 2) / width) * 100}%` } as CSSProperties)
    : undefined;

  const yTicks = [0, 25, 50, 75, 100];

  return (
    <div className="battery-chart-card">
      <div className="battery-chart-header">
        <span><ChartBar weight="regular" /> {t('batteryUsage.change' + (range === '24h' ? '24h' : '10d'))}</span>
      </div>
      <div className="battery-chart-stage">
        <svg
          className="battery-chart"
          viewBox={`0 0 ${width} ${height}`}
          role="img"
          aria-label={t('batteryUsage.title')}
        >
          <defs>
            <linearGradient id="battery-bar-normal" x1="0" y1="1" x2="0" y2="0">
              <stop offset="0%" stopColor="#34c759" />
              <stop offset="100%" stopColor="#8ff4a4" />
            </linearGradient>
            <linearGradient id="battery-bar-charging" x1="0" y1="1" x2="0" y2="0">
              <stop offset="0%" stopColor="#0a84ff" />
              <stop offset="100%" stopColor="#64d2ff" />
            </linearGradient>
            <linearGradient id="battery-bar-low" x1="0" y1="1" x2="0" y2="0">
              <stop offset="0%" stopColor="#ff453a" />
              <stop offset="100%" stopColor="#ffd60a" />
            </linearGradient>
          </defs>
          <rect
            className="battery-chart-plot"
            x={padding.left}
            y={padding.top}
            width={chartWidth}
            height={chartHeight}
            rx={10}
          />
          {/* Y 轴参考线 */}
          {yTicks.map((tick) => {
            const y = padding.top + chartHeight - (tick / 100) * chartHeight;
            return (
              <g key={tick}>
                <line
                  x1={padding.left}
                  y1={y}
                  x2={width - padding.right}
                  y2={y}
                  stroke="var(--muted)"
                  strokeOpacity={0.12}
                  strokeWidth={1}
                />
                <text
                  x={padding.left - 6}
                  y={y + 3}
                  textAnchor="end"
                  fontSize={9}
                  fill="var(--muted)"
                >
                  {tick}
                </text>
              </g>
            );
          })}

          {/* 电量柱 */}
          {points.map((point, i) => {
            const slotX = padding.left + i * slotWidth;
            const x = slotX + (slotWidth - visualBarWidth) / 2;
            const hasData = point.percentage !== undefined;
            const pct = point.percentage ?? 0;
            const barH = hasData ? (pct / 100) * chartHeight : 0;
            const renderedBarH = hasData ? Math.max(barH, 3) : 2;
            const y = padding.top + chartHeight - renderedBarH;
            const isCharging = point.charging ?? false;
            const isLow = point.lowBattery ?? false;
            const isEmpty = !hasData;
            const fillId = isCharging ? 'battery-bar-charging' : isLow ? 'battery-bar-low' : 'battery-bar-normal';

            let barClass = 'battery-chart-bar';
            if (isEmpty) barClass += ' battery-chart-empty';
            else if (isCharging) barClass += ' battery-chart-charging';
            else if (isLow) barClass += ' battery-chart-low';

            return (
              <g
                key={i}
                onMouseEnter={() => setHoverIndex(i)}
                onMouseLeave={() => setHoverIndex(null)}
                onPointerEnter={() => setHoverIndex(i)}
                onPointerLeave={() => setHoverIndex(null)}
                onFocus={() => setFocusIndex(i)}
                onBlur={() => setFocusIndex(null)}
                tabIndex={0}
                role="button"
                aria-label={`${point.bucketLabel}: ${hasData ? `${pct}%` : t('batteryUsage.notEnoughData')}`}
              >
                <rect
                  className="battery-chart-hit-area"
                  x={slotX}
                  y={padding.top}
                  width={slotWidth}
                  height={chartHeight}
                  fill="transparent"
                />
                <rect
                  x={x}
                  y={y}
                  width={visualBarWidth}
                  height={renderedBarH}
                  rx={visualBarWidth / 2}
                  className={barClass}
                  fill={isEmpty ? undefined : `url(#${fillId})`}
                />
                {/* X 轴标签：稀疏显示累计使用时长，断连/长空闲间隔已在后端压缩。 */}
                {(range === '24h' ? i % 12 === 0 : i % 8 === 0) && (
                  <text
                    x={slotX + slotWidth / 2}
                    y={height - 8}
                    textAnchor="middle"
                    fontSize={9}
                    fill="var(--muted)"
                  >
                    {point.bucketLabel}
                  </text>
                )}
              </g>
            );
          })}
        </svg>

        {/* Tooltip */}
        {activePoint && activePoint.percentage !== undefined && (
          <div className="battery-chart-tooltip" style={tooltipStyle} role="tooltip">
            <div className="tooltip-row"><strong>{t('batteryUsage.tooltipTime')}: </strong><span>{activePoint.bucketLabel}</span></div>
            <div className="tooltip-row"><strong>{t('batteryUsage.tooltipPercentage')}: </strong><span>{activePoint.percentage}%</span></div>
            {activePoint.minPercentage !== undefined && activePoint.maxPercentage !== undefined && (
              <div className="tooltip-row"><strong>{t('batteryUsage.tooltipMin')}/{t('batteryUsage.tooltipMax')}: </strong><span>{activePoint.minPercentage}%-{activePoint.maxPercentage}%</span></div>
            )}
            <div className="tooltip-row"><strong>{t('batteryUsage.tooltipCharging')}: </strong><span>{activePoint.charging ? t('common.on') : t('common.off')}</span></div>
            <div className="tooltip-row"><strong>{t('batteryUsage.tooltipLowBattery')}: </strong><span>{activePoint.lowBattery ? t('common.on') : t('common.off')}</span></div>
            <div className="tooltip-row"><strong>{t('batteryUsage.tooltipSamples')}: </strong><span>{activePoint.sampleCount}</span></div>
          </div>
        )}
      </div>
    </div>
  );
}

// ─── 摘要卡片 ───────────────────────────────────────────────────────────────

interface SummaryProps {
  device: BatteryHistoryDevice | undefined;
  insights: BatteryInsight[];
  range: BatteryHistoryRange;
}

function BatteryUsageSummary({ device, insights, range }: SummaryProps) {
  const { t } = useTranslation();
  if (!device) return null;

  const remaining = insights.find((i) => i.type === 'estimatedRemaining');
  const runout = insights.find((i) => i.type === 'estimatedRunout');
  const charging = device.latestCharging ?? false;
  const lowBattery = device.lowBattery ?? false;

  const statusText = charging
    ? t('batteryUsage.charging')
    : lowBattery
      ? t('batteryUsage.lowBattery')
      : t('batteryUsage.normal');

  return (
    <div className="battery-summary-grid">
      <div className="battery-summary-item primary">
        <span className="summary-label with-icon">
          <BatteryLevelIcon percentage={device.latestPercentage} charging={charging} />
          {t('batteryUsage.currentBattery')}
        </span>
        <OverflowTip className="summary-value" text={`${device.latestPercentage ?? '--'}%`} />
        {device.latestAt && <OverflowTip className="summary-sub" text={formatRelativeTime(device.latestAt, t)} />}
      </div>
      <div className="battery-summary-item">
        <OverflowTip className="summary-label" text={t('batteryUsage.change' + (range === '24h' ? '24h' : '10d'))} />
        <OverflowTip className="summary-value" text={statusText} />
        <OverflowTip className="summary-sub" text={charging ? t('batteryUsage.charging') : t('batteryUsage.notCharging')} />
      </div>
      <div className="battery-summary-item">
        <OverflowTip className="summary-label" text={t('batteryUsage.estimatedRemaining')} />
        <OverflowTip className="summary-value" text={remaining ? formatInsightMessage(remaining, t) : t('batteryUsage.notEnoughData')} />
      </div>
      <div className="battery-summary-item">
        <OverflowTip className="summary-label" text={t('batteryUsage.estimatedRunout')} />
        <OverflowTip className="summary-value" text={!charging ? (runout?.message ?? t('batteryUsage.notEnoughData')) : t('batteryUsage.charging')} />
      </div>
    </div>
  );
}

function BatteryUsageStatusStrip({
  device,
  devices,
  insights,
  onSelectDevice,
}: {
  device: BatteryHistoryDevice | undefined;
  devices: BatteryHistoryDevice[];
  insights: BatteryInsight[];
  onSelectDevice: (key: string) => void;
}) {
  const { t } = useTranslation();
  const [menuOpen, setMenuOpen] = useState(false);
  const stripRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    const handler = (e: MouseEvent) => {
      if (!stripRef.current?.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [menuOpen]);

  if (!device) return null;

  const charging = device.latestCharging ?? false;
  const lowBattery = device.lowBattery ?? false;
  const remaining = insights.find((i) => i.type === 'estimatedRemaining');
  const statusText = charging
    ? t('batteryUsage.charging')
    : lowBattery
      ? t('batteryUsage.lowBattery')
      : t('batteryUsage.normal');

  const hasMultipleDevices = devices.length > 1;

  const toggleMenu = () => {
    if (hasMultipleDevices) setMenuOpen((v) => !v);
  };

  return (
    <div
      ref={stripRef}
      className={`battery-status-strip ${charging ? 'charging' : lowBattery ? 'low' : 'normal'} ${hasMultipleDevices ? 'switchable' : ''}`}
      role={hasMultipleDevices ? 'button' : undefined}
      tabIndex={hasMultipleDevices ? 0 : undefined}
      aria-expanded={hasMultipleDevices ? menuOpen : undefined}
      aria-haspopup={hasMultipleDevices ? 'menu' : undefined}
      aria-label={hasMultipleDevices ? t('batteryUsage.switchDevice') : undefined}
      onClick={hasMultipleDevices ? toggleMenu : undefined}
      onKeyDown={hasMultipleDevices ? (e) => {
        if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggleMenu(); }
      } : undefined}
    >
      <div className="battery-status-device">
        <BatteryLevelIcon percentage={device.latestPercentage} charging={charging} />
        <div className="battery-status-device-info">
          <span>{device.deviceName}</span>
          <strong>{t(device.componentLabel, { defaultValue: device.componentLabel })}</strong>
        </div>
      </div>
      <div className="battery-status-metric">
        <strong>{device.latestPercentage ?? '--'}%</strong>
        <span>{remaining ? formatInsightMessage(remaining, t) : statusText}</span>
      </div>
      {hasMultipleDevices && (
        <CaretUpDown weight="thin" className="battery-status-switch-icon" aria-hidden="true" />
      )}

      {menuOpen && hasMultipleDevices && (
        <div className="battery-device-menu" role="menu" onClick={(e) => e.stopPropagation()}>
          {devices.map((d) => {
            const dCharging = d.latestCharging ?? false;
            const active = d.key === device.key;
            return (
              <button
                key={d.key}
                type="button"
                role="menuitemradio"
                aria-checked={active}
                className={`battery-device-menu-item ${active ? 'active' : ''}`}
                onClick={() => {
                  onSelectDevice(d.key);
                  setMenuOpen(false);
                }}
              >
                <BatteryLevelIcon percentage={d.latestPercentage} charging={dCharging} />
                <span className="battery-device-menu-copy">
                  <span className="battery-device-menu-title">{d.deviceName}</span>
                  <span className="battery-device-menu-label">{t(d.componentLabel, { defaultValue: d.componentLabel })}</span>
                </span>
                <span className="battery-device-menu-percent">{d.latestPercentage ?? '--'}%</span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

// ─── 洞察卡片 ───────────────────────────────────────────────────────────────

function BatteryInsightCards({ insights }: { insights: BatteryInsight[] }) {
  const { t } = useTranslation();

  // 过滤掉已在上方摘要 grid 中展示的"预计剩余"和"预计耗尽"，避免重复。
  // 同时过滤"设备对比"和"最低电量"——前者信息密度低，后者已由摘要区当前电量覆盖。
  const deduped = insights.filter(
    (i) => i.type !== 'estimatedRemaining'
      && i.type !== 'estimatedRunout'
      && i.type !== 'deviceComparison'
      && i.type !== 'lowestLevel',
  );

  if (deduped.length === 0) return null;

  const iconFor = (type: BatteryInsight['type']) => {
    switch (type) {
      case 'abnormalDrain': return <Warning weight="regular" />;
      case 'powerSavingTip': return <Lightbulb weight="regular" />;
      case 'chargingHabit': return <Plug weight="regular" />;
      case 'batteryConsistency': return <Gauge weight="regular" />;
      case 'deviceComparison': return <ArrowsLeftRight weight="regular" />;
      case 'averageDailyDrain': return <TrendDown weight="regular" />;
      case 'chargingCount': return <BatteryCharging weight="regular" />;
      case 'lowestLevel': return <BatteryLow weight="regular" />;
      default: return <Clock weight="regular" />;
    }
  };

  // 固定 2 列布局：奇数时截断最后一个，保证始终是 2 的倍数，避免单块占行。
  const visibleCount = deduped.length - (deduped.length % 2);
  const visible = deduped.slice(0, visibleCount);

  return (
    <div className="battery-insight-cards">
      {visible.map((insight, i) => (
        <div key={i} className={`battery-insight-card severity-${insight.severity}`}>
          <span className="insight-icon">{iconFor(insight.type)}</span>
          <div className="insight-body">
            <OverflowTip className="insight-title" text={t(`batteryUsage.${insight.title}`)} />
            <OverflowTip className="insight-text" text={formatInsightMessage(insight, t)} multiline />
          </div>
        </div>
      ))}
    </div>
  );
}

// ─── 空状态 ─────────────────────────────────────────────────────────────────

function BatteryHistoryDisabledState({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="battery-usage-empty">
      <h3>{t('batteryUsage.disabledTitle')}</h3>
      <p>{t('batteryUsage.disabledHint')}</p>
      <button className="battery-usage-close" onClick={onClose}>{t('batteryUsage.close')}</button>
    </div>
  );
}

function BatteryHistoryEmptyState({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="battery-usage-empty">
      <h3>{t('batteryUsage.emptyTitle')}</h3>
      <p>{t('batteryUsage.emptyHint')}</p>
      <button className="battery-usage-close" onClick={onClose}>{t('batteryUsage.close')}</button>
    </div>
  );
}

function BatteryHistoryUnsupportedState({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="battery-usage-empty">
      <h3>{t('batteryUsage.unsupportedTitle')}</h3>
      <p>{t('batteryUsage.unsupportedHint')}</p>
      <button className="battery-usage-close" onClick={onClose}>{t('batteryUsage.close')}</button>
    </div>
  );
}

// ─── 主弹窗 ─────────────────────────────────────────────────────────────────

export interface BatteryUsageModalProps {
  open: boolean;
  onClose: () => void;
  hasBattery: boolean;
}

export function BatteryUsageModal({ open, onClose, hasBattery }: BatteryUsageModalProps) {
  const { t } = useTranslation();
  const [range, setRange] = useState<BatteryHistoryRange>('24h');
  const [selectedDeviceKey, setSelectedDeviceKey] = useState<string>('');
  const [response, setResponse] = useState<BatteryHistoryResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [confirmingClear, setConfirmingClear] = useState(false);
  // 模态打开时拉取设置中的 batteryHistoryEnabled
  const [historyEnabled, setHistoryEnabled] = useState(true);
  const [reloadNonce, setReloadNonce] = useState(0);
  const pureWeb = isPureWebPreview();

  // 打开时拉取设置；纯 web 预览默认开启。
  useEffect(() => {
    if (!open || pureWeb) return;
    invoke<{ batteryHistoryEnabled?: boolean }>('settings_get')
      .then((s) => {
        setHistoryEnabled(s.batteryHistoryEnabled ?? true);
      })
      .catch(() => { /* 保留默认值 */ });
  }, [open, pureWeb]);

  // 数据加载：仅在异步回调中调用 setState，避免 effect 内同步 setState 引发级联渲染
  useEffect(() => {
    if (!open || !historyEnabled) return;
    let cancelled = false;
    if (pureWeb) {
      queueMicrotask(() => {
        if (cancelled) return;
        setResponse(range === '24h' ? MOCK_BATTERY_HISTORY_24H : MOCK_BATTERY_HISTORY_10D);
        setLoading(false);
      });
      return () => { cancelled = true; };
    }
    invoke<BatteryHistoryResponse>('battery_history_get', { range })
      .then((res) => {
        if (cancelled) return;
        setResponse(res);
        setLoading(false);
      })
      .catch((err) => {
        if (cancelled) return;
        notifyError(t('batteryUsage.title'), String(err));
        setLoading(false);
      });
    return () => { cancelled = true; };
  }, [open, historyEnabled, range, pureWeb, t, reloadNonce]);

  // 手动刷新（清除后调用）
  const loadData = useCallback(() => setReloadNonce((n) => n + 1), []);

  // 派生默认选中设备：未显式选择时取第一个
  const effectiveDeviceKey = selectedDeviceKey || response?.devices[0]?.key || '';

  const selectedDevice = useMemo(
    () => response?.devices.find((d) => d.key === effectiveDeviceKey),
    [response, effectiveDeviceKey],
  );

  const selectedSeries = useMemo(
    () => response?.series.find((s) => s.key === effectiveDeviceKey),
    [response, effectiveDeviceKey],
  );

  // 按选中设备过滤洞察：deviceKey 为空的洞察（如设备对比）始终展示。
  const selectedInsights = useMemo(
    () => (response?.insights ?? []).filter(
      (i) => !i.deviceKey || i.deviceKey === effectiveDeviceKey,
    ),
    [response, effectiveDeviceKey],
  );

  const handleClear = useCallback(async () => {
    if (pureWeb) {
      if (effectiveDeviceKey) {
        setResponse((current) => current
          ? {
              ...current,
              devices: current.devices.filter((d) => d.key !== effectiveDeviceKey),
              series: current.series.filter((s) => s.key !== effectiveDeviceKey),
              insights: current.insights.filter((i) => i.deviceKey !== effectiveDeviceKey),
            }
          : null);
      } else {
        setResponse(null);
      }
      setConfirmingClear(false);
      notifySuccess(t('batteryUsage.clearDone'));
      return;
    }
    try {
      await invoke('battery_history_clear', { deviceKey: effectiveDeviceKey || undefined });
      setSelectedDeviceKey('');
      setConfirmingClear(false);
      notifySuccess(t('batteryUsage.clearDone'));
      loadData();
    } catch (err) {
      notifyError(t('batteryUsage.clear'), String(err));
    }
  }, [effectiveDeviceKey, pureWeb, loadData, t]);

  const handleExport = useCallback(async (format: 'json' | 'csv') => {
    try {
      const ext = format === 'csv' ? 'csv' : 'json';
      if (pureWeb) {
        // pureWeb 预览：从 response 生成对应格式内容。
        let content: string;
        let mime: string;
        if (format === 'csv') {
          const rows = ['deviceKey,bucketStart,bucketLabel,percentage,charging,lowBattery'];
          for (const series of response?.series ?? []) {
            for (const p of series.points) {
              rows.push(`${series.key},${p.bucketStart},${p.bucketLabel},${p.percentage ?? ''},${p.charging ?? ''},${p.lowBattery ?? ''}`);
            }
          }
          content = rows.join('\n');
          mime = 'text/csv';
        } else {
          content = JSON.stringify(response, null, 2);
          mime = 'application/json';
        }
        const blob = new Blob([content], { type: mime });
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = `battery-history.${ext}`;
        a.click();
        URL.revokeObjectURL(url);
        notifySuccess(t('batteryUsage.exportDone'));
        return;
      }
      const filePath = await save({
        defaultPath: `battery-history.${ext}`,
        filters: [{ name: format.toUpperCase(), extensions: [ext] }],
      });
      if (filePath) {
        // 后端写入文件：battery_history_export 接收 path 参数时直接写盘
        await invoke<string>('battery_history_export', { format, path: filePath });
        notifySuccess(t('batteryUsage.exportDone'));
      }
    } catch (err) {
      notifyError(t('batteryUsage.exportFailed'), String(err));
    }
  }, [pureWeb, response, t]);

  if (!open) return null;

  // 功能关闭。
  if (!historyEnabled) {
    return (
      <div className="battery-usage-modal-overlay" onClick={onClose}>
        <div className="battery-usage-modal" onClick={(e) => e.stopPropagation()}>
          <BatteryHistoryDisabledState onClose={onClose} />
        </div>
      </div>
    );
  }

  // 设备不支持电量上报。
  if (!hasBattery && !pureWeb) {
    return (
      <div className="battery-usage-modal-overlay" onClick={onClose}>
        <div className="battery-usage-modal" onClick={(e) => e.stopPropagation()}>
          <BatteryHistoryUnsupportedState onClose={onClose} />
        </div>
      </div>
    );
  }

  return (
    <div className="battery-usage-modal-overlay" onClick={onClose}>
      <div className="battery-usage-modal" onClick={(e) => e.stopPropagation()}>
        {/* 标题区 */}
        <div className="battery-usage-header">
          <div>
            <h2>{t('batteryUsage.title')}</h2>
          </div>
          <button className="battery-usage-close-icon" onClick={onClose} aria-label={t('batteryUsage.close')}>
            <X weight="regular" />
          </button>
        </div>

        {/* 无数据空状态 */}
        {!loading && (!response || response.devices.length === 0) ? (
          <BatteryHistoryEmptyState onClose={onClose} />
        ) : (
          <>
            {/* 时间范围切换 */}
            <div className="battery-usage-controls">
              <div className="battery-range-toggle" role="tablist">
                <button
                  role="tab"
                  aria-selected={range === '24h'}
                  className={range === '24h' ? 'active' : ''}
                  onClick={() => setRange('24h')}
                >
                  {t('batteryUsage.range24h')}
                </button>
                <button
                  role="tab"
                  aria-selected={range === '10d'}
                  className={range === '10d' ? 'active' : ''}
                  onClick={() => setRange('10d')}
                >
                  {t('batteryUsage.range10d')}
                </button>
              </div>
            </div>

            {/* 设备状态条：多设备时点击可切换 */}
            <BatteryUsageStatusStrip
              device={selectedDevice}
              devices={response?.devices ?? []}
              insights={selectedInsights}
              onSelectDevice={setSelectedDeviceKey}
            />
            <BatteryUsageSummary
              device={selectedDevice}
              insights={selectedInsights}
              range={range}
            />

            {/* 图表 */}
            {selectedSeries && (
              <BatteryUsageChart points={selectedSeries.points} range={range} />
            )}

            {/* 洞察 */}
            <BatteryInsightCards insights={selectedInsights} />

            {/* 操作区 */}
            <div className="battery-history-actions">
              {confirmingClear ? (
                <div className="clear-confirm-bar">
                  <span>{t('batteryUsage.clearConfirm')}</span>
                  <button className="danger" onClick={handleClear}>{t('batteryUsage.clearHistoryConfirm')}</button>
                  <button onClick={() => setConfirmingClear(false)}>{t('common.cancel')}</button>
                </div>
              ) : (
                <>
                  <button className="action-btn" onClick={() => setConfirmingClear(true)}>
                    <Trash weight="regular" /> {t('batteryUsage.clearHistory')}
                  </button>
                  <button className="action-btn" onClick={() => handleExport('json')}>
                    <Download weight="regular" /> {t('batteryUsage.exportJson')}
                  </button>
                  <button className="action-btn" onClick={() => handleExport('csv')}>
                    <Download weight="regular" /> {t('batteryUsage.exportCsv')}
                  </button>
                </>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
