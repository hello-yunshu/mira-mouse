// SPDX-License-Identifier: AGPL-3.0-or-later
import { check, type DownloadEvent, type Update } from '@tauri-apps/plugin-updater';
import { invoke } from '@tauri-apps/api/core';
import { notifyInfo } from './notify';
import i18n from './i18n';
import { createAutomaticUpdateScheduler } from './update-check-scheduler';
import { friendlyUpdateError } from './update-errors';

export type AppUpdatePhase = 'idle' | 'checking' | 'up-to-date' | 'available' | 'downloading' | 'installed' | 'error';

export interface AppUpdateState {
  phase: AppUpdatePhase;
  version?: string;
  notes?: string;
  date?: string;
  downloadedBytes: number;
  totalBytes?: number;
  error?: string;
}

const target = new EventTarget();
export const APP_UPDATE_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
let pendingUpdate: Update | null = null;
let state: AppUpdateState = { phase: 'idle', downloadedBytes: 0 };
let automaticInstallRequested = false;

// 更新已安装但用户一直未重启时的重启提醒节流：
// 用户主动关闭通知累计 2 次，或任由通知超时消失累计 3 次后，不再打扰。
// installAppUpdate 成功时重置，开启新的安装周期。
const UPDATE_REMINDER_DISMISS_LIMIT = 2;
const UPDATE_REMINDER_IGNORE_LIMIT = 3;
let remindDismissedCount = 0;
let remindIgnoredCount = 0;

function publish(next: AppUpdateState): void {
  state = next;
  target.dispatchEvent(new CustomEvent<AppUpdateState>('change', { detail: state }));
}

export function appUpdateState(): AppUpdateState {
  return state;
}

export function onAppUpdateState(listener: (state: AppUpdateState) => void): () => void {
  const handler = (event: Event) => listener((event as CustomEvent<AppUpdateState>).detail);
  target.addEventListener('change', handler);
  listener(state);
  return () => target.removeEventListener('change', handler);
}

export async function checkForAppUpdate(automatic = false): Promise<void> {
  if (state.phase === 'checking' || state.phase === 'downloading') return;
  publish({ phase: 'checking', downloadedBytes: 0 });
  try {
    if (pendingUpdate) {
      await pendingUpdate.close().catch(() => undefined);
      pendingUpdate = null;
    }
    const update = await check();
    if (!update) {
      pendingUpdate = null;
      publish({ phase: 'up-to-date', downloadedBytes: 0 });
      return;
    }
    pendingUpdate = update;
    publish({
      phase: 'available',
      version: update.version,
      notes: update.body,
      date: update.date,
      downloadedBytes: 0,
    });
    if (automatic && !automaticInstallRequested) {
      const title = i18n.t('notification.updateFound.title');
      const body = i18n.t('notification.updateFound.body', { version: update.version });
      notifyInfo(title, body, 'about-update');
      void invoke('show_update_notification', { title, body, action: 'about-update' }).catch(() => {});
    }
  } catch (error) {
    publish({ phase: 'error', downloadedBytes: 0, error: friendlyUpdateError(error) });
    if (!automatic) throw error;
  }
}

function automaticCheckShouldRun(): boolean {
  return state.phase !== 'checking'
    && state.phase !== 'downloading'
    && state.phase !== 'available'
    && state.phase !== 'installed';
}

async function runAutomaticAppUpdateCheck(): Promise<boolean> {
  if (!automaticCheckShouldRun()) return false;
  await checkForAppUpdate(true);
  if (automaticInstallRequested && state.phase === 'available') {
    try {
      await installAppUpdate();
    } catch {
      // The error state is already published for the About page.
    }
  }
  return true;
}

const automaticAppUpdateScheduler = createAutomaticUpdateScheduler({
  intervalMs: APP_UPDATE_CHECK_INTERVAL_MS,
  run: runAutomaticAppUpdateCheck,
});

export function stopAutomaticAppUpdateCheck(): void {
  automaticAppUpdateScheduler.stop();
  automaticInstallRequested = false;
  remindDismissedCount = 0;
  remindIgnoredCount = 0;
}

export async function startAutomaticAppUpdateCheck(enabled: boolean, installAutomatically = false): Promise<void> {
  if (!enabled) {
    stopAutomaticAppUpdateCheck();
    return;
  }
  automaticInstallRequested = installAutomatically;
  await automaticAppUpdateScheduler.start(true);
}

export async function installAppUpdate(): Promise<void> {
  if (!pendingUpdate) await checkForAppUpdate();
  if (!pendingUpdate) return;
  const update = pendingUpdate;
  let downloadedBytes = 0;
  let totalBytes: number | undefined;
  publish({ ...state, phase: 'downloading', downloadedBytes: 0, error: undefined });
  const onEvent = (event: DownloadEvent) => {
    if (event.event === 'Started') totalBytes = event.data.contentLength;
    if (event.event === 'Progress') downloadedBytes += event.data.chunkLength;
    publish({ ...state, phase: 'downloading', downloadedBytes, totalBytes });
  };
  try {
    await update.downloadAndInstall(onEvent);
    await update.close().catch(() => undefined);
    pendingUpdate = null;
    publish({ ...state, phase: 'installed', downloadedBytes, totalBytes, error: undefined });
    // 新的安装周期：重置提醒的关闭/忽视计数。
    remindDismissedCount = 0;
    remindIgnoredCount = 0;
    // 安装完成后发送重启通知：应用内 Toast（点击直接重启）+ 原生系统通知（点击跳转到关于页）。
    // 无论手动还是自动安装都发送，覆盖用户在下载过程中切走、应用在后台等场景。
    const title = i18n.t('notification.updateInstalled.title');
    const body = i18n.t('notification.updateInstalled.body');
    notifyInfo(title, body, 'relaunch');
    void invoke('show_update_notification', { title, body, action: 'about-update' }).catch(() => {});
  } catch (error) {
    publish({ ...state, phase: 'error', downloadedBytes, totalBytes, error: friendlyUpdateError(error) });
    throw error;
  }
}

export async function relaunchAfterUpdate(): Promise<void> {
  // 统一交给后端处理平台生命周期：macOS 等旧进程退出后打开 .app bundle，
  // Windows/Linux 先完成 RunEvent::Exit 清理，再重启 exe/AppImage。
  await invoke('relaunch_app');
}

// 更新已安装但用户一直未重启时，每次重新打开主程序（窗口从托盘恢复）在程序内提醒一次。
// 仅发应用内 Toast（点击直接重启），不重复弹系统通知。
function shouldRemindInstalledUpdate(): boolean {
  return remindDismissedCount < UPDATE_REMINDER_DISMISS_LIMIT
    && remindIgnoredCount < UPDATE_REMINDER_IGNORE_LIMIT;
}

/** 用户主动点击关闭按钮关闭更新重启提醒。 */
export function recordUpdateReminderDismissed(): void {
  if (state.phase !== 'installed') return;
  remindDismissedCount += 1;
}

/** 更新重启提醒因超时自动消失（用户未做任何操作）。 */
export function recordUpdateReminderIgnored(): void {
  if (state.phase !== 'installed') return;
  remindIgnoredCount += 1;
}

/** 主窗口从托盘恢复时调用：若更新已安装等待重启且未达阈值，发送应用内提醒。 */
export function remindInstalledUpdateOnShown(): void {
  if (state.phase !== 'installed') return;
  if (!shouldRemindInstalledUpdate()) return;
  notifyInfo(
    i18n.t('notification.updateInstalled.title'),
    i18n.t('notification.updateInstalled.body'),
    'relaunch',
  );
}
