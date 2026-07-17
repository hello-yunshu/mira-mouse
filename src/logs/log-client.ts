// SPDX-License-Identifier: AGPL-3.0-or-later
/**
 * 日志后端客户端。封装 Tauri invoke 调用与实时事件订阅。
 *
 * 设计约束（对齐 spec 四.4 / 五.2）：
 * - 单例：整个应用共享一个 LogClient 实例。
 * - 失败不递归：invoke 失败只走 console.error，不调用 log_write。
 * - 实时事件：通过 `listen` 订阅 `mira://logs/batch`，组件卸载时 unlisten。
 * - 防抖：前端日志写入受内部节流，避免高频递归。
 * - 测试友好：所有 invoke 调用可通过依赖注入替换。
 */

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  DeleteResult,
  DeleteScope,
  ExportOutcomeDto,
  ExportScope,
  LogBatchEvent,
  LogPage,
  LogQuery,
  LogStatus,
} from './log-types';
import { LOG_BATCH_EVENT } from './log-types';

/** 前端日志写入节流间隔（毫秒）。 */
const FRONTEND_LOG_THROTTLE_MS = 200;
/** 前端单次批量写入最大条数。 */
const FRONTEND_LOG_BATCH_MAX = 10;

/**
 * 日志客户端。单例模式，通过 `getLogClient()` 获取。
 * 在 Web preview 或测试环境中，invoke 会失败，客户端静默降级。
 */
export class LogClient {
  private batchListener: UnlistenFn | null = null;
  private batchHandler: ((batch: LogBatchEvent) => void) | null = null;
  private frontendLogBuffer: Parameters<typeof invoke>[1][] = [];
  private frontendLogTimer: ReturnType<typeof setTimeout> | null = null;
  private subscribed = false;

  /** 查询历史日志。 */
  async query(query: LogQuery): Promise<LogPage> {
    return invoke<LogPage>('log_query', { query });
  }

  /** 获取日志服务状态。 */
  async status(): Promise<LogStatus> {
    return invoke<LogStatus>('log_status');
  }

  /** 删除磁盘历史日志。 */
  async delete(scope: DeleteScope): Promise<DeleteResult> {
    return invoke<DeleteResult>('log_delete', { scope });
  }

  /** 前端订阅实时日志批次。返回取消订阅函数。 */
  async subscribe(handler: (batch: LogBatchEvent) => void): Promise<() => void> {
    // 如果已有订阅，先取消旧的。
    if (this.batchListener) {
      this.unsubscribe();
    }
    this.batchHandler = handler;
    this.subscribed = true;
    try {
      await invoke('log_subscribe');
      this.batchListener = await listen<LogBatchEvent>(LOG_BATCH_EVENT, (event) => {
        if (this.batchHandler) {
          this.batchHandler(event.payload as LogBatchEvent);
        }
      });
    } catch {
      // Tauri 未就绪或 invoke 失败：静默降级，不递归。
      this.subscribed = false;
    }
    return () => this.unsubscribe();
  }

  /** 取消订阅。 */
  unsubscribe(): void {
    if (this.batchListener) {
      this.batchListener();
      this.batchListener = null;
    }
    this.batchHandler = null;
    if (this.subscribed) {
      this.subscribed = false;
      try {
        void invoke('log_unsubscribe');
      } catch {
        // 静默。
      }
    }
  }

  /** 临时设置最低采集等级。 */
  async setLevel(level: string): Promise<void> {
    await invoke('log_set_level', { level });
  }

  /** 开始临时诊断会话。 */
  async startDiagnosticSession(
    minutes?: number,
    level?: string,
    autoExpire?: boolean,
  ): Promise<void> {
    await invoke('log_start_diagnostic_session', {
      minutes: minutes ?? null,
      level: level ?? null,
      autoExpire: autoExpire ?? null,
    });
  }

  /** 手动停止临时诊断会话。 */
  async stopDiagnosticSession(): Promise<void> {
    await invoke('log_stop_diagnostic_session');
  }

  /**
   * 前端写入少量日志。受节流限制，避免高频递归。
   * 失败静默，绝不抛出或递归。
   */
  writeFrontendLog(
    level: string,
    target: string,
    message: string,
    fields?: Record<string, unknown>,
  ): void {
    // 节流：缓冲到队列，定时批量发送。
    this.frontendLogBuffer.push({
      input: {
        level,
        source: 'frontend',
        target,
        message,
        fields: fields ?? {},
      },
    });
    if (this.frontendLogTimer === null) {
      this.frontendLogTimer = setTimeout(() => {
        this.flushFrontendLogs();
      }, FRONTEND_LOG_THROTTLE_MS);
    }
  }

  /** 立即刷新前端日志缓冲。 */
  private flushFrontendLogs(): void {
    this.frontendLogTimer = null;
    const batch = this.frontendLogBuffer.splice(0, FRONTEND_LOG_BATCH_MAX);
    if (batch.length === 0) return;
    for (const args of batch) {
      try {
        void invoke('log_write', args);
      } catch {
        // 静默：Tauri 未就绪或命令失败。
      }
    }
    // 如果还有剩余，继续调度。
    if (this.frontendLogBuffer.length > 0) {
      this.frontendLogTimer = setTimeout(() => {
        this.flushFrontendLogs();
      }, FRONTEND_LOG_THROTTLE_MS);
    }
  }

  /** 导出日志。`path` 由前端通过保存对话框获取。 */
  async exportLogs(scope: ExportScope, path: string): Promise<ExportOutcomeDto> {
    return invoke<ExportOutcomeDto>('log_export', { scope, path });
  }

  /** 导出诊断包 ZIP。`path` 由前端通过保存对话框获取。诊断上下文由后端收集。 */
  async exportDiagnosticsBundle(path: string): Promise<ExportOutcomeDto> {
    return invoke<ExportOutcomeDto>('log_export_diagnostics_bundle', { path });
  }

  /** 打开日志目录。 */
  async openLogDir(): Promise<void> {
    await invoke('log_open_dir');
  }

  /** 销毁客户端：取消订阅、刷新缓冲。 */
  destroy(): void {
    this.unsubscribe();
    if (this.frontendLogTimer) {
      clearTimeout(this.frontendLogTimer);
      this.frontendLogTimer = null;
    }
    this.frontendLogBuffer = [];
  }
}

// 单例。
let logClientInstance: LogClient | null = null;

/** 获取日志客户端单例。 */
export function getLogClient(): LogClient {
  if (!logClientInstance) {
    logClientInstance = new LogClient();
  }
  return logClientInstance;
}

/** 仅用于测试：重置单例。 */
export function _resetLogClientForTest(): void {
  if (logClientInstance) {
    logClientInstance.destroy();
    logClientInstance = null;
  }
}
