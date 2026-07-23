// SPDX-License-Identifier: AGPL-3.0-or-later
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  ArrowsClockwise,
  ArrowsInLineVertical,
  ArrowLineDown,
  CaretDown,
  Clipboard,
  Database,
  DotsThree,
  Eraser,
  Export,
  HardDrive,
  MagnifyingGlass,
  Pause,
  Play,
  Trash,
  Wrench,
} from '@phosphor-icons/react';
import { save } from '@tauri-apps/plugin-dialog';
import { Modal, Popover } from '../overlay';
import { Tooltip } from '../Tooltip';
import { getLogClient } from './log-client';
import {
  LOG_LEVELS,
  LEVEL_WEIGHT,
  levelAtLeast,
  type DeleteScope,
  type ExportScope,
  type LogEntry,
  type LogLevel,
  type LogPage as LogPageData,
  type LogQuery,
  type LogSource,
  type LogStatus,
} from './log-types';
import { notifyError, notifySuccess } from '../notify';
import i18n from '../i18n';

/** 是否为纯 Web 预览环境（非 Tauri 运行时）。 */
function isPureWebPreview(): boolean {
  return !(typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window);
}

/** 前端展示用的最大日志条数（窗口化兜底，避免无界增长）。 */
const MAX_VIEW_ENTRIES = 800;
/** 自动跟随到底部的距离阈值（像素）。 */
const FOLLOW_THRESHOLD_PX = 24;
/** 实时批次合并间隔（毫秒），降低高频日志造成的主线程压力。 */
const BATCH_FLUSH_MS = 120;

const PREVIEW_SESSION_ID = 'web-preview-session';

function previewTimestamp(secondsAgo: number): string {
  return new Date(Date.now() - secondsAgo * 1000).toISOString();
}

/**
 * Web 预览直接复用正式日志页，只替换数据源。条目覆盖不同来源、等级、长文本和
 * 结构化字段，便于在浏览器里检查筛选、展开、状态徽章和列表动效。
 */
const PREVIEW_LOG_ENTRIES: LogEntry[] = [
  {
    id: 1012,
    timestamp: previewTimestamp(8),
    level: 'info',
    source: 'app',
    target: 'device::session',
    message: 'device session recovered; live readings restored',
    sessionId: PREVIEW_SESSION_ID,
    correlationId: 'device-7f2a31c4',
    fields: {
      event: 'device-session-recovered',
      device: 'Mira Example Wireless Mouse',
      connection: 'wireless',
      durationMs: 184,
    },
  },
  {
    id: 1011,
    timestamp: previewTimestamp(24),
    level: 'warn',
    source: 'app',
    target: 'device::session',
    message: 'device session interrupted; waiting for recovery',
    sessionId: PREVIEW_SESSION_ID,
    correlationId: 'device-7f2a31c4',
    fields: {
      event: 'device-session-interrupted',
      device: 'Mira Example Wireless Mouse',
      connection: 'wireless',
      errorKind: 'timeout',
    },
  },
  {
    id: 1010,
    timestamp: previewTimestamp(41),
    level: 'info',
    source: 'plugin',
    target: 'plugin::battery',
    message: '电量读取成功：鼠标 82%，接收器 100%',
    sessionId: PREVIEW_SESSION_ID,
    fields: { event: 'plugin-battery-read', pluginId: 'mira.amaster', mouse: 82, receiver: 100 },
  },
  {
    id: 1009,
    timestamp: previewTimestamp(67),
    level: 'info',
    source: 'local-ai',
    target: 'local_ai::predict',
    message: 'prediction batch ok: 1/1 devices returned estimates; 0 used deterministic fallback',
    sessionId: PREVIEW_SESSION_ID,
    correlationId: 'battery-91c4',
    fields: {
      event: 'local-ai-prediction-completed',
      status: 'ok',
      batchCount: 1,
      resultCount: 1,
      fallbackCount: 0,
      durationMs: 38,
      modelVersion: '0.8.3',
    },
  },
  {
    id: 1008,
    timestamp: previewTimestamp(92),
    level: 'error',
    source: 'local-ai',
    target: 'local_ai::runtime',
    message: 'local AI marked failed: response timeout',
    sessionId: PREVIEW_SESSION_ID,
    correlationId: 'battery-8ab1',
    fields: {
      event: 'local-ai-runtime-failed',
      reason: 'response timeout',
      timeoutMs: 2000,
      fallback: true,
      runtimeVersion: '0.8.1',
    },
  },
  {
    id: 1007,
    timestamp: previewTimestamp(126),
    level: 'info',
    source: 'frontend',
    target: 'ui::settings',
    message: '主题设置已保存',
    sessionId: PREVIEW_SESSION_ID,
    fields: { theme: 'dark', source: 'system' },
  },
  {
    id: 1006,
    timestamp: previewTimestamp(158),
    level: 'warn',
    source: 'app',
    target: 'logging::storage',
    message: '日志磁盘用量接近预览阈值，将自动清理最旧记录',
    sessionId: PREVIEW_SESSION_ID,
    fields: { usageBytes: 7340032, quotaBytes: 8388608 },
  },
  {
    id: 1005,
    timestamp: previewTimestamp(201),
    level: 'info',
    source: 'plugin',
    target: 'plugin::verify',
    message: '插件签名和声明式能力契约校验通过',
    sessionId: PREVIEW_SESSION_ID,
    fields: { pluginId: 'mira.amaster', capabilities: 9, signature: 'valid' },
  },
  {
    id: 1004,
    timestamp: previewTimestamp(248),
    level: 'debug',
    source: 'app',
    target: 'device::polling',
    message: '设备轮询完成',
    sessionId: PREVIEW_SESSION_ID,
    fields: { intervalMs: 1000, changedFields: 3 },
  },
  {
    id: 1003,
    timestamp: previewTimestamp(305),
    level: 'trace',
    source: 'frontend',
    target: 'ui::motion',
    message: '控制页共享几何位置已同步',
    sessionId: PREVIEW_SESSION_ID,
    fields: { mode: 'polling-rate', durationMs: 320 },
  },
];

const PREVIEW_LOG_STATUS: LogStatus = {
  sessionId: PREVIEW_SESSION_ID,
  minLevel: 'info',
  bufferCount: PREVIEW_LOG_ENTRIES.length,
  bufferCapacity: 4000,
  storageDirDisplay: 'Web Preview · memory',
  diskUsageBytes: 7340032,
  diskQuotaBytes: 8388608,
  recentErrorCount: 1,
  recentWarnCount: 2,
  filePersistenceEnabled: false,
  diagnosticSession: null,
};

/** 来源筛选选项。 */
type SourceFilter = 'all' | LogSource;

/** 删除确认对话框状态。 */
type DeleteDialogState =
  | { open: false }
  | { open: true; scope: DeleteScope; label: string };

/** 临时诊断会话对话框。 */
type DiagnosticDialogState = {
  open: boolean;
  minutes: number;
  level: LogLevel;
};

/** 格式化本地时间：列表只显示时分秒（与设计稿一致），完整时间保留在 dateTime 属性中。 */
function formatLocalTime(rfc3339: string): string {
  try {
    const date = new Date(rfc3339);
    const pad = (n: number) => String(n).padStart(2, '0');
    return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
  } catch {
    return rfc3339;
  }
}

