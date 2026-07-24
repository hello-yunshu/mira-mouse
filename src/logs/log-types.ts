// SPDX-License-Identifier: AGPL-3.0-or-later
/**
 * 前端日志类型镜像。与 Rust 端 `model.rs` 保持语义一致。
 * 仅定义类型与常量，不引入运行时依赖。
 */

export type LogLevel = 'error' | 'warn' | 'info' | 'debug' | 'trace';

export type LogSource = 'app' | 'frontend' | 'plugin' | 'local-ai';

export type FieldValue = string | number | boolean | null;

export type Fields = Record<string, FieldValue>;

export interface LogEntry {
  id: number;
  timestamp: string;
  level: LogLevel;
  source: LogSource;
  target: string;
  message: string;
  sessionId: string;
  correlationId?: string;
  fields?: Fields;
}

export interface LogQuery {
  source?: LogSource;
  minLevel?: LogLevel;
  keyword?: string;
  sessionId?: string;
  from?: string;
  to?: string;
  beforeId?: number;
  limit?: number;
  /** 关联 ID 精确筛选。用于"复制当前设备诊断"按 correlationId 过滤。 */
  correlationId?: string;
  /** target 前缀筛选（区分大小写）。例如 "plugin::" 匹配所有插件协议事件。 */
  targetPrefix?: string;
  /** 结构化字段精确匹配。键为字段名，值为期望的标量值。多键 AND 语义。 */
  fieldsExact?: Record<string, FieldValue>;
}

export interface LogPage {
  entries: LogEntry[];
  hasMore: boolean;
  oldestId: number | null;
  totalInSession: number;
}

export interface DiagnosticSessionStatus {
  startedAt: string;
  endsAt: string;
  originalLevel: LogLevel;
  currentLevel: LogLevel;
  autoExpire: boolean;
}

/** 协议诊断会话状态。授权对指定设备临时记录 HID payload。 */
export interface ProtocolDiagnosticStatus {
  /** 目标设备 key（VID:PID:interface），只对此设备的 HID 交换记录 payload。 */
  deviceKey: string;
  startedAt: string;
  endsAt: string;
  autoExpire: boolean;
}

export interface LogStatus {
  sessionId: string;
  minLevel: LogLevel;
  bufferCount: number;
  bufferCapacity: number;
  storageDirDisplay: string;
  diskUsageBytes: number;
  diskQuotaBytes: number;
  recentErrorCount: number;
  recentWarnCount: number;
  filePersistenceEnabled: boolean;
  diagnosticSession: DiagnosticSessionStatus | null;
  protocolDiagnostic: ProtocolDiagnosticStatus | null;
}

export type DeleteScope =
  | { scope: 'olderThanDays'; days: number }
  | { scope: 'beforeCurrentSession' }
  | { scope: 'all' };

export type ExportScope =
  | { scope: 'filtered'; query: LogQuery }
  | { scope: 'currentSession' }
  | { scope: 'diagnosticsBundle' };

export interface DeleteResult {
  deletedFiles: number;
  deletedBufferEntries: number;
  partial: boolean;
  error: string | null;
}

export interface ExportOutcomeDto {
  path: string;
  entryCount: number;
  bytesWritten: number;
  truncated: boolean;
}

/** 设备定向诊断导出输入（对齐 spec 13.3）。 */
export interface DeviceDiagnosticsInput {
  /** 插件 ID（必填，用于日志筛选）。 */
  pluginId: string;
  /** 设备 key（VID:PID:interface 格式，用于日志筛选）。 */
  deviceKey: string;
  /** 设备 model（可选，用于日志筛选，缩小到特定型号）。 */
  model?: string;
  /** 会话 ID（可选，用于日志筛选，缩小到当前会话）。 */
  sessionId?: string;
  /** 关联 ID（可选，缩小到特定读取会话）。 */
  correlationId?: string;
  /** 当前"全部读数"的 JSON 表示（前端从快照传入）。 */
  readingsJson: string;
  /** 当前 read statuses 的 JSON 表示（前端从快照传入）。 */
  readStatusesJson: string;
  /** 是否包含临时协议诊断（HID payload）。仅在协议诊断模式启用时有效。 */
  includeProtocolPayload?: boolean;
  /** 输出格式："markdown" 或 "json"。默认 "markdown"。 */
  format?: 'markdown' | 'json';
}

/** 设备定向诊断导出结果。 */
export interface DeviceDiagnosticsOutcome {
  path: string;
  bytesWritten: number;
  logEntryCount: number;
  /** 报告内容（用于前端复制到剪贴板）。 */
  content: string;
}

/** 后端向前端推送的实时日志批次事件 payload。 */
export type LogBatchEvent = LogEntry[];

/** 后端向前端推送的实时日志批次事件名。 */
export const LOG_BATCH_EVENT = 'mira://logs/batch';

/** 等级权重，便于前端排序与筛选。与 Rust `LogLevel::weight` 一致：数值越小等级越高。 */
export const LEVEL_WEIGHT: Record<LogLevel, number> = {
  error: 0,
  warn: 1,
  info: 2,
  debug: 3,
  trace: 4,
};

/** 判断 entry.level 是否 ≥ minLevel。 */
export function levelAtLeast(entryLevel: LogLevel, minLevel: LogLevel): boolean {
  return LEVEL_WEIGHT[entryLevel] <= LEVEL_WEIGHT[minLevel];
}

/** 所有可选日志来源（用于筛选 UI）。 */
export const LOG_SOURCES: LogSource[] = ['app', 'frontend', 'plugin', 'local-ai'];

/** 所有可选日志等级（从高到低）。 */
export const LOG_LEVELS: LogLevel[] = ['error', 'warn', 'info', 'debug', 'trace'];
