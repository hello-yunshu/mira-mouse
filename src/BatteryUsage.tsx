// SPDX-License-Identifier: AGPL-3.0-or-later
import { forwardRef, memo, useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import { Warning, X, Trash, Download, Clock, ChartBar, CaretUpDown, Plug, Gauge, ArrowsLeftRight, Lightbulb, TrendDown, BatteryCharging, BatteryLow } from '@phosphor-icons/react';
import type {
  AppSettings,
  BatteryHistoryRange,
  BatteryHistoryResponse,
  BatteryHistoryDevice,
  BatteryHistoryPoint,
  BatteryInsight,
} from './types';
import { MOCK_BATTERY_HISTORY_24H, MOCK_BATTERY_HISTORY_10D } from './mock';
import { notifyError, notifySuccess } from './notify';
import { BatteryLevelIcon } from './BatteryLevelIcon';
import { LOCAL_AI_FEATURE, localAiFeatureEnabled } from './localAi';
import { segmentedIndicatorStyle } from './segmentedControl';
import { Modal } from './overlay';
import { Tooltip } from './Tooltip';

// ─── 工具函数 ───────────────────────────────────────────────────────────────

function isPureWebPreview(): boolean {
  return !('__TAURI_INTERNALS__' in window);
}

// 切换 battery range 时后端返回新 response 引用但内容常相同，新引用会让下游
// memo 浅比较失效，触发 StatusStrip/Summary 重渲染与 CSS transition/animation 抖动。
// equal 函数 + useStable 在内容相同时复用旧引用，让 memo 浅比较生效。
function batteryDeviceEqual(a: BatteryHistoryDevice | undefined, b: BatteryHistoryDevice | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return (
    a.key === b.key &&
    a.deviceId === b.deviceId &&
    a.deviceName === b.deviceName &&
    a.connection === b.connection &&
    a.componentId === b.componentId &&
    a.componentLabel === b.componentLabel &&
    a.latestPercentage === b.latestPercentage &&
    a.latestCharging === b.latestCharging &&
    a.latestAt === b.latestAt &&
    a.lowBattery === b.lowBattery
  );
}

function batteryInsightEqual(a: BatteryInsight | undefined, b: BatteryInsight | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return (
    a.type === b.type &&
    a.severity === b.severity &&
    a.title === b.title &&
    a.message === b.message &&
    a.deviceKey === b.deviceKey
  );
}

const deviceArrayEqual = (a: readonly BatteryHistoryDevice[], b: readonly BatteryHistoryDevice[]): boolean =>
  a.length === b.length && a.every((d, i) => batteryDeviceEqual(d, b[i]));

const insightArrayEqual = (a: readonly BatteryInsight[], b: readonly BatteryInsight[]): boolean =>
  a.length === b.length && a.every((x, i) => batteryInsightEqual(x, b[i]));

/**
 * 引用稳定 hook：内容相同时复用旧引用，让下游 memo 浅比较生效。
 * 用于切换 battery range 时 response 引用变化但内容相同的场景。
 *
 * 实现说明：用 useState 持有「上一次返回的引用」，render 中比较内容——
 * 内容相同则返回旧引用（命中下游 memo）；内容变化则 setState 触发一次
 * 重渲染，并直接返回新引用，避免本次渲染先用旧值再切到新值的视觉跳跃。
 * 这符合 React 关于「storing information from previous renders」的官方
 * 用法（https://react.dev/reference/react/useState#storing-information-from-previous-renders）。
 */
function useStable<T>(value: T, equal: (a: T, b: T) => boolean): T {
  const [stable, setStable] = useState<T>(value);
  if (!equal(stable, value)) {
    setStable(value);
    return value;
  }
  return stable;
}

function formatRelativeTime(iso: string, t: (key: string, options?: Record<string, unknown>) => string): string {
  const now = Date.now();
  const then = new Date(iso).getTime();
  const diff = now - then;
  if (diff < 60_000) return t('batteryUsage.lastUpdated');
  const minutes = Math.floor(diff / 60_000);
  if (minutes < 60) return t('batteryUsage.relativeMinutes', { count: minutes });
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return t('batteryUsage.relativeHours', { count: hours });
  const days = Math.floor(hours / 24);
  return t('batteryUsage.relativeDays', { count: days });
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

// ─── 文本淡入淡出 ───────────────────────────────────────────────────────────
// 双 span 交叉 opacity 过渡：旧值淡出 + 新值淡入，单层 opacity + 1px translateY，
// 220ms 时长（对齐 motion-base）。独立 .battery-fade-text 类避免与 LiveValue 共享样式。
// 多行模式：next span 用 absolute 脱离文档流，容器高度仅由 current 决定，避免新旧文案
// 行数不同时撑开容器导致重排。
// delay prop：父组件按子块位置传不同延迟（如 0/40/80/120ms），让多个文本错落切换；
// delay 同时作用于 CSS transition-delay 和 setTimeout 时长，保证过渡时序与状态切换同步。
const FadeText = forwardRef<HTMLSpanElement, {
  text: string;
  className?: string;
  multiline?: boolean;
  delay?: number;
}>(function FadeText({ text, className, multiline, delay = 0 }, ref) {
  const [currentValue, setCurrentValue] = useState(text);
  const [nextValue, setNextValue] = useState<string | undefined>(undefined);
  const [transitioning, setTransitioning] = useState(false);

  useEffect(() => {
    if (text === currentValue) return;
    let prepareFrame = 0;
    let transitionFrame = 0;
    let timeout = 0;
    prepareFrame = window.requestAnimationFrame(() => {
      setNextValue(text);
      setTransitioning(false);
      transitionFrame = window.requestAnimationFrame(() => {
        setTransitioning(true);
        timeout = window.setTimeout(() => {
          setCurrentValue(text);
          setNextValue(undefined);
          setTransitioning(false);
        }, 220 + delay);
      });
    });
    return () => {
      window.cancelAnimationFrame(prepareFrame);
      window.cancelAnimationFrame(transitionFrame);
      window.clearTimeout(timeout);
    };
  }, [currentValue, text, delay]);

  return (
    <span
      ref={ref}
      style={{ '--fade-delay': `${delay}ms` } as CSSProperties}
      className={[
        className,
        multiline ? 'battery-fade-multiline' : 'battery-fade-text',
        transitioning ? 'is-transitioning' : undefined,
      ].filter(Boolean).join(' ')}
      aria-label={text}
    >
      <span className="battery-fade-current" aria-hidden="true">{currentValue}</span>
      {nextValue !== undefined && (
        <span className="battery-fade-next" aria-hidden="true">{nextValue}</span>
      )}
    </span>
  );
});

// ─── 溢出文字弹窗 ───────────────────────────────────────────────────────────
// 检测文字溢出容器时复用 Tooltip 显示完整内容。Tooltip 通过 OverlayPortal 渲染到
// 顶层 #mira-overlay-root，避免被祖先 overflow: hidden / overflow-y: auto 裁切。
// 内部用 FadeText 渲染，让 summary 文本在内容变化时获得淡入淡出动画。
function OverflowTip({ text, className, multiline, delay }: { text: string; className?: string; multiline?: boolean; delay?: number }) {
  const ref = useRef<HTMLSpanElement>(null);
  const [overflowed, setOverflowed] = useState(false);

  const checkOverflow = useCallback(() => {
    const element = ref.current;
    if (!element) return;
    setOverflowed(
      multiline
        ? element.scrollHeight > element.clientHeight
        : element.scrollWidth > element.clientWidth,
    );
  }, [multiline]);

  useEffect(() => {
    checkOverflow();
    const observer = new ResizeObserver(checkOverflow);
    if (ref.current) observer.observe(ref.current);
    return () => observer.disconnect();
  }, [checkOverflow]);

  const content = <FadeText ref={ref} text={text} className={className} multiline={multiline} delay={delay} />;
  return overflowed ? <Tooltip label={text}>{content}</Tooltip> : content;
}

// ─── SVG 图表 ───────────────────────────────────────────────────────────────

interface ChartProps {
  points: BatteryHistoryPoint[];
  range: BatteryHistoryRange;
  generatedAt?: string;
}

function BatteryUsageChart({ points, range, generatedAt }: ChartProps) {
  const { t, i18n } = useTranslation();
  const [hoverIndex, setHoverIndex] = useState<number | null>(null);
  const [focusIndex, setFocusIndex] = useState<number | null>(null);

  const width = 520;
  // 两种范围共用同一画布高度，避免切换时图表卡片跳动。10d 在底部给日期 + 图例留位。
  const height = 162;
  const padding = { top: 8, right: 8, bottom: range === '24h' ? 18 : 46, left: 28 };
  const chartWidth = width - padding.left - padding.right;
  const chartHeight = height - padding.top - padding.bottom;
  const pointCount = Math.max(points.length, 1);
  const slotWidth = chartWidth / pointCount;
  // 24h 最多显示 48 个聚合点、10d 固定 30 点；采样密度由后端独立保留给分析。
  const visualBarWidth = Math.max(2, Math.min(slotWidth * 0.52, slotWidth - 2));

  const recordedPercentages = useMemo(
    () => points.flatMap((point) => point.percentage === undefined ? [] : [point.percentage]),
    [points],
  );
  const averagePercentage = recordedPercentages.length > 0
    ? recordedPercentages.reduce((sum, percentage) => sum + percentage, 0) / recordedPercentages.length
    : null;
  const averageY = averagePercentage === null
    ? null
    : padding.top + chartHeight - (averagePercentage / 100) * chartHeight;
  const averageLabelBelow = averageY !== null && averageY <= padding.top + 12;
  const averageLabelOffset = averageLabelBelow ? 6 : -6;
  const latestRecordedIndex = useMemo(() => {
    for (let index = points.length - 1; index >= 0; index -= 1) {
      if (points[index]?.percentage !== undefined) return index;
    }
    return -1;
  }, [points]);
  // 10d 每天三个 8 小时槽。历史日中从“当前尚未结束的时段”开始降低不透明度，
  // 今天的最新实测值保持高亮；这样全天轨迹与截至现在的轨迹可以直接对照，
  // 同时不把未来时段伪装成预测数据。
  const generatedHour = generatedAt && !Number.isNaN(new Date(generatedAt).getTime())
    ? new Date(generatedAt).getHours()
    : new Date().getHours();
  const currentDaySlot = Math.min(2, Math.floor(generatedHour / 8));
  const currentDayStartIndex = Math.max(0, points.length - 3);

  const activeIndex = hoverIndex ?? focusIndex;
  const activePoint = activeIndex !== null ? points[activeIndex] : null;
  const tooltipStyle = activeIndex !== null
    ? ({ '--tooltip-x': `${((padding.left + activeIndex * slotWidth + slotWidth / 2) / width) * 100}%` } as CSSProperties)
    : undefined;

  // 绘图区圆角克制；切换 range 时让整个绘图区（背景、网格和柱体）一起伸缩。
  // 只给背景 rect 做 height transition 会被同步换新的网格/柱体盖过，视觉上近似瞬切。
  const plotCornerR = 5;

  const yTicks = [0, 25, 50, 75, 100];
  const xTicks = useMemo(() => {
    const locale = i18n.resolvedLanguage ?? i18n.language;
    if (range === '24h') {
      const timestamps = points
        .map((point) => new Date(point.bucketStart).getTime())
        .filter((timestamp) => Number.isFinite(timestamp));
      const startMs = timestamps[0] ?? 0;
      const sampledEndMs = timestamps[timestamps.length - 1] ?? startMs + 24 * 60 * 60 * 1000;
      const slotDuration = timestamps.length > 1
        ? Math.max(1, (sampledEndMs - startMs) / (timestamps.length - 1))
        : 30 * 60 * 1000;
      const endMs = sampledEndMs + slotDuration;
      const spanMs = Math.max(1, endMs - startMs);
      const firstTick = new Date(startMs);
      firstTick.setMinutes(0, 0, 0);
      const hoursToNextTick = (3 - (firstTick.getHours() % 3)) % 3;
      firstTick.setHours(firstTick.getHours() + hoursToNextTick);
      if (firstTick.getTime() < startMs) firstTick.setHours(firstTick.getHours() + 3);

      const ticks = [];
      for (let cursor = firstTick; cursor.getTime() < endMs; cursor = new Date(cursor.getTime() + 3 * 60 * 60 * 1000)) {
        const hour = cursor.getHours();
        const hour12 = hour % 12 || 12;
        const isChinese = locale.toLowerCase().startsWith('zh');
        const label = hour === 0
          ? (isChinese ? `上午${hour12}时` : `${hour12} AM`)
          : hour === 12
            ? (isChinese ? `下午${hour12}时` : `${hour12} PM`)
            : String(hour12);
        const x = padding.left + ((cursor.getTime() - startMs) / spanMs) * chartWidth;
        ticks.push({
          key: `time-${cursor.getTime()}`,
          lineX: x,
          labelX: x,
          label,
          dateLabel: '',
          major: false,
        });
      }
      return ticks;
    }

    return Array.from({ length: Math.ceil(pointCount / 3) }, (_, dayIndex) => {
      const pointIndex = dayIndex * 3;
      const point = points[pointIndex];
      const date = point ? new Date(point.bucketStart) : null;
      const validDate = date && !Number.isNaN(date.getTime()) ? date : null;
      const showDate = dayIndex === 0 || validDate?.getDay() === 1;
      return {
        key: `day-${pointIndex}`,
        lineX: padding.left + pointIndex * slotWidth,
        labelX: padding.left + (pointIndex + 1.5) * slotWidth,
        label: validDate
          ? new Intl.DateTimeFormat(locale, { weekday: 'narrow' }).format(validDate)
          : point?.bucketLabel.slice(0, 5) ?? '',
        dateLabel: showDate && validDate
          ? new Intl.DateTimeFormat(locale, { month: 'numeric', day: 'numeric' }).format(validDate)
          : '',
        major: Boolean(showDate),
      };
    }).filter((tick) => tick.label);
  }, [chartWidth, i18n.language, i18n.resolvedLanguage, padding.left, pointCount, points, range, slotWidth]);

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
            <clipPath id="battery-chart-plot-clip">
              <rect
                className="battery-chart-clip-rect"
                x={padding.left}
                y={padding.top}
                width={chartWidth}
                height={chartHeight}
                style={{ height: `${chartHeight}px` }}
              />
            </clipPath>
            <linearGradient id="battery-bar-normal" x1="0" y1="1" x2="0" y2="0">
              <stop offset="0%" stopColor="#2f9f7a" />
              <stop offset="56%" stopColor="#5fc58f" />
              <stop offset="100%" stopColor="#b7e7c8" />
            </linearGradient>
            <linearGradient id="battery-bar-charging" x1="0" y1="1" x2="0" y2="0">
              <stop offset="0%" stopColor="#4d6fd6" />
              <stop offset="58%" stopColor="#63b7de" />
              <stop offset="100%" stopColor="#c1eef1" />
            </linearGradient>
            <linearGradient id="battery-bar-low" x1="0" y1="1" x2="0" y2="0">
              <stop offset="0%" stopColor="#c85f66" />
              <stop offset="58%" stopColor="#e69b69" />
              <stop offset="100%" stopColor="#f3d38c" />
            </linearGradient>
            <linearGradient id="battery-bar-current" x1="0" y1="1" x2="0" y2="0">
              <stop offset="0%" stopColor="color-mix(in oklch, var(--accent), #6f86dd 30%)" />
              <stop offset="100%" stopColor="color-mix(in oklch, var(--accent), white 38%)" />
            </linearGradient>
          </defs>
          <g className={`battery-chart-plot-content range-${range}`}>
            <rect
              className="battery-chart-plot"
              x={padding.left}
              y={padding.top}
              width={chartWidth}
              height={chartHeight}
              rx={plotCornerR}
            />
            {yTicks.map((tick) => {
              const y = padding.top + chartHeight - (tick / 100) * chartHeight;
              return (
                <line
                  key={tick}
                  x1={padding.left}
                  y1={y}
                  x2={width - padding.right}
                  y2={y}
                  stroke="var(--muted)"
                  strokeOpacity={0.12}
                  strokeWidth={1}
                  clipPath="url(#battery-chart-plot-clip)"
                />
              );
            })}

            {/* 绘图区内的 X 轴网格随背景一起伸缩。 */}
            {xTicks.map((tick) => (
                <line
                  key={`${tick.key}-grid`}
                  className={`battery-chart-x-grid${tick.major ? ' major' : ''}`}
                  x1={tick.lineX}
                  y1={padding.top}
                  x2={tick.lineX}
                  y2={padding.top + chartHeight}
                  clipPath="url(#battery-chart-plot-clip)"
                />
            ))}

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
              const isCurrent = i === latestRecordedIndex;
              const isAfterNow = range === '10d'
                && i < currentDayStartIndex
                && i % 3 >= currentDaySlot;
              const fillId = isCharging
                ? 'battery-bar-charging'
                : isLow
                  ? 'battery-bar-low'
                  : isCurrent
                    ? 'battery-bar-current'
                    : 'battery-bar-normal';

              let barClass = 'battery-chart-bar';
              if (isEmpty) barClass += ' battery-chart-empty';
              else if (isCharging) barClass += ' battery-chart-charging';
              else if (isLow) barClass += ' battery-chart-low';
              if (isCurrent) barClass += ' battery-chart-current';
              if (isAfterNow) barClass += ' battery-chart-after-now';

              // 顶部克制圆角、底部平直：用 path 绘制仅上方两角圆角的柱体
              const cornerR = Math.min(visualBarWidth / 2, renderedBarH / 2, 4.5);
              const barPath = `M ${x},${y + renderedBarH} L ${x},${y + cornerR} Q ${x},${y} ${x + cornerR},${y} L ${x + visualBarWidth - cornerR},${y} Q ${x + visualBarWidth},${y} ${x + visualBarWidth},${y + cornerR} L ${x + visualBarWidth},${y + renderedBarH} Z`;

              return (
                <g
                  key={`${range}-${point.bucketStart}-${i}`}
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
                  <path
                    d={barPath}
                    className={barClass}
                    fill={isEmpty ? undefined : `url(#${fillId})`}
                    style={{ '--bar-delay': `${Math.min(i, 10) * 6}ms` } as CSSProperties}
                  />
                </g>
              );
            })}

          </g>

          {/* 均值线独立于会缩放的绘图区，切换范围时沿 Y 轴平顺移动。 */}
          {averageY !== null && (
            <g
              className="battery-chart-average"
              aria-label={t('batteryUsage.averageLine', { value: Math.round(averagePercentage ?? 0) })}
              style={{ transform: `translateY(${averageY}px)` }}
            >
              <line
                className="battery-chart-average-line"
                x1={padding.left}
                y1={0}
                x2={width - padding.right}
                y2={0}
              />
              <text
                key={`${range}-${Math.round(averagePercentage ?? 0)}`}
                className="battery-chart-average-label"
                x={width - padding.right}
                y={averageLabelOffset}
                textAnchor="end"
                dominantBaseline={averageLabelBelow ? 'text-before-edge' : 'text-after-edge'}
              >
                {t('batteryUsage.averageLine', { value: Math.round(averagePercentage ?? 0) })}
              </text>
            </g>
          )}

          {/* 纵轴数字保持原始比例，逐个跟随各自对应的网格线移动。 */}
          <g className={`battery-chart-y-axis range-${range}`}>
            {yTicks.map((tick) => {
              const y = padding.top + chartHeight - (tick / 100) * chartHeight;
              const shift = (range === '24h' ? -1 : 1) * 28 * (1 - tick / 100);
              return (
                <text
                  key={`${tick}-label`}
                  className="battery-chart-y-label"
                  x={padding.left - 6}
                  y={y + 3}
                  textAnchor="end"
                  fontSize={9}
                  fill="var(--muted)"
                  style={{ '--axis-shift': `${shift}px` } as CSSProperties}
                >
                  {tick}
                </text>
              );
            })}
          </g>

          {/* 横轴稍后整体浮现，让视线先读懂绘图区的高度变化。 */}
          <g className={`battery-chart-x-axis range-${range}`}>
            {xTicks.map((tick) => {
              const plotBottom = padding.top + chartHeight;
              const extensionBottom = range === '10d'
                ? (tick.major ? plotBottom + 28 : plotBottom + 13)
                : plotBottom + 15;
              return (
                <line
                  key={`${tick.key}-extension`}
                  className={`battery-chart-x-extension${tick.major ? ' major' : ''}`}
                  x1={tick.lineX}
                  y1={plotBottom}
                  x2={tick.lineX}
                  y2={extensionBottom}
                />
              );
            })}

            {xTicks.map((tick, index) => (
              <g key={`${tick.key}-label`} className="battery-chart-x-tick">
                <text
                  className="battery-chart-x-label"
                  x={tick.labelX}
                  y={padding.top + chartHeight + 13}
                  textAnchor={range === '24h' && index === 0 && tick.labelX - padding.left < 14 ? 'start' : 'middle'}
                >
                  {tick.label}
                </text>
                {tick.dateLabel && (
                  <text
                    className="battery-chart-x-date"
                    x={tick.lineX + 4}
                    y={padding.top + chartHeight + 26}
                    textAnchor="start"
                  >
                    {tick.dateLabel}
                  </text>
                )}
              </g>
            ))}
          </g>
        </svg>

        {activePoint && activePoint.percentage !== undefined && (
          <div className="battery-chart-tooltip" style={tooltipStyle} role="tooltip">
            <div className="tooltip-row"><strong>{t(range === '24h' ? 'batteryUsage.tooltipTime' : 'batteryUsage.tooltipDate')}: </strong><span>{activePoint.bucketLabel}</span></div>
            <div className="tooltip-row"><strong>{t('batteryUsage.tooltipPercentage')}: </strong><span>{activePoint.percentage}%</span></div>
            {activePoint.minPercentage !== undefined && activePoint.maxPercentage !== undefined && (
              <div className="tooltip-row"><strong>{t('batteryUsage.tooltipMin')}/{t('batteryUsage.tooltipMax')}: </strong><span>{activePoint.minPercentage}%-{activePoint.maxPercentage}%</span></div>
            )}
            <div className="tooltip-row"><strong>{t('batteryUsage.tooltipCharging')}: </strong><span>{t(activePoint.charging ? 'batteryUsage.tooltipYes' : 'batteryUsage.tooltipNo')}</span></div>
            <div className="tooltip-row"><strong>{t('batteryUsage.tooltipLowBattery')}: </strong><span>{activePoint.lowBattery ? t('common.on') : t('common.off')}</span></div>
            <div className="tooltip-row"><strong>{t('batteryUsage.tooltipSamples')}: </strong><span>{activePoint.sampleCount}</span></div>
          </div>
        )}
        {range === '10d' && (
          <div className="battery-chart-legend" aria-label={t('batteryUsage.comparisonLegend')}>
            <span><i className="through-now" aria-hidden="true" />{t('batteryUsage.throughNow')}</span>
            <span><i className="all-day" aria-hidden="true" />{t('batteryUsage.allDay')}</span>
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

// 摘要卡片拆成 4 个 memo 子块：切换 range 时只有真正变化的块重渲染，
// 不变块的 DOM 与子组件实例完全复用，避免电池图标重渲染触发 CSS 抖动。
// 每个子块自己调用 useTranslation() —— react-i18next 在相同 i18n 状态下
// t 引用稳定，子块各自调用更解耦，且不依赖父层 t 引用。

// 子块 1：当前电量（不依赖 range，依赖 device）
const CurrentBatteryItem = memo(function CurrentBatteryItem({ device, delay }: { device: BatteryHistoryDevice; delay?: number }) {
  const { t } = useTranslation();
  const charging = device.latestCharging ?? false;
  return (
    <div className="battery-summary-item primary">
      <span className="summary-label with-icon">
        <BatteryLevelIcon percentage={device.latestPercentage} charging={charging} />
        {t('batteryUsage.currentBattery')}
      </span>
      <OverflowTip className="summary-value" text={`${device.latestPercentage ?? '--'}%`} delay={delay} />
      {device.latestAt && <OverflowTip className="summary-sub" text={formatRelativeTime(device.latestAt, t)} delay={delay} />}
    </div>
  );
});

// 子块 2：24h/10d 变化（依赖 range + charging/lowBattery）
const BatteryChangeItem = memo(function BatteryChangeItem({
  range,
  charging,
  lowBattery,
  delay,
}: {
  range: BatteryHistoryRange;
  charging: boolean;
  lowBattery: boolean;
  delay?: number;
}) {
  const { t } = useTranslation();
  const statusText = charging
    ? t('batteryUsage.charging')
    : lowBattery
      ? t('batteryUsage.lowBattery')
      : t('batteryUsage.normal');
  return (
    <div className="battery-summary-item">
      <OverflowTip className="summary-label" text={t('batteryUsage.change' + (range === '24h' ? '24h' : '10d'))} delay={delay} />
      <OverflowTip className="summary-value" text={statusText} delay={delay} />
      <OverflowTip className="summary-sub" text={charging ? t('batteryUsage.charging') : t('batteryUsage.notCharging')} delay={delay} />
    </div>
  );
});

// 子块 3：预计剩余（依赖 insights.remaining，自定义比较避免 find 返回新引用导致 memo 失效）
const EstimatedRemainingItem = memo(
  function EstimatedRemainingItem({ remaining, delay }: { remaining: BatteryInsight | undefined; delay?: number }) {
    const { t } = useTranslation();
    return (
      <div className="battery-summary-item">
        <OverflowTip className="summary-label" text={t('batteryUsage.estimatedRemaining')} delay={delay} />
        <OverflowTip className="summary-value" text={remaining ? formatInsightMessage(remaining, t) : t('batteryUsage.notEnoughData')} delay={delay} />
      </div>
    );
  },
  (prev, next) => batteryInsightEqual(prev.remaining, next.remaining),
);

// 子块 4：预计耗尽（依赖 insights.runout + charging）
const EstimatedRunoutItem = memo(
  function EstimatedRunoutItem({ runout, charging, delay }: { runout: BatteryInsight | undefined; charging: boolean; delay?: number }) {
    const { t } = useTranslation();
    return (
      <div className="battery-summary-item">
        <OverflowTip className="summary-label" text={t('batteryUsage.estimatedRunout')} delay={delay} />
        <OverflowTip className="summary-value" text={!charging ? (runout?.message ?? t('batteryUsage.notEnoughData')) : t('batteryUsage.charging')} delay={delay} />
      </div>
    );
  },
  (prev, next) => batteryInsightEqual(prev.runout, next.runout) && prev.charging === next.charging,
);

// 父组件：派生数据并组装 4 个子块。device/insights 引用已通过 useStable 稳定，
// 切换 range 时若内容相同则子块全部命中 memo 缓存。
// delay 按子块位置错落分配（0/40/80/120ms），让多个文本错开过渡。
function BatteryUsageSummary({ device, insights, range }: SummaryProps) {
  if (!device) return null;
  const charging = device.latestCharging ?? false;
  const lowBattery = device.lowBattery ?? false;
  const remaining = insights.find((i) => i.type === 'estimatedRemaining');
  const runout = insights.find((i) => i.type === 'estimatedRunout');
  return (
    <div className="battery-summary-grid">
      <CurrentBatteryItem device={device} delay={0} />
      <BatteryChangeItem range={range} charging={charging} lowBattery={lowBattery} delay={40} />
      <EstimatedRemainingItem remaining={remaining} delay={80} />
      <EstimatedRunoutItem runout={runout} charging={charging} delay={120} />
    </div>
  );
}

const BatteryUsageStatusStrip = memo(function BatteryUsageStatusStrip({
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
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuId = useId();

  useEffect(() => {
    if (!menuOpen) return;
    const handlePointerDown = (e: PointerEvent) => {
      if (!stripRef.current?.contains(e.target as Node)) setMenuOpen(false);
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      setMenuOpen(false);
      triggerRef.current?.focus();
    };
    document.addEventListener('pointerdown', handlePointerDown);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown);
      document.removeEventListener('keydown', handleKeyDown);
    };
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
      className="battery-status-strip-shell"
    >
      <button
        ref={triggerRef}
        type="button"
        className={`battery-status-strip ${charging ? 'charging' : lowBattery ? 'low' : 'normal'} ${hasMultipleDevices ? 'switchable' : ''}`}
        aria-expanded={hasMultipleDevices ? menuOpen : undefined}
        aria-haspopup={hasMultipleDevices ? 'menu' : undefined}
        aria-controls={hasMultipleDevices ? menuId : undefined}
        aria-label={hasMultipleDevices ? t('batteryUsage.switchDevice') : undefined}
        disabled={!hasMultipleDevices}
        onClick={toggleMenu}
      >
        <div className="battery-status-device">
          <BatteryLevelIcon percentage={device.latestPercentage} charging={charging} />
          <div className="battery-status-device-info">
            <span><FadeText text={device.deviceName} delay={0} /></span>
            <strong><FadeText text={t(device.componentLabel, { defaultValue: device.componentLabel })} delay={30} /></strong>
          </div>
        </div>
        <div className="battery-status-metric">
          <strong><FadeText text={`${device.latestPercentage ?? '--'}%`} delay={60} /></strong>
          <span><FadeText text={remaining ? formatInsightMessage(remaining, t) : statusText} delay={90} /></span>
        </div>
        {hasMultipleDevices && (
          <CaretUpDown weight="thin" className="battery-status-switch-icon" aria-hidden="true" />
        )}
      </button>

      {menuOpen && hasMultipleDevices && (
        <div id={menuId} className="battery-device-menu" role="menu">
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
                  setMenuOpen(false);
                  onSelectDevice(d.key);
                  triggerRef.current?.focus();
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
});

// ─── 洞察卡片筛选 ───────────────────────────────────────────────────────────
// 从全量 insights 中筛出可展示的卡片：去重 → 特殊洞察置顶 + 基础洞察按优先级补齐
// → 截断到 maxCount → 偶数化（固定 2 列布局，避免单块占行）。
// 抽出为独立函数，让 BatteryUsageModal 对 24h/10d 各跑一次取最小值作为 maxCount，
// 实现「两个 range 卡片数一致」的视觉稳定性。
const INSIGHT_SPECIAL_TYPES: BatteryInsight['type'][] = ['abnormalDrain', 'powerSavingTip'];
const INSIGHT_BASIC_PRIORITY: BatteryInsight['type'][] = ['chargingHabit', 'batteryConsistency', 'averageDailyDrain', 'chargingCount'];
const INSIGHT_DEDUP_TYPES: ReadonlySet<BatteryInsight['type']> = new Set(['estimatedRemaining', 'estimatedRunout', 'deviceComparison', 'lowestLevel']);
const INSIGHT_HARD_MAX = 6;

function filterInsightsForCards(insights: BatteryInsight[], maxCount: number = INSIGHT_HARD_MAX): BatteryInsight[] {
  const effectiveMax = Math.max(0, Math.min(maxCount, INSIGHT_HARD_MAX));
  const deduped = insights.filter((i) => !INSIGHT_DEDUP_TYPES.has(i.type));
  const special = deduped.filter((i) => INSIGHT_SPECIAL_TYPES.includes(i.type));
  const basic = deduped
    .filter((i) => !INSIGHT_SPECIAL_TYPES.includes(i.type))
    .sort((a, b) => {
      const pa = INSIGHT_BASIC_PRIORITY.indexOf(a.type);
      const pb = INSIGHT_BASIC_PRIORITY.indexOf(b.type);
      return (pa === -1 ? 99 : pa) - (pb === -1 ? 99 : pb);
    });
  const basicTake = Math.max(0, Math.min(basic.length, effectiveMax - special.length));
  let visible: BatteryInsight[] = [...special, ...basic.slice(0, basicTake)].slice(0, effectiveMax);
  // 固定 2 列布局：奇数时截断最后一个，避免单块占行。
  if (visible.length % 2 !== 0) {
    visible = visible.slice(0, visible.length - 1);
  }
  return visible;
}

// ─── 洞察卡片 ───────────────────────────────────────────────────────────────

function BatteryInsightCards({ insights, aiAnalysisEnabled, maxCount }: { insights: BatteryInsight[]; aiAnalysisEnabled: boolean; maxCount?: number }) {
  const { t } = useTranslation();

  // maxCount 由父组件计算（24h 与 10d 两个 range 的可见卡片数取最小），
  // 让切换 range 时卡片数保持一致，避免块的增加/减少造成布局抖动。
  const visible = filterInsightsForCards(insights, maxCount);

  if (visible.length === 0) return null;

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

  return (
    <section className="battery-insight-section">
      <div className="battery-insight-section-head">
        <span className="battery-insight-section-title">
          {t(aiAnalysisEnabled ? 'batteryUsage.insightSectionTitle' : 'batteryUsage.insightSectionTitleBasic')}
        </span>
        <span className="battery-insight-section-hint">
          {t(aiAnalysisEnabled ? 'batteryUsage.insightSectionHint' : 'batteryUsage.insightSectionHintBasic')}
        </span>
      </div>
      <div className="battery-insight-cards">
        {visible.map((insight, i) => (
          <div key={i} className={`battery-insight-card severity-${insight.severity}`}>
            <span className="insight-icon">{iconFor(insight.type)}</span>
            <div className="insight-body">
              <OverflowTip className="insight-title" text={t(`batteryUsage.${insight.title}`)} delay={i * 50} />
              <OverflowTip className="insight-text" text={formatInsightMessage(insight, t)} multiline delay={i * 50 + 30} />
            </div>
          </div>
        ))}
      </div>
    </section>
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
  batteryHistoryEnabled?: boolean;
  aiAnalysisEnabled?: boolean;
  connectedTargets?: readonly BatteryUsageConnectedTarget[];
  preferredDeviceName?: string;
  preferredComponentId?: string;
}

export interface BatteryUsageConnectedTarget {
  deviceName: string;
  componentId: string;
}

function normalizedDeviceName(value: string | undefined): string {
  return value?.trim().replace(/\s+/g, ' ').toLocaleLowerCase() ?? '';
}

function batteryUsageTargetKey(deviceName: string, componentId: string): string {
  return `${normalizedDeviceName(deviceName)}\u0000${componentId}`;
}

export function BatteryUsageModal({
  open,
  onClose,
  hasBattery,
  batteryHistoryEnabled: providedHistoryEnabled,
  aiAnalysisEnabled: providedAiAnalysisEnabled,
  connectedTargets,
  preferredDeviceName,
  preferredComponentId,
}: BatteryUsageModalProps) {
  const { t } = useTranslation();
  const [range, setRange] = useState<BatteryHistoryRange>('24h');
  const [selectedDeviceKey, setSelectedDeviceKey] = useState<string>('');
  // 同时缓存 24h 与 10d 两个 range 的响应：打开时并行拉取，切换 range 命中缓存
  // 即可直接渲染（无须再触发请求），并可用于计算「两个 range 洞察卡片数取最小」
  // 的 maxCount 上限，让切换 range 时卡片数保持一致，避免块增减造成布局抖动。
  const [responses, setResponses] = useState<Partial<Record<BatteryHistoryRange, BatteryHistoryResponse>>>({});
  const response = responses[range] ?? null;
  const [loading, setLoading] = useState(false);
  const [confirmingClear, setConfirmingClear] = useState(false);
  const [loadedHistoryEnabled, setLoadedHistoryEnabled] = useState(true);
  const [loadedAiAnalysisEnabled, setLoadedAiAnalysisEnabled] = useState(false);
  const [reloadNonce, setReloadNonce] = useState(0);
  const pureWeb = isPureWebPreview();
  const historyEnabled = providedHistoryEnabled ?? loadedHistoryEnabled;
  const aiAnalysisEnabled = providedAiAnalysisEnabled ?? loadedAiAnalysisEnabled;

  useEffect(() => {
    if (
      !open
      || pureWeb
      || (providedHistoryEnabled !== undefined && providedAiAnalysisEnabled !== undefined)
    ) return;
    invoke<AppSettings>('settings_get')
      .then((s) => {
        setLoadedHistoryEnabled(s.batteryHistoryEnabled ?? true);
        setLoadedAiAnalysisEnabled(localAiFeatureEnabled(s, LOCAL_AI_FEATURE.batteryUsage));
      })
      .catch(() => { /* 保留默认值 */ });
  }, [open, providedAiAnalysisEnabled, providedHistoryEnabled, pureWeb]);

  // 数据加载：打开时并行拉取 24h + 10d 存入 responses 缓存。切换 range 直接命中缓存，
  // 仅 open/historyEnabled/reloadNonce 变化时重新拉取。Promise.allSettled 容忍单边失败。
  // 两个 range 到齐前保持 loading=true，确保 minInsightCount 计算时已有完整数据，
  // 避免先全量渲染再因 10d 较少而突然裁剪造成抖动。
  useEffect(() => {
    if (!open || !historyEnabled) return;
    let cancelled = false;
    if (pureWeb) {
      queueMicrotask(() => {
        if (cancelled) return;
        setResponses({
          '24h': MOCK_BATTERY_HISTORY_24H,
          '10d': MOCK_BATTERY_HISTORY_10D,
        });
        setLoading(false);
      });
      return () => { cancelled = true; };
    }
    // setLoading(true) 必须同步调用：两个 range 到齐前需要显示 loading 占位，
    // 避免 minInsightCount 未定时洞察卡片先全量渲染再裁剪造成布局抖动。
    // effect 依赖不包含 loading，不会触发级联渲染。
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setLoading(true);
    Promise.allSettled([
      invoke<BatteryHistoryResponse>('battery_history_get', { range: '24h' }),
      invoke<BatteryHistoryResponse>('battery_history_get', { range: '10d' }),
    ]).then(([r24h, r10d]) => {
      if (cancelled) return;
      const next: Partial<Record<BatteryHistoryRange, BatteryHistoryResponse>> = {};
      if (r24h.status === 'fulfilled') next['24h'] = r24h.value;
      else notifyError(t('batteryUsage.title'), String(r24h.reason));
      if (r10d.status === 'fulfilled') next['10d'] = r10d.value;
      else notifyError(t('batteryUsage.title'), String(r10d.reason));
      setResponses(next);
      setLoading(false);
    });
    return () => { cancelled = true; };
  }, [open, historyEnabled, pureWeb, t, reloadNonce]);

  const loadData = useCallback(() => setReloadNonce((n) => n + 1), []);

  // 历史响应继续保留完整数据；设备切换器只投影当前连接快照中的电量组件。
  // 未提供 connectedTargets 时保持组件的独立使用兼容性（例如单元测试和故事页）。
  const selectableDevices = useMemo(() => {
    const historyDevices = response?.devices ?? [];
    if (connectedTargets === undefined) return historyDevices;
    const connectedKeys = new Set(
      connectedTargets.map((target) => batteryUsageTargetKey(target.deviceName, target.componentId)),
    );
    return historyDevices.filter((device) => (
      connectedKeys.has(batteryUsageTargetKey(device.deviceName, device.componentId))
    ));
  }, [connectedTargets, response]);

  // 每次打开时，优先定位 Dashboard 当前鼠标的历史记录；手动切换仍保持优先。
  const preferredDeviceKey = useMemo(() => {
    const deviceName = normalizedDeviceName(preferredDeviceName);
    if (!deviceName) return '';
    const matchingDevices = selectableDevices.filter(
      (device) => normalizedDeviceName(device.deviceName) === deviceName,
    );
    return matchingDevices.find((device) => device.componentId === preferredComponentId)?.key
      ?? matchingDevices[0]?.key
      ?? '';
  }, [selectableDevices, preferredDeviceName, preferredComponentId]);

  // 未显式选择时，定位当前鼠标；没有匹配的旧记录时才回退到第一个。
  // 不让旧范围中的选择键拖垮整个切换器：某台设备在新响应里暂时缺席时，
  // 立即回退到当前鼠标或第一个可用记录。
  const selectedDeviceAvailable = selectableDevices.some((device) => device.key === selectedDeviceKey);
  const effectiveDeviceKey = (selectedDeviceAvailable ? selectedDeviceKey : '')
    || preferredDeviceKey
    || selectableDevices[0]?.key
    || '';

  const selectedDevice = useMemo(
    () => selectableDevices.find((device) => device.key === effectiveDeviceKey),
    [selectableDevices, effectiveDeviceKey],
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

  // 分别计算 24h 与 10d 经设备过滤后的洞察卡片数，取最小作为 BatteryInsightCards
  // 的 maxCount 上限，让切换 range 时卡片数保持一致，从源头避免布局抖动。
  // 仅在两个 range 都到齐时才取最小；只到齐一个时让已到齐的那个自然渲染，避免空帧。
  const selectedInsights24h = useMemo(
    () => (responses['24h']?.insights ?? []).filter(
      (i) => !i.deviceKey || i.deviceKey === effectiveDeviceKey,
    ),
    [responses, effectiveDeviceKey],
  );
  const selectedInsights10d = useMemo(
    () => (responses['10d']?.insights ?? []).filter(
      (i) => !i.deviceKey || i.deviceKey === effectiveDeviceKey,
    ),
    [responses, effectiveDeviceKey],
  );
  const visibleCount24h = filterInsightsForCards(selectedInsights24h).length;
  const visibleCount10d = filterInsightsForCards(selectedInsights10d).length;
  const minInsightCount = responses['24h'] && responses['10d']
    ? Math.min(visibleCount24h, visibleCount10d)
    : undefined;

  // 切换 range 时后端返回新 response 引用但内容常相同。用 useStable 在内容相同时
  // 复用旧引用，让下游 memo（StatusStrip、Summary 子块、InsightCards）浅比较生效，
  // 避免不变块重渲染触发 CSS transition/animation 抖动。
  const stableSelectableDevices = useStable(selectableDevices, deviceArrayEqual);
  const stableSelectedDevice = useStable(selectedDevice, batteryDeviceEqual);
  const stableSelectedInsights = useStable(selectedInsights, insightArrayEqual);

  // 请求下一范围时继续按旧响应自己的范围渲染，避免旧数据短暂套入新坐标系。
  // 新响应到齐后，图表与摘要再作为一个完整状态一次性切换。
  const displayedRange = response?.range ?? range;

  const handleClear = useCallback(async () => {
    if (pureWeb) {
      if (effectiveDeviceKey) {
        // 同步从两个 range 的缓存响应中剔除该设备数据。
        setResponses((current) => {
          const next: Partial<Record<BatteryHistoryRange, BatteryHistoryResponse>> = {};
          for (const key of Object.keys(current) as BatteryHistoryRange[]) {
            const resp = current[key];
            if (!resp) continue;
            next[key] = {
              ...resp,
              devices: resp.devices.filter((d) => d.key !== effectiveDeviceKey),
              series: resp.series.filter((s) => s.key !== effectiveDeviceKey),
              insights: resp.insights.filter((i) => i.deviceKey !== effectiveDeviceKey),
            };
          }
          return next;
        });
      } else {
        setResponses({});
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

  const modalTitle = t('batteryUsage.title');

  if (!historyEnabled) {
    return (
      <Modal
        open={open}
        title={modalTitle}
        size="large"
        className="battery-usage-modal"
        backdropClassName="battery-usage-modal-overlay"
        onClose={onClose}
      >
        <BatteryHistoryDisabledState onClose={onClose} />
      </Modal>
    );
  }

  if (!hasBattery && !pureWeb) {
    return (
      <Modal
        open={open}
        title={modalTitle}
        size="large"
        className="battery-usage-modal"
        backdropClassName="battery-usage-modal-overlay"
        onClose={onClose}
      >
        <BatteryHistoryUnsupportedState onClose={onClose} />
      </Modal>
    );
  }

  return (
    <Modal
      open={open}
      title={modalTitle}
      size="large"
      className="battery-usage-modal"
      backdropClassName="battery-usage-modal-overlay"
      onClose={onClose}
    >
      {/* 布局容器：标题区固定，内容区滚动 */}
      <div className="battery-usage-modal-layout">
        <div className="battery-usage-header">
          <div className="battery-usage-title-wrap">
            <h2>{t('batteryUsage.title')}</h2>
            {aiAnalysisEnabled && (
              <span className="battery-ai-badge">{t('batteryUsage.aiBadgeShort')}</span>
            )}
          </div>
          <button className="battery-usage-close-icon" onClick={onClose} aria-label={t('batteryUsage.close')}>
            <X weight="regular" />
          </button>
        </div>

        <div className="battery-usage-scroll-region">
          {!loading && (!response || selectableDevices.length === 0) ? (
            <BatteryHistoryEmptyState onClose={onClose} />
          ) : (
            <>
              <div className="battery-usage-controls">
                <div
                  className="battery-range-toggle segmented-slider"
                  role="tablist"
                  data-active-index={range === '24h' ? 0 : 1}
                  style={segmentedIndicatorStyle(2, range === '24h' ? 0 : 1)}
                >
                  <button
                    type="button"
                    role="tab"
                    aria-selected={range === '24h'}
                    className={range === '24h' ? 'active' : ''}
                    onClick={() => setRange('24h')}
                  >
                    {t('batteryUsage.range24h')}
                  </button>
                  <button
                    type="button"
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
                device={stableSelectedDevice}
                devices={stableSelectableDevices}
                insights={stableSelectedInsights}
                onSelectDevice={setSelectedDeviceKey}
              />
              <BatteryUsageSummary
                device={stableSelectedDevice}
                insights={stableSelectedInsights}
                range={displayedRange}
              />

              {selectedSeries && (
                <BatteryUsageChart
                  points={selectedSeries.points}
                  range={displayedRange}
                  generatedAt={response?.generatedAt}
                />
              )}

              <BatteryInsightCards
                insights={stableSelectedInsights}
                aiAnalysisEnabled={aiAnalysisEnabled}
                maxCount={minInsightCount}
              />

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
    </Modal>
  );
}