/** 格式化字节大小。 */
function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/** 等级对应的视觉 token 名（不使用内联颜色，保持主题一致）。 */
function levelClassName(level: LogLevel): string {
  return `log-level log-level-${level}`;
}

/** 来源显示标签。 */
function sourceLabel(source: LogSource): string {
  return i18n.t(`logs.filter.${source === 'local-ai' ? 'localAi' : source}`);
}

const LOG_EVENT_TRANSLATION_KEYS: Record<string, string> = {
  'app-starting': 'logs.events.appStarting',
  'device-session-ready': 'logs.events.deviceSessionReady',
  'device-session-recovered': 'logs.events.deviceSessionRecovered',
  'device-session-interrupted': 'logs.events.deviceSessionInterrupted',
  'device-mutation-attempt': 'logs.events.deviceMutationAttempt',
  'device-mutation-succeeded': 'logs.events.deviceMutationSucceeded',
  'device-mutation-failed': 'logs.events.deviceMutationFailed',
  'device-mutation-timeout': 'logs.events.deviceMutationTimeout',
  'device-mutation-worker-failed': 'logs.events.deviceMutationWorkerFailed',
  'plugins-loaded': 'logs.events.pluginsLoaded',
  'plugin-load-failed': 'logs.events.pluginLoadFailed',
  'plugin-installed-added': 'logs.events.pluginInstalledAdded',
  'plugin-installed-override': 'logs.events.pluginInstalledOverride',
  'plugin-installed-ignored': 'logs.events.pluginInstalledIgnored',
  'local-ai-disabled': 'logs.events.localAiDisabled',
  'local-ai-starting': 'logs.events.localAiStarting',
  'local-ai-unavailable': 'logs.events.localAiUnavailable',
  'local-ai-handshake-failed': 'logs.events.localAiHandshakeFailed',
  'local-ai-ready': 'logs.events.localAiReady',
  'local-ai-stopping': 'logs.events.localAiStopping',
  'local-ai-prediction-completed': 'logs.events.localAiPredictionCompleted',
  'local-ai-prediction-partial': 'logs.events.localAiPredictionPartial',
  'local-ai-baseline-selected': 'logs.events.localAiBaselineSelected',
  'local-ai-handler-interrupted': 'logs.events.localAiHandlerInterrupted',
  'local-ai-handler-failed': 'logs.events.localAiHandlerFailed',
  'local-ai-runtime-failed': 'logs.events.localAiRuntimeFailed',
  'local-ai-stderr-suppressed': 'logs.events.localAiStderrSuppressed',
  'plugin-battery-read': 'logs.events.pluginBatteryRead',
};

const LOG_FIELD_TRANSLATION_KEYS: Record<string, string> = {
  device: 'logs.fieldLabels.device',
  connection: 'logs.fieldLabels.connection',
  durationMs: 'logs.fieldLabels.durationMs',
  errorKind: 'logs.fieldLabels.errorKind',
  errorCode: 'logs.fieldLabels.errorCode',
  reason: 'logs.fieldLabels.reason',
  version: 'logs.fieldLabels.version',
  platform: 'logs.fieldLabels.platform',
  arch: 'logs.fieldLabels.arch',
  pluginId: 'logs.fieldLabels.pluginId',
  pluginVersion: 'logs.fieldLabels.pluginVersion',
  previousVersion: 'logs.fieldLabels.previousVersion',
  pluginCount: 'logs.fieldLabels.pluginCount',
  mutation: 'logs.fieldLabels.mutation',
  paramCount: 'logs.fieldLabels.paramCount',
  runtimeVersion: 'logs.fieldLabels.runtimeVersion',
  modelVersion: 'logs.fieldLabels.modelVersion',
  handlerVersion: 'logs.fieldLabels.handlerVersion',
  handlerApiVersion: 'logs.fieldLabels.handlerApiVersion',
  batchCount: 'logs.fieldLabels.batchCount',
  resultCount: 'logs.fieldLabels.resultCount',
  fallbackCount: 'logs.fieldLabels.fallbackCount',
  status: 'logs.fieldLabels.status',
  enabled: 'logs.fieldLabels.enabled',
  fallback: 'logs.fieldLabels.fallback',
  restart: 'logs.fieldLabels.restart',
  suppressedCount: 'logs.fieldLabels.suppressedCount',
  timeoutMs: 'logs.fieldLabels.timeoutMs',
  mouse: 'logs.fieldLabels.mouse',
  receiver: 'logs.fieldLabels.receiver',
  remainingHours: 'logs.fieldLabels.remainingHours',
  confidence: 'logs.fieldLabels.confidence',
  model: 'logs.fieldLabels.model',
  usageBytes: 'logs.fieldLabels.usageBytes',
  quotaBytes: 'logs.fieldLabels.quotaBytes',
  capabilities: 'logs.fieldLabels.capabilities',
  signature: 'logs.fieldLabels.signature',
  intervalMs: 'logs.fieldLabels.intervalMs',
  changedFields: 'logs.fieldLabels.changedFields',
  theme: 'logs.fieldLabels.theme',
  source: 'logs.fieldLabels.source',
  stage: 'logs.fieldLabels.stage',
  truncated: 'logs.fieldLabels.truncated',
  characterCount: 'logs.fieldLabels.characterCount',
};

/** 后端保留稳定英文消息；界面根据结构化事件键按当前语言展示。 */
function displayLogMessage(entry: LogEntry): string {
  const event = entry.fields?.event;
  if (typeof event === 'string') {
    const translationKey = LOG_EVENT_TRANSLATION_KEYS[event];
    if (translationKey) return i18n.t(translationKey);
  }
  return entry.message;
}

function displayLogFieldValue(key: string, value: unknown): string {
  if (
    key === 'connection'
    && typeof value === 'string'
    && ['usb', 'wireless', 'bluetooth', 'virtual'].includes(value)
  ) {
    return i18n.t(`connection.${value}`);
  }
  if (key === 'status' && typeof value === 'string' && ['ok', 'fallback', 'partial'].includes(value)) {
    return i18n.t(`logs.fieldValues.${value}`);
  }
  if (typeof value === 'boolean') {
    return i18n.t(value ? 'logs.fieldValues.yes' : 'logs.fieldValues.no');
  }
  return String(value);
}

function displayLogFieldLabel(key: string): string {
  const translationKey = LOG_FIELD_TRANSLATION_KEYS[key];
  return translationKey ? i18n.t(translationKey) : key;
}

/** 复制单条日志到剪贴板。 */
async function copyEntryToClipboard(entry: LogEntry): Promise<void> {
  const payload = {
    id: entry.id,
    timestamp: entry.timestamp,
    level: entry.level,
    source: entry.source,
    target: entry.target,
    message: entry.message,
    sessionId: entry.sessionId,
    correlationId: entry.correlationId,
    fields: entry.fields,
  };
  try {
    await navigator.clipboard.writeText(JSON.stringify(payload, null, 2));
  } catch {
    // 静默：剪贴板可能不可用
  }
}

