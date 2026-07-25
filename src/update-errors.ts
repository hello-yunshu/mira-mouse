// SPDX-License-Identifier: AGPL-3.0-or-later
// 更新通道错误友好化：将后端返回的技术性错误（含镜像 URL、堆栈细节）
// 转换为面向用户的简洁提示。三个更新通道（主程序 / 鼠标插件 / 本地 AI）
// 共用此模块，确保错误呈现对齐一致。
import i18n from './i18n';

const NETWORK_ERROR_PATTERNS: RegExp[] = [
  /all \d+ mirror source\(s\) unavailable/i,
  /failed to fetch local AI release index/i,
  /download failed/i,
  /network error/i,
  /failed to fetch/i,
  /ECONNREFUSED|ENOTFOUND|ETIMEDOUT|ECONNRESET/i,
];

const VERIFICATION_ERROR_PATTERNS: RegExp[] = [
  /SHA-?256 mismatch/i,
  /artifact size mismatch/i,
  /artifact exceeds the size limit/i,
  /signature or package validation failed/i,
  /registry identity does not match/i,
  /signature verification failed/i,
];

const RAW_ERROR_MAX_LENGTH = 200;

/**
 * 将任意错误转换为面向用户的友好字符串。
 *
 * - 识别网络下载失败模式 → 返回 i18n `notification.updateErrorNetwork`
 * - 识别包校验失败模式 → 返回 i18n `notification.updateErrorVerification`
 * - 其他错误 → 返回截断后的原始错误字符串（保留诊断信息）
 *
 * 注意：原始错误的完整镜像 URL 与堆栈细节已通过 Rust 端 `eprintln!`
 * 输出到 stderr，开发者仍可诊断。
 */
export function friendlyUpdateError(error: unknown): string {
  const raw = error instanceof Error ? error.message : String(error);
  if (NETWORK_ERROR_PATTERNS.some((pattern) => pattern.test(raw))) {
    return i18n.t('notification.updateErrorNetwork');
  }
  if (VERIFICATION_ERROR_PATTERNS.some((pattern) => pattern.test(raw))) {
    return i18n.t('notification.updateErrorVerification');
  }
  if (raw.length > RAW_ERROR_MAX_LENGTH) {
    return `${raw.slice(0, RAW_ERROR_MAX_LENGTH)}…`;
  }
  return raw;
}
