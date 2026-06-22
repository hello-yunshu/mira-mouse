// SPDX-License-Identifier: AGPL-3.0-or-later
import { sendNotification } from '@tauri-apps/plugin-notification';

const isTauri = (): boolean =>
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/**
 * 发送系统级错误通知。非 Tauri 环境下回退到 console.error。
 */
export function notifyError(title: string, body?: string): void {
  if (isTauri()) {
    try {
      sendNotification({ title, body });
      return;
    } catch (error) {
      console.error('通知发送失败，回退到控制台：', error);
    }
  }
  console.error(title, body ?? '');
}