/** 单条日志条目（行式布局，与设计稿一致）。 */
function LogEntryRow({
  entry,
  expanded,
  onToggle,
  onCopy,
  index,
}: {
  entry: LogEntry;
  expanded: boolean;
  onToggle: (entryId: number) => void;
  onCopy: (entry: LogEntry) => void;
  index: number;
}) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    void copyEntryToClipboard(entry).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
      onCopy(entry);
    });
  };

  const fieldEntries = entry.fields
    ? Object.entries(entry.fields).filter(([key]) => key !== 'event')
    : [];

  const staggerDelay = Math.min(index, 10) * 20;
  return (
    <article className={`log-entry${expanded ? ' expanded' : ''}`} data-level={entry.level} style={{ '--entry-delay': `${staggerDelay}ms` } as React.CSSProperties}>
      <button
        type="button"
        className="log-entry-summary"
        aria-expanded={expanded}
        aria-label={t(expanded ? 'logs.list.collapse' : 'logs.list.expand')}
        onClick={() => onToggle(entry.id)}
      >
        <time className="log-entry-time" dateTime={entry.timestamp}>{formatLocalTime(entry.timestamp)}</time>
        <span className={levelClassName(entry.level)}>{entry.level.toUpperCase()}</span>
        <span className="log-entry-source">{sourceLabel(entry.source)}</span>
        <span className="log-entry-message">{displayLogMessage(entry)}</span>
        <CaretDown className="log-entry-caret" weight="bold" aria-hidden="true" />
      </button>
      {expanded && (
        <div className="log-entry-detail">
          <div className="log-entry-detail-card">
            <div className="log-entry-detail-head">
              <dl className="log-entry-fields">
                <div><dt>{t('logs.fields.target')}</dt><dd><code>{entry.target}</code></dd></div>
                <div><dt>{t('logs.fields.sessionId')}</dt><dd><code>{entry.sessionId}</code></dd></div>
                {entry.correlationId && (
                  <div><dt>{t('logs.fields.correlationId')}</dt><dd><code>{entry.correlationId}</code></dd></div>
                )}
              </dl>
              <button type="button" className="log-entry-copy" onClick={handleCopy} aria-label={t('logs.list.copy')}>
                <Clipboard weight="regular" aria-hidden="true" />
                <span>{copied ? t('logs.list.copied') : t('logs.list.copy')}</span>
              </button>
            </div>
            {fieldEntries.length > 0 && (
              <div className="log-entry-structured">
                <div className="log-entry-structured-title">{t('logs.fields.fields')}</div>
                <dl className="log-entry-structured-grid">
                  {fieldEntries.map(([key, value]) => (
                    <div key={key}>
                      <dt title={key}>{displayLogFieldLabel(key)}</dt>
                      <dd><code>{displayLogFieldValue(key, value)}</code></dd>
                    </div>
                  ))}
                </dl>
              </div>
            )}
          </div>
        </div>
      )}
    </article>
  );
}

/** 加载更多按钮 + 状态显示。 */
function LoadMoreFooter({ hasMore, onLoadMore, loading }: { hasMore: boolean; onLoadMore: () => void; loading: boolean }) {
  const { t } = useTranslation();
  if (!hasMore) return null;
  return (
    <div className="log-list-footer">
      <button type="button" className="log-list-more" onClick={onLoadMore} disabled={loading}>
        <span>{loading ? t('logs.list.loading') : t('logs.list.more')}</span>
        <CaretDown weight="bold" aria-hidden="true" />
      </button>
    </div>
  );
}

/** 日志列表：纯展示组件，滚动逻辑（自动跟随 / atTop 检测 / 信号跳转）由 LogPage 统一管理。 */
function LogList({
  entries,
  expandedEntryIds,
  hasMore,
  loading,
  onLoadMore,
  onToggleEntry,
}: {
  entries: LogEntry[];
  expandedEntryIds: ReadonlySet<number>;
  hasMore: boolean;
  loading: boolean;
  onLoadMore: () => void;
  onToggleEntry: (entryId: number) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="log-list-wrapper">
      <div className="log-list">
        {entries.length === 0 ? (
          <p className="log-list-empty">{loading ? t('logs.list.loading') : t('logs.list.empty')}</p>
        ) : (
          entries.map((entry, i) => (
            <LogEntryRow
              key={entry.id}
              entry={entry}
              expanded={expandedEntryIds.has(entry.id)}
              onToggle={onToggleEntry}
              onCopy={() => undefined}
              index={i}
            />
          ))
        )}
        {/* 列表按最新到最旧排列，所以加载更旧记录的入口留在底部。 */}
        <LoadMoreFooter hasMore={hasMore} onLoadMore={onLoadMore} loading={loading} />
      </div>
    </div>
  );
}

