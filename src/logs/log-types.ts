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

export interface DiagnosticsContext {
  appName: string;
  appVersion: string;
  appIdentifier: string;
  platform: string;
  architecture: string;
  rustVersion: string;
  sessionId: string;
  appInfoJson: string;
  pluginStatusJson: string;
  localAiStatusJson: string;
  recentErrorCount: number;
  recentWarnCount: number;
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
