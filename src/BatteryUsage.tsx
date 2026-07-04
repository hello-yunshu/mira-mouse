// SPDX-License-Identifier: AGPL-3.0-or-later
import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import { BatteryHigh, Lightning, Warning, X, Trash, Download, Clock, ChartBar } from '@phosphor-icons/react';
import type {
  BatteryHistoryRange,
  BatteryHistoryResponse,
  BatteryHistoryDevice,
  BatteryHistoryPoint,
  BatteryInsight,
} from './types';
import { MOCK_BATTERY_HISTORY_24H, MOCK_BATTERY_HISTORY_10D } from './mock';
import { notifyError, notifySuccess } from './notify';

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
  const height = 160;
  const padding = { top: 12, right: 8, bottom: 24, left: 28 };
  const chartWidth = width - padding.left - padding.right;
  const chartHeight = height - padding.top - padding.bottom;
  const barGap = 2;
  const barWidth = (chartWidth - barGap * (points.length - 1)) / points.length;

  const activeIndex = hoverIndex ?? focusIndex;
  const activePoint = activeIndex !== null ? points[activeIndex] : null;

  const yTicks = [0, 25, 50, 75, 100];

  return (
    <div className="battery-chart-card">
      <div className="battery-chart-header">
        <span><ChartBar weight="regular" /> {t('batteryUsage.title')}</span>
      </div>
      <svg
        className="battery-chart"
        viewBox={`0 0 ${width} ${height}`}
        role="img"
        aria-label={t('batteryUsage.title')}
      >
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
          const x = padding.left + i * (barWidth + barGap);
          const hasData = point.percentage !== undefined;
          const pct = point.percentage ?? 0;
          const barH = hasData ? (pct / 100) * chartHeight : 0;
          const y = padding.top + chartHeight - barH;
          const isCharging = point.charging ?? false;
          const isLow = point.lowBattery ?? false;
          const isEmpty = !hasData;

          let barClass = 'battery-chart-bar';
          if (isEmpty) barClass += ' battery-chart-empty';
          else if (isCharging) barClass += ' battery-chart-charging';
          else if (isLow) barClass += ' battery-chart-low';

          return (
            <g
              key={i}
              onMouseEnter={() => setHoverIndex(i)}
              onMouseLeave={() => setHoverIndex(null)}
              onFocus={() => setFocusIndex(i)}
              onBlur={() => setFocusIndex(null)}
              tabIndex={0}
              role="button"
              aria-label={`${point.bucketLabel}: ${hasData ? `${pct}%` : t('batteryUsage.notEnoughData')}`}
            >
              <rect
                x={x}
                y={isEmpty ? padding.top + chartHeight - 2 : y}
                width={barWidth}
                height={isEmpty ? 2 : barH}
                rx={1.5}
                className={barClass}
              />
              {/* X 轴标签：稀疏显示 */}
              {(range === '24h' ? i % 4 === 0 : i % 2 === 0) && (
                <text
                  x={x + barWidth / 2}
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
        <div className="battery-chart-tooltip" role="tooltip">
          <div className="tooltip-row"><strong>{t('batteryUsage.tooltipTime')}: </strong>{activePoint.bucketLabel}</div>
          <div className="tooltip-row"><strong>{t('batteryUsage.tooltipPercentage')}: </strong>{activePoint.percentage}%</div>
          {activePoint.minPercentage !== undefined && activePoint.maxPercentage !== undefined && (
            <div className="tooltip-row"><strong>{t('batteryUsage.tooltipMin')}/{t('batteryUsage.tooltipMax')}: </strong>{activePoint.minPercentage}%-{activePoint.maxPercentage}%</div>
          )}
          <div className="tooltip-row"><strong>{t('batteryUsage.tooltipCharging')}: </strong>{activePoint.charging ? t('common.on') : t('common.off')}</div>
          <div className="tooltip-row"><strong>{t('batteryUsage.tooltipLowBattery')}: </strong>{activePoint.lowBattery ? t('common.on') : t('common.off')}</div>
          <div className="tooltip-row"><strong>{t('batteryUsage.tooltipSamples')}: </strong>{activePoint.sampleCount}</div>
        </div>
      )}
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
      <div className="battery-summary-item">
        <span className="summary-label">{t('batteryUsage.currentBattery')}</span>
        <strong className="summary-value">{device.latestPercentage ?? '--'}%</strong>
        {device.latestAt && <span className="summary-sub">{formatRelativeTime(device.latestAt, t)}</span>}
      </div>
      <div className="battery-summary-item">
        <span className="summary-label">{t('batteryUsage.change' + (range === '24h' ? '24h' : '10d'))}</span>
        <strong className="summary-value">{statusText}</strong>
        <span className="summary-sub">{charging ? t('batteryUsage.charging') : t('batteryUsage.notCharging')}</span>
      </div>
      <div className="battery-summary-item">
        <span className="summary-label">{t('batteryUsage.estimatedRemaining')}</span>
        <strong className="summary-value">{remaining?.message ?? t('batteryUsage.notEnoughData')}</strong>
      </div>
      <div className="battery-summary-item">
        <span className="summary-label">{t('batteryUsage.estimatedRunout')}</span>
        <strong className="summary-value">{!charging ? (runout?.message ?? t('batteryUsage.notEnoughData')) : t('batteryUsage.charging')}</strong>
      </div>
    </div>
  );
}

// ─── 洞察卡片 ───────────────────────────────────────────────────────────────

function BatteryInsightCards({ insights }: { insights: BatteryInsight[] }) {
  const { t } = useTranslation();
  if (insights.length === 0) return null;

  const iconFor = (type: BatteryInsight['type']) => {
    switch (type) {
      case 'abnormalDrain': return <Warning weight="regular" />;
      case 'powerSavingTip': return <Lightning weight="regular" />;
      default: return <Clock weight="regular" />;
    }
  };

  return (
    <div className="battery-insight-cards">
      {insights.map((insight, i) => (
        <div key={i} className={`battery-insight-card severity-${insight.severity}`}>
          <span className="insight-icon">{iconFor(insight.type)}</span>
          <div className="insight-body">
            <strong>{t(`batteryUsage.${insight.title}`)}</strong>
            <span>{insight.message}</span>
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
      setResponse(null);
      setConfirmingClear(false);
      notifySuccess(t('batteryUsage.clearDone'));
      return;
    }
    try {
      await invoke('battery_history_clear');
      setResponse(null);
      setConfirmingClear(false);
      notifySuccess(t('batteryUsage.clearDone'));
      loadData();
    } catch (err) {
      notifyError(t('batteryUsage.clear'), String(err));
    }
  }, [pureWeb, loadData, t]);

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
            <p>{t('batteryUsage.subtitle')}</p>
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
            {/* 设备选择 + 时间范围 */}
            <div className="battery-usage-controls">
              <div className="battery-device-selector" role="tablist">
                {response?.devices.map((device) => (
                  <button
                    key={device.key}
                    role="tab"
                    aria-selected={effectiveDeviceKey === device.key}
                    className={`battery-device-chip ${effectiveDeviceKey === device.key ? 'active' : ''}`}
                    onClick={() => setSelectedDeviceKey(device.key)}
                  >
                    <BatteryHigh weight="regular" />
                    {device.deviceName} · {t(device.componentLabel, { defaultValue: device.componentLabel })}
                  </button>
                ))}
              </div>
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

            {/* 摘要 */}
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