/** 工具栏：两行布局 — 统一搜索筛选条 + 控制与状态条。 */
function LogToolbar({
  sourceFilter,
  minLevel,
  keyword,
  follow,
  paused,
  status,
  currentCount,
  newCount,
  expandedCount,
  diagnosticRemainingMinutes,
  onSourceChange,
  onLevelChange,
  onKeywordChange,
  onFollowChange,
  onPauseToggle,
  onClearView,
  onExportFiltered,
  onExportSession,
  onExportBundle,
  onDelete,
  onCopyFiltered,
  copyDisabled,
  onOpenDir,
  onDiagnosticStart,
  onDiagnosticStop,
  onNewCountClick,
  onCollapseAll,
}: {
  sourceFilter: SourceFilter;
  minLevel: LogLevel;
  keyword: string;
  follow: boolean;
  paused: boolean;
  status: LogStatus | null;
  currentCount: number;
  newCount: number;
  expandedCount: number;
  diagnosticRemainingMinutes: number | null;
  onSourceChange: (source: SourceFilter) => void;
  onLevelChange: (level: LogLevel) => void;
  onKeywordChange: (keyword: string) => void;
  onFollowChange: (follow: boolean) => void;
  onPauseToggle: () => void;
  onClearView: () => void;
  onExportFiltered: () => void;
  onExportSession: () => void;
  onExportBundle: () => void;
  onDelete: (scope: DeleteScope, label: string) => void;
  onCopyFiltered: () => void;
  copyDisabled: boolean;
  onOpenDir: () => void;
  onDiagnosticStart: () => void;
  onDiagnosticStop: () => void;
  onNewCountClick: () => void;
  onCollapseAll: () => void;
}) {
  const { t } = useTranslation();
  const [moreOpen, setMoreOpen] = useState(false);
  const moreBtnRef = useRef<HTMLButtonElement>(null);
  const prevLevelRef = useRef<LogLevel | null>(null);

  const levelLabel = (level: LogLevel): string => {
    if (level === 'error') return t('logs.filter.error');
    if (level === 'warn') return t('logs.filter.warn');
    if (level === 'info') return t('logs.filter.info');
    if (level === 'debug') return t('logs.filter.debug');
    return t('logs.filter.trace');
  };

  /** 点击错误/警告徽章切换筛选：已处于该级别时恢复之前的级别，否则记录当前级别并切换。 */
  const toggleLevelFilter = (target: LogLevel) => {
    if (minLevel === target) {
      onLevelChange(prevLevelRef.current ?? 'info');
      prevLevelRef.current = null;
    } else {
      prevLevelRef.current = minLevel;
      onLevelChange(target);
    }
  };

  return (
    <div className="log-toolbar">
      {/* Row 1 — 统一搜索筛选条：来源 pill + 级别 pill + 分隔线 + 搜索框 */}
      <div className="log-toolbar-row log-search-bar">
        <label className="log-filter-pill">
          <span className="log-filter-pill-label">
            {sourceFilter === 'all' ? t('logs.filter.all') : sourceLabel(sourceFilter)}
          </span>
          <select
            value={sourceFilter}
            onChange={(e) => onSourceChange(e.target.value as SourceFilter)}
            aria-label={t('logs.filter.source')}
            tabIndex={-1}
          >
            <option value="all">{t('logs.filter.all')}</option>
            <option value="app">{t('logs.filter.app')}</option>
            <option value="plugin">{t('logs.filter.plugin')}</option>
            <option value="local-ai">{t('logs.filter.localAi')}</option>
          </select>
          <CaretDown className="log-filter-caret" weight="bold" aria-hidden="true" />
        </label>
        <label className="log-filter-pill">
          <span className="log-filter-pill-label" key={minLevel}>
            {levelLabel(minLevel)}
          </span>
          <select
            value={minLevel}
            onChange={(e) => onLevelChange(e.target.value as LogLevel)}
            aria-label={t('logs.filter.level')}
            tabIndex={-1}
          >
            {LOG_LEVELS.map((level) => (
              <option key={level} value={level}>{levelLabel(level)}</option>
            ))}
          </select>
          <CaretDown className="log-filter-caret" weight="bold" aria-hidden="true" />
        </label>
        <span className="log-search-divider" aria-hidden="true" />
        <label className="log-search-input">
          <MagnifyingGlass weight="regular" aria-hidden="true" />
          <input
            type="search"
            placeholder={t('logs.filter.search')}
            value={keyword}
            onChange={(e) => onKeywordChange(e.target.value)}
            aria-label={t('logs.filter.search')}
          />
        </label>
      </div>

      {/* Row 2 — 控制与精简状态：只显示关键数值，完整信息通过悬停查看。 */}
      <div className="log-toolbar-row log-toolbar-control">
        <div className="log-toolbar-group log-toolbar-follow">
          <button
            type="button"
            className={`log-toggle${follow ? ' active' : ''}`}
            aria-pressed={follow}
            onClick={() => onFollowChange(!follow)}
          >
            <ArrowLineDown weight="regular" aria-hidden="true" />
            <span>{t('logs.toolbar.follow')}</span>
          </button>
          <button
            type="button"
            className="log-icon-btn"
            aria-pressed={!paused}
            aria-label={paused ? t('logs.toolbar.resume') : t('logs.toolbar.pause')}
            onClick={onPauseToggle}
          >
            {paused ? <Play weight="regular" aria-hidden="true" /> : <Pause weight="regular" aria-hidden="true" />}
          </button>
        </div>
        {status && (
          <div className="log-toolbar-status" aria-live="polite">
            <Tooltip fitContent label={t('logs.status.bufferCount', { count: status.bufferCount })}>
              <span className="log-status-pill log-status-buffer">
                <Database weight="regular" aria-hidden="true" />
                <span>{status.bufferCount}</span>
              </span>
            </Tooltip>
            <Tooltip fitContent label={t('logs.status.diskUsage', {
              usage: formatBytes(status.diskUsageBytes),
              quota: formatBytes(status.diskQuotaBytes),
            })}>
              <span className="log-status-pill log-status-disk">
                <HardDrive weight="regular" aria-hidden="true" />
                <span>{formatBytes(status.diskUsageBytes)}</span>
              </span>
            </Tooltip>
            {status.recentErrorCount > 0 && (
              <Tooltip
                fitContent
                label={`${t('logs.status.recentErrors', { count: status.recentErrorCount })} · ${minLevel === 'error'
                  ? t('logs.status.restoreFilter')
                  : t('logs.status.filterByLevel', { level: t('logs.filter.error') })}`}
              >
                <button
                  type="button"
                  className={`log-status-count log-status-error btn-enter${minLevel === 'error' ? ' active' : ''}`}
                  aria-label={t('logs.status.recentErrors', { count: status.recentErrorCount })}
                  onClick={() => toggleLevelFilter('error')}
                >
                  {status.recentErrorCount}
                </button>
              </Tooltip>
            )}
            {status.recentWarnCount > 0 && (
              <Tooltip
                fitContent
                label={`${t('logs.status.recentWarns', { count: status.recentWarnCount })} · ${minLevel === 'warn'
                  ? t('logs.status.restoreFilter')
                  : t('logs.status.filterByLevel', { level: t('logs.filter.warn') })}`}
              >
                <button
                  type="button"
                  className={`log-status-count log-status-warn btn-enter${minLevel === 'warn' ? ' active' : ''}`}
                  aria-label={t('logs.status.recentWarns', { count: status.recentWarnCount })}
                  onClick={() => toggleLevelFilter('warn')}
                >
                  {status.recentWarnCount}
                </button>
              </Tooltip>
            )}
          </div>
        )}
        <div className="log-toolbar-group log-toolbar-actions">
          {expandedCount > 0 && (
            <Tooltip fitContent label={t('logs.toolbar.collapseAll', { count: expandedCount })}>
              <button
                type="button"
                className="log-icon-btn log-collapse-all btn-enter"
                aria-label={t('logs.toolbar.collapseAll', { count: expandedCount })}
                onClick={onCollapseAll}
              >
                <ArrowsInLineVertical weight="regular" aria-hidden="true" />
              </button>
            </Tooltip>
          )}
          <div className="log-menu-wrap">
            <button
              type="button"
              ref={moreBtnRef}
              className="log-icon-btn"
              aria-expanded={moreOpen}
              aria-haspopup="menu"
              aria-label={t('logs.toolbar.moreActions')}
              onClick={() => setMoreOpen((v) => !v)}
            >
              <DotsThree weight="bold" aria-hidden="true" />
            </button>
            <Popover
              open={moreOpen}
              onClose={() => setMoreOpen(false)}
              triggerRef={moreBtnRef}
              ariaLabel={t('logs.toolbar.moreActions')}
              className="log-menu log-menu-wide"
            >
              <button type="button" role="menuitem" onClick={() => { setMoreOpen(false); onClearView(); }} disabled={copyDisabled}>
                <Eraser weight="regular" aria-hidden="true" />
                <span>{t('logs.toolbar.clearView')}</span>
              </button>
              <button type="button" role="menuitem" onClick={() => { setMoreOpen(false); onCopyFiltered(); }} disabled={copyDisabled}>
                <Clipboard weight="regular" aria-hidden="true" />
                <span>{t('logs.toolbar.copyFiltered')}</span>
              </button>
              <div className="log-menu-separator" />
              <button type="button" role="menuitem" onClick={() => { setMoreOpen(false); onExportFiltered(); }}>
                <Export weight="regular" aria-hidden="true" />
                <span>{t('logs.toolbar.exportFiltered')}</span>
              </button>
              <button type="button" role="menuitem" onClick={() => { setMoreOpen(false); onExportSession(); }}>
                <Export weight="regular" aria-hidden="true" />
                <span>{t('logs.toolbar.exportSession')}</span>
              </button>
              <button type="button" role="menuitem" onClick={() => { setMoreOpen(false); onExportBundle(); }}>
                <Export weight="regular" aria-hidden="true" />
                <span>{t('logs.toolbar.exportBundle')}</span>
              </button>
              <div className="log-menu-separator" />
              <button type="button" role="menuitem" onClick={() => { setMoreOpen(false); onDelete({ scope: 'olderThanDays', days: 7 }, t('logs.delete.olderThanDays')); }}>
                <Trash weight="regular" aria-hidden="true" />
                <span>{t('logs.toolbar.deleteOlder')}</span>
              </button>
              <button type="button" role="menuitem" onClick={() => { setMoreOpen(false); onDelete({ scope: 'beforeCurrentSession' }, t('logs.delete.beforeCurrentSession')); }}>
                <Trash weight="regular" aria-hidden="true" />
                <span>{t('logs.toolbar.deleteBeforeSession')}</span>
              </button>
              <button type="button" role="menuitem" onClick={() => { setMoreOpen(false); onDelete({ scope: 'all' }, t('logs.delete.all')); }}>
                <Trash weight="regular" aria-hidden="true" />
                <span>{t('logs.delete.all')}</span>
              </button>
              <div className="log-menu-separator" />
              <button type="button" role="menuitem" onClick={() => { setMoreOpen(false); onOpenDir(); }}>
                <ArrowsClockwise weight="regular" aria-hidden="true" />
                <span>{t('logs.toolbar.openDir')}</span>
              </button>
              {diagnosticRemainingMinutes !== null ? (
                <button type="button" role="menuitem" onClick={() => { setMoreOpen(false); onDiagnosticStop(); }}>
                  <Wrench weight="regular" aria-hidden="true" />
                  <span>{t('logs.toolbar.diagnosticActive', { minutes: diagnosticRemainingMinutes })}</span>
                </button>
              ) : (
                <button type="button" role="menuitem" onClick={() => { setMoreOpen(false); onDiagnosticStart(); }}>
                  <Wrench weight="regular" aria-hidden="true" />
                  <span>{t('logs.toolbar.diagnosticStart')}</span>
                </button>
              )}
            </Popover>
          </div>
        </div>
      </div>
      <div
        className="log-toolbar-divider"
        aria-label={follow && newCount > 0 ? undefined : t('logs.status.currentCount', { count: currentCount })}
      >
        {follow && newCount > 0 ? (
          <button type="button" className="log-toolbar-new-count" onClick={onNewCountClick}>
            <span className="log-new-count-dot" aria-hidden="true" />
            <span>{t('logs.list.newCount', { count: newCount })}</span>
          </button>
        ) : (
          <span className="log-toolbar-current-count">{t('logs.status.currentCount', { count: currentCount })}</span>
        )}
      </div>
    </div>
  );
}

/** 删除确认对话框。 */
function DeleteConfirmDialog({ state, onClose, onConfirm }: {
  state: DeleteDialogState;
  onClose: () => void;
  onConfirm: (scope: DeleteScope) => Promise<void>;
}) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  if (!state.open) return null;
  const handleConfirm = async () => {
    setBusy(true);
    try { await onConfirm(state.scope); } finally { setBusy(false); }
  };
  return (
    <Modal open={true} title={t('logs.delete.confirmTitle')} size="small"
      className="edit-modal log-confirm-dialog" backdropClassName="edit-modal-backdrop" onClose={onClose}>
      <header><h3>{t('logs.delete.confirmTitle')}</h3></header>
      <div className="edit-modal-body">
        <p className="setting-hint">{t('logs.delete.confirmHint')}</p>
        <p className="setting-hint"><strong>{state.label}</strong></p>
      </div>
      <footer>
        <button type="button" className="secondary" onClick={onClose} disabled={busy}>{t('logs.delete.cancel')}</button>
        <button type="button" onClick={handleConfirm} disabled={busy}>{t('logs.delete.confirm')}</button>
      </footer>
    </Modal>
  );
}

/** 临时诊断会话对话框。 */
function DiagnosticStartDialog({ state, onClose, onConfirm }: {
  state: DiagnosticDialogState;
  onClose: () => void;
  onConfirm: (minutes: number, level: LogLevel) => Promise<void>;
}) {
  const { t } = useTranslation();
  const [minutes, setMinutes] = useState(state.minutes);
  const [level, setLevel] = useState<LogLevel>(state.level);
  const [busy, setBusy] = useState(false);
  if (!state.open) return null;
  const handleConfirm = async () => {
    setBusy(true);
    try { await onConfirm(minutes, level); } finally { setBusy(false); }
  };
  return (
    <Modal open={true} title={t('logs.diagnostic.startTitle')} size="small"
      className="edit-modal" backdropClassName="edit-modal-backdrop" onClose={onClose}>
      <header><h3>{t('logs.diagnostic.startTitle')}</h3></header>
      <div className="edit-modal-body">
        <p className="setting-hint">{t('logs.diagnostic.startHint')}</p>
        <label className="edit-field">
          <span>{t('logs.diagnostic.durationLabel')}</span>
          <input type="number" min={1} max={30} value={minutes}
            onChange={(e) => setMinutes(Math.max(1, Math.min(30, Number(e.target.value) || 10)))} />
        </label>
        <label className="edit-field">
          <span>{t('logs.diagnostic.levelLabel')}</span>
          <select value={level} onChange={(e) => setLevel(e.target.value as LogLevel)}>
            {LOG_LEVELS.filter((l) => LEVEL_WEIGHT[l] >= LEVEL_WEIGHT.debug).map((l) => (
              <option key={l} value={l}>{l}</option>
            ))}
          </select>
        </label>
      </div>
      <footer>
        <button type="button" className="secondary" onClick={onClose} disabled={busy}>{t('logs.diagnostic.cancel')}</button>
        <button type="button" onClick={handleConfirm} disabled={busy}>{t('logs.diagnostic.start')}</button>
      </footer>
    </Modal>
  );
}

/** 当前日期（用于默认文件名）。 */
function currentDateStamp(): string {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}`;
}

/** 构建 LogQuery。 */
function buildQuery(sourceFilter: SourceFilter, minLevel: LogLevel, keyword: string, beforeId?: number): LogQuery {
  const query: LogQuery = {
    minLevel,
    limit: 200,
  };
  if (sourceFilter !== 'all') query.source = sourceFilter;
  const trimmed = keyword.trim();
  if (trimmed) query.keyword = trimmed;
  if (beforeId !== undefined) query.beforeId = beforeId;
  return query;
}

/** 前端二次筛选：实时事件按当前 filter 立即过滤，避免等到刷新。 */
function entryMatchesFilter(entry: LogEntry, sourceFilter: SourceFilter, minLevel: LogLevel, keyword: string): boolean {
  if (sourceFilter !== 'all') {
    if (sourceFilter === 'app') {
      if (entry.source !== 'app' && entry.source !== 'frontend') return false;
    } else if (entry.source !== sourceFilter) return false;
  }
  if (!levelAtLeast(entry.level, minLevel)) return false;
  const trimmed = keyword.trim().toLowerCase();
  if (!trimmed) return true;
  return displayLogMessage(entry).toLowerCase().includes(trimmed)
    || entry.message.toLowerCase().includes(trimmed)
    || entry.target.toLowerCase().includes(trimmed);
}

function queryPreviewLogs(query: LogQuery): LogPageData {
  const sourceFilter = query.source ?? 'all';
  const minLevel = query.minLevel ?? 'trace';
  const keyword = query.keyword ?? '';
  const limit = query.limit ?? 200;
  const filtered = PREVIEW_LOG_ENTRIES.filter((entry) => (
    entryMatchesFilter(entry, sourceFilter, minLevel, keyword)
    && (query.beforeId === undefined || entry.id < query.beforeId)
  ));
  const pageEntries = filtered.slice(0, limit);
  return {
    entries: pageEntries,
    hasMore: filtered.length > pageEntries.length,
    oldestId: pageEntries.at(-1)?.id ?? null,
    totalInSession: filtered.length,
  };
}

/** 主日志页。 */
export function LogPage({ onBack }: { onBack: () => void }) {
  const { t } = useTranslation();
  const pureWeb = isPureWebPreview();

  const [entries, setEntries] = useState<LogEntry[]>(() => (
    pureWeb ? queryPreviewLogs({ minLevel: 'info', limit: 200 }).entries : []
  ));
  const [hasMore, setHasMore] = useState(() => (
    pureWeb ? queryPreviewLogs({ minLevel: 'info', limit: 200 }).hasMore : false
  ));
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string>('');

  const [sourceFilter, setSourceFilter] = useState<SourceFilter>('all');
  const [minLevel, setMinLevel] = useState<LogLevel>('info');
  const [keyword, setKeyword] = useState('');
  const [listKey, setListKey] = useState(0);
  const [previewCleared, setPreviewCleared] = useState(false);
  const [expandedEntryIds, setExpandedEntryIds] = useState<Set<number>>(() => new Set());

  const [follow, setFollow] = useState(true);
  const [paused, setPaused] = useState(false);
  const [newCount, setNewCount] = useState(0);
  /** 强制滚动到顶部的信号（跳转最新 / 恢复暂停时自增）。 */
  const [scrollSignal, setScrollSignal] = useState(0);
  /** 用户是否停留在页面顶部（最新日志所在位置）。 */
  const [atTop, setAtTop] = useState(true);
  const atTopRef = useRef(true);
  useEffect(() => { atTopRef.current = atTop; }, [atTop]);

  /** 页面级滚动容器 ref（挂在 <main> 上）。滚动跟随 / atTop 检测均基于此 ref。 */
  const scrollRef = useRef<HTMLElement>(null);

  /** 检测当前是否在顶部附近。 */
  const checkAtTop = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return true;
    return el.scrollTop <= FOLLOW_THRESHOLD_PX;
  }, []);

  /** 滚动到最新日志所在的顶部。 */
  const scrollToTop = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = 0;
  }, []);

  // 自动跟随：新日志到达时若用户在顶部且未暂停 + follow 开启，保持在顶部。
  useEffect(() => {
    if (paused || !follow) return;
    if (!atTop) return;
    scrollToTop();
  }, [entries.length, paused, follow, atTop, scrollToTop]);

  // 强制滚动信号：跳转最新 / 恢复暂停时滚到顶部。
  useEffect(() => {
    if (scrollSignal === 0) return;
    scrollToTop();
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setAtTop(true);
  }, [scrollSignal, scrollToTop]);

  const handleScroll = useCallback(() => {
    const next = checkAtTop();
    setAtTop(next);
  }, [checkAtTop]);

  useEffect(() => {
    scrollToTop();
    // 初始挂载后将状态同步为「已在顶部」。这是同步滚动后的副作用，不会造成级联渲染。
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setAtTop(true);
  }, [scrollToTop]);

  const [status, setStatus] = useState<LogStatus | null>(() => (pureWeb ? PREVIEW_LOG_STATUS : null));
  const [oldestId, setOldestId] = useState<number | null>(() => (
    pureWeb ? queryPreviewLogs({ minLevel: 'info', limit: 200 }).oldestId : null
  ));
  const [deleteDialog, setDeleteDialog] = useState<DeleteDialogState>({ open: false });
  const [diagnosticDialog, setDiagnosticDialog] = useState<DiagnosticDialogState>({ open: false, minutes: 10, level: 'debug' });
  const [diagnosticRemaining, setDiagnosticRemaining] = useState<number | null>(null);

  // 实时事件缓冲：在暂停期间累计，恢复时合并。
  const pendingBatchRef = useRef<LogEntry[]>([]);
  // 防抖：高频事件合并到单次 React 更新。
  const flushTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const client = getLogClient();
  const clientRef = useRef(client);

  const previewPage = pureWeb
    ? queryPreviewLogs(buildQuery(sourceFilter, minLevel, keyword))
    : null;
  const displayedEntries = pureWeb
    ? (previewCleared ? [] : previewPage?.entries ?? [])
    : entries;
  const displayedHasMore = pureWeb ? (previewPage?.hasMore ?? false) : hasMore;
  const displayedStatus = pureWeb ? PREVIEW_LOG_STATUS : status;
  const expandedCount = expandedEntryIds.size;

  const toggleExpandedEntry = useCallback((entryId: number) => {
    setExpandedEntryIds((current) => {
      const next = new Set(current);
      if (next.has(entryId)) next.delete(entryId);
      else next.add(entryId);
      return next;
    });
  }, []);

  const collapseAllEntries = useCallback(() => {
    setExpandedEntryIds(new Set());
  }, []);

  // 同步最新值到 ref，供事件回调读取。在 effect 中更新 ref 是 React 19 推荐方式。
  const sourceFilterRef = useRef(sourceFilter);
  const minLevelRef = useRef(minLevel);
  const keywordRef = useRef(keyword);
  const pausedRef = useRef(paused);
  useEffect(() => {
    sourceFilterRef.current = sourceFilter;
    minLevelRef.current = minLevel;
    keywordRef.current = keyword;
    pausedRef.current = paused;
  }, [sourceFilter, minLevel, keyword, paused]);

  /** 刷新状态。 */
  const refreshStatus = useCallback(async () => {
    try {
      const next = await clientRef.current.status();
      setStatus(next);
      if (next.diagnosticSession) {
        const endsAtMs = Date.parse(next.diagnosticSession.endsAt);
        if (!Number.isNaN(endsAtMs)) {
          const remaining = Math.max(0, Math.ceil((endsAtMs - Date.now()) / 60000));
          setDiagnosticRemaining(remaining);
        }
      } else {
        setDiagnosticRemaining(null);
      }
    } catch {
      // 静默
    }
  }, []);

  /** 初始加载：查询历史日志 + 状态。 */
  useEffect(() => {
    if (pureWeb) return;
    let cancelled = false;
    const initialQuery = buildQuery(sourceFilter, minLevel, keyword);
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setLoading(true);
    clientRef.current.query(initialQuery)
      .then((page: LogPageData) => {
        if (cancelled) return;
        // 后端已返回最新→最旧，直接保留这一顺序。
        setEntries(page.entries.slice(0, MAX_VIEW_ENTRIES));
        setHasMore(page.hasMore);
        setOldestId(page.oldestId);
        setError('');
      })
      .catch((err) => {
        if (cancelled) return;
        setError(String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    void refreshStatus();
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** 订阅实时日志批次。 */
  useEffect(() => {
    if (pureWeb) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    const flush = () => {
      flushTimerRef.current = null;
      const batch = pendingBatchRef.current;
      pendingBatchRef.current = [];
      if (batch.length === 0) return;
      setEntries((prev) => {
        // emitter 批次按产生时间（旧→新）到达；反转后前插，保持最新在上。
        return [...batch.slice().reverse(), ...prev].slice(0, MAX_VIEW_ENTRIES);
      });
      void refreshStatus();
    };

    const handleBatch = (batch: LogEntry[]) => {
      if (cancelled) return;
      const sf = sourceFilterRef.current;
      const ml = minLevelRef.current;
      const kw = keywordRef.current;
      const filtered = batch.filter((e) => entryMatchesFilter(e, sf, ml, kw));
      if (filtered.length === 0) return;

      if (pausedRef.current) {
        // 暂停期间：累计到 pendingBatchRef，恢复或跳转最新时合并；同时更新计数。
        pendingBatchRef.current.push(...filtered);
        // 防止长时间暂停导致无界增长：只保留最近一个展示窗口的条目。
        if (pendingBatchRef.current.length > MAX_VIEW_ENTRIES) {
          pendingBatchRef.current = pendingBatchRef.current.slice(-MAX_VIEW_ENTRIES);
        }
        setNewCount((n) => n + filtered.length);
        return;
      }

      // 非暂停：日志进入列表（flush），但若用户不在顶部则计数提示
      pendingBatchRef.current.push(...filtered);
      if (flushTimerRef.current === null) {
        flushTimerRef.current = setTimeout(flush, BATCH_FLUSH_MS);
      }
      if (!atTopRef.current) {
        // 用户在阅读历史，不强制滚动，显示徽章
        setNewCount((n) => n + filtered.length);
      }
    };

    void clientRef.current.subscribe(handleBatch).then((un) => {
      if (cancelled) {
        un();
      } else {
        unlisten = un;
      }
    });

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
      if (flushTimerRef.current) {
        clearTimeout(flushTimerRef.current);
        flushTimerRef.current = null;
      }
      pendingBatchRef.current = [];
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** 用当前筛选条件重新查询并刷新列表（供删除等操作后调用）。 */
  const refreshEntries = useCallback(async () => {
    if (pureWeb) return;
    const query = buildQuery(sourceFilterRef.current, minLevelRef.current, keywordRef.current);
    setLoading(true);
    try {
      const page = await clientRef.current.query(query);
      setEntries(page.entries.slice(0, MAX_VIEW_ENTRIES));
      setHasMore(page.hasMore);
      setOldestId(page.oldestId);
      setNewCount(0);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, [pureWeb]);

  /** 筛选条件变化时重新查询。 */
  useEffect(() => {
    if (pureWeb) return;
    let cancelled = false;
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setLoading(true);
    setListKey((k) => k + 1);
    const query = buildQuery(sourceFilter, minLevel, keyword);
    clientRef.current.query(query)
      .then((page) => {
        if (cancelled) return;
        setEntries(page.entries.slice(0, MAX_VIEW_ENTRIES));
        setHasMore(page.hasMore);
        setOldestId(page.oldestId);
        setNewCount(0);
      })
      .catch((err) => {
        if (cancelled) return;
        setError(String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sourceFilter, minLevel]);

  useEffect(() => {
    if (pureWeb) return;
    // cancelled 提升到 effect 作用域，避免快速输入时用过期结果覆盖最新结果。
    let cancelled = false;
    const timer = setTimeout(() => {
      const query = buildQuery(sourceFilter, minLevel, keyword);
      clientRef.current.query(query)
        .then((page) => {
          if (cancelled) return;
          setEntries(page.entries.slice(0, MAX_VIEW_ENTRIES));
          setHasMore(page.hasMore);
          setOldestId(page.oldestId);
          setNewCount(0);
        })
        .catch(() => { /* 静默 */ });
    }, 250);
    return () => { cancelled = true; clearTimeout(timer); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [keyword]);

  /** 加载更多（向前翻页，加载更旧的条目）。 */
  const loadMore = useCallback(async () => {
    if (oldestId === null || loading) return;
    setLoading(true);
    try {
      const query = buildQuery(sourceFilter, minLevel, keyword, oldestId);
      const page = await clientRef.current.query(query);
      // page.entries 为更旧一页的最新→最旧；追加到底部，保持最新在上。
      setEntries((prev) => [...prev, ...page.entries].slice(0, MAX_VIEW_ENTRIES));
      setHasMore(page.hasMore);
      setOldestId(page.oldestId);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, [oldestId, loading, sourceFilter, minLevel, keyword]);

  /** 跳到最新：合并暂停期间累计的日志并强制滚动到顶部。 */
  const jumpToNewest = useCallback(() => {
    if (pendingBatchRef.current.length > 0) {
      const pending = pendingBatchRef.current;
      pendingBatchRef.current = [];
      setEntries((prev) => {
        return [...pending.slice().reverse(), ...prev].slice(0, MAX_VIEW_ENTRIES);
      });
    }
    setNewCount(0);
    setScrollSignal((n) => n + 1);
    // 点击徽章表示用户想看新日志：若处于暂停则恢复自动刷新
    if (pausedRef.current) {
      setPaused(false);
    }
  }, []);

  /** 暂停切换。 */
  const pauseToggle = useCallback(() => {
    // 读取当前暂停状态决定是否恢复；副作用（合并日志）放在 updater 之外，
    // 避免在 StrictMode 双调用 updater 时重复触发副作用。
    const resuming = pausedRef.current;
    setPaused((p) => !p);
    if (resuming) {
      jumpToNewest();
    } else if (pureWeb) {
      // Web 预览没有真实日志事件；暂停时模拟一小批新日志，便于直接检查提示样式。
      setNewCount(3);
    }
  }, [jumpToNewest, pureWeb]);

  /** 清空当前视图：仅清前端显示，不调用删除命令。 */
  const clearView = useCallback(() => {
    setEntries([]);
    setPreviewCleared(true);
    setNewCount(0);
    setExpandedEntryIds(new Set());
    pendingBatchRef.current = [];
  }, []);

  /** 导出当前筛选结果。 */
  const exportFiltered = useCallback(async () => {
    if (pureWeb) {
      notifySuccess(t('logs.previewAction'));
      return;
    }
    try {
      const date = currentDateStamp();
      const path = await save({
        defaultPath: t('logs.export.defaultName', { date }),
        filters: [{ name: t('logs.export.jsonlFilter'), extensions: ['jsonl'] }],
      });
      if (!path) {
        notifySuccess(t('logs.export.cancelled'));
        return;
      }
      const query = buildQuery(sourceFilter, minLevel, keyword);
      const scope: ExportScope = { scope: 'filtered', query };
      const outcome = await clientRef.current.exportLogs(scope, path);
      notifySuccess(t('logs.export.success', {
        count: outcome.entryCount,
        bytes: formatBytes(outcome.bytesWritten),
        path: outcome.path,
      }) + (outcome.truncated ? t('logs.export.truncated') : ''));
    } catch (err) {
      notifyError(t('notification.exportFailed'), t('logs.export.failed', { error: String(err) }));
    }
  }, [pureWeb, sourceFilter, minLevel, keyword, t]);

  /** 导出当前会话。 */
  const exportSession = useCallback(async () => {
    if (pureWeb) {
      notifySuccess(t('logs.previewAction'));
      return;
    }
    try {
      const date = currentDateStamp();
      const path = await save({
        defaultPath: t('logs.export.defaultName', { date }),
        filters: [{ name: t('logs.export.jsonlFilter'), extensions: ['jsonl'] }],
      });
      if (!path) return;
      const scope: ExportScope = { scope: 'currentSession' };
      const outcome = await clientRef.current.exportLogs(scope, path);
      notifySuccess(t('logs.export.success', {
        count: outcome.entryCount,
        bytes: formatBytes(outcome.bytesWritten),
        path: outcome.path,
      }));
    } catch (err) {
      notifyError(t('notification.exportFailed'), t('logs.export.failed', { error: String(err) }));
    }
  }, [pureWeb, t]);

  /** 导出诊断包。诊断上下文（版本/平台/架构/本地 AI 状态等）由后端收集。 */
  const exportBundle = useCallback(async () => {
    if (pureWeb) {
      notifySuccess(t('logs.previewAction'));
      return;
    }
    try {
      const date = currentDateStamp();
      const path = await save({
        defaultPath: t('logs.export.bundleName', { date }),
        filters: [{ name: t('logs.export.zipFilter'), extensions: ['zip'] }],
      });
      if (!path) return;
      const outcome = await clientRef.current.exportDiagnosticsBundle(path);
      notifySuccess(
        t('logs.export.bundleSuccess', { path: outcome.path })
        + (outcome.truncated ? t('logs.export.truncated') : ''),
      );
    } catch (err) {
      notifyError(t('notification.exportFailed'), t('logs.export.failed', { error: String(err) }));
    }
  }, [pureWeb, t]);

  /** 复制当前筛选结果到剪贴板（JSONL）。 */
  const copyFiltered = async () => {
    if (displayedEntries.length === 0) return;
    const jsonl = displayedEntries.map((e) => JSON.stringify({
      id: e.id, timestamp: e.timestamp, level: e.level, source: e.source,
      target: e.target, message: e.message, sessionId: e.sessionId,
      correlationId: e.correlationId, fields: e.fields,
    })).join('\n');
    try {
      await navigator.clipboard.writeText(jsonl);
      notifySuccess(t('logs.copy.success', { count: displayedEntries.length }));
    } catch {
      notifyError(t('logs.copy.failed'));
    }
  };

  /** 打开删除确认。 */
  const openDelete = useCallback((scope: DeleteScope, label: string) => {
    setDeleteDialog({ open: true, scope, label });
  }, []);

  /** 确认删除。 */
  const confirmDelete = useCallback(async (scope: DeleteScope) => {
    if (pureWeb) {
      setPreviewCleared(true);
      setDeleteDialog({ open: false });
      notifySuccess(t('logs.previewAction'));
      return;
    }
    try {
      const result = await clientRef.current.delete(scope);
      if (result.partial && result.error) {
        notifyError(t('logs.delete.failed'), t('logs.delete.partial', { error: result.error }));
      } else if (result.error) {
        notifyError(t('logs.delete.failed'), t('logs.delete.failed', { error: result.error }));
      } else {
        notifySuccess(t('logs.delete.success', {
          files: result.deletedFiles,
          entries: result.deletedBufferEntries,
        }));
      }
      setDeleteDialog({ open: false });
      void refreshStatus();
      void refreshEntries();
    } catch (err) {
      notifyError(t('logs.delete.failed'), String(err));
    }
  }, [pureWeb, refreshStatus, refreshEntries, t]);

  /** 打开日志目录。 */
  const openDir = useCallback(async () => {
    if (pureWeb) {
      notifySuccess(t('logs.previewAction'));
      return;
    }
    try {
      await clientRef.current.openLogDir();
    } catch (err) {
      notifyError(t('logs.openDirFailed'), String(err));
    }
  }, [pureWeb, t]);

  /** 开始临时诊断会话。 */
  const startDiagnostic = useCallback(() => {
    setDiagnosticDialog({ open: true, minutes: 10, level: 'debug' });
  }, []);

  /** 确认开始诊断。 */
  const confirmDiagnostic = useCallback(async (minutes: number, level: LogLevel) => {
    if (pureWeb) {
      setDiagnosticRemaining(minutes);
      setDiagnosticDialog((s) => ({ ...s, open: false }));
      notifySuccess(t('logs.previewAction'));
      return;
    }
    try {
      await clientRef.current.startDiagnosticSession(minutes, level, true);
      notifySuccess(t('logs.diagnostic.started'));
      setDiagnosticDialog((s) => ({ ...s, open: false }));
      void refreshStatus();
    } catch (err) {
      notifyError(t('logs.diagnostic.startFailed'), String(err));
    }
  }, [pureWeb, refreshStatus, t]);

  /** 停止诊断。 */
  const stopDiagnostic = useCallback(async () => {
    if (pureWeb) {
      setDiagnosticRemaining(null);
      notifySuccess(t('logs.previewAction'));
      return;
    }
    try {
      await clientRef.current.stopDiagnosticSession();
      notifySuccess(t('logs.diagnostic.stopped'));
      setDiagnosticRemaining(null);
      void refreshStatus();
    } catch (err) {
      notifyError(t('logs.diagnostic.stopFailed'), String(err));
    }
  }, [pureWeb, refreshStatus, t]);

  /** 周期性刷新诊断剩余时间。 */
  useEffect(() => {
    if (diagnosticRemaining === null) return;
    const timer = setInterval(() => {
      void refreshStatus();
    }, 30000);
    return () => clearInterval(timer);
  }, [diagnosticRemaining, refreshStatus]);

  return (
    <main className="log-page" ref={scrollRef} onScroll={handleScroll}>
      <header>
        <div>
          <p className="eyebrow">{t('logs.eyebrow')}</p>
          <h1>{t('logs.title')}</h1>
        </div>
        <button className="secondary" onClick={onBack}>{t('common.back')}</button>
      </header>
      {pureWeb && <p className="setting-hint log-preview-hint">{t('logs.previewEmpty')}</p>}
      {error && <p className="setting-hint">{t('logs.loadFailed', { error })}</p>}
      <LogToolbar
        sourceFilter={sourceFilter}
        minLevel={minLevel}
        keyword={keyword}
        follow={follow}
        paused={paused}
        status={displayedStatus}
        currentCount={displayedEntries.length}
        newCount={newCount}
        expandedCount={expandedCount}
        diagnosticRemainingMinutes={diagnosticRemaining}
        onSourceChange={(source) => { collapseAllEntries(); setPreviewCleared(false); setSourceFilter(source); }}
        onLevelChange={(level) => { collapseAllEntries(); setPreviewCleared(false); setMinLevel(level); }}
        onKeywordChange={(value) => { collapseAllEntries(); setPreviewCleared(false); setKeyword(value); }}
        onFollowChange={setFollow}
        onPauseToggle={pauseToggle}
        onClearView={clearView}
        onExportFiltered={exportFiltered}
        onExportSession={exportSession}
        onExportBundle={exportBundle}
        onDelete={openDelete}
        onCopyFiltered={copyFiltered}
        copyDisabled={displayedEntries.length === 0}
        onOpenDir={openDir}
        onDiagnosticStart={startDiagnostic}
        onDiagnosticStop={stopDiagnostic}
        onNewCountClick={jumpToNewest}
        onCollapseAll={collapseAllEntries}
      />
      <LogList
        key={listKey}
        entries={displayedEntries}
        expandedEntryIds={expandedEntryIds}
        hasMore={displayedHasMore}
        loading={loading}
        onLoadMore={loadMore}
        onToggleEntry={toggleExpandedEntry}
      />
      <DeleteConfirmDialog state={deleteDialog} onClose={() => setDeleteDialog({ open: false })} onConfirm={confirmDelete} />
      <DiagnosticStartDialog
        state={diagnosticDialog}
        onClose={() => setDiagnosticDialog((s) => ({ ...s, open: false }))}
        onConfirm={confirmDiagnostic}
      />
    </main>
  );
}
