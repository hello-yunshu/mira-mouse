// SPDX-License-Identifier: AGPL-3.0-or-later
// 本地 AI 引擎（local-ai bundle）更新管理器。
// 与 src/plugin-updater.ts 对齐：状态机 + 自动检查调度器 + 应用内/系统级通知。
// 额外特性：下载进度条——监听 `local-ai-install-progress` 事件，按 stage 切换显示文案。
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import i18n from './i18n';
import { notifyInfo } from './notify';
import { createAutomaticUpdateScheduler } from './update-check-scheduler';
import { isComponentUpdateNotificationSuppressed } from './update-priority';
import type { LocalAiInstallResult, LocalAiStatus, LocalAiUpdateInfo } from './types';

export type LocalAiUpdatePhase = 'idle' | 'checking' | 'up-to-date' | 'available' | 'downloading' | 'installed' | 'error';

export type LocalAiInstallStage = 'runtime' | 'model' | 'handler' | 'verifying' | 'activating';

export interface LocalAiUpdateState {
  phase: LocalAiUpdatePhase;
  updates: LocalAiUpdateInfo[];
  downloadedBytes: number;
  totalBytes?: number;
  stage?: LocalAiInstallStage;
  error?: string;
}

interface LocalAiInstallProgressPayload {
  component: string;
  stage: LocalAiInstallStage;
  downloadedBytes: number;
  totalBytes?: number;
}

const target = new EventTarget();
export const LOCAL_AI_UPDATE_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
let state: LocalAiUpdateState = { phase: 'idle', updates: [], downloadedBytes: 0 };
let progressUnlisten: UnlistenFn | undefined;

function publish(next: LocalAiUpdateState): void {
  state = next;
  target.dispatchEvent(new CustomEvent<LocalAiUpdateState>('change', { detail: state }));
}

export function localAiUpdateState(): LocalAiUpdateState {
  return state;
}

export function onLocalAiUpdateState(listener: (next: LocalAiUpdateState) => void): () => void {
  const handler = (event: Event) => listener((event as CustomEvent<LocalAiUpdateState>).detail);
  target.addEventListener('change', handler);
  listener(state);
  return () => target.removeEventListener('change', handler);
}

function publishCheckedUpdates(updates: LocalAiUpdateInfo[]): void {
  publish({
    phase: updates.some((item) => item.updateAvailable) ? 'available' : 'up-to-date',
    updates,
    downloadedBytes: 0,
  });
}

export async function checkForLocalAiUpdates(automatic = false): Promise<LocalAiUpdateInfo[]> {
  if (state.phase === 'checking' || state.phase === 'downloading') return state.updates;
  publish({ ...state, phase: 'checking', error: undefined });
  try {
    const updates = await invoke<LocalAiUpdateInfo[]>('local_ai_updates_check');
    publishCheckedUpdates(updates ?? []);
    const available = (updates ?? []).filter((item) => item.updateAvailable);
    if (automatic && available.length > 0) {
      // 应用内 Toast 始终发出；系统级通知受主程序优先级协调层控制。
      const title = i18n.t('notification.localAiUpdateFound');
      const body = i18n.t('notification.localAiUpdateFoundBody', {
        version: available[0].availableVersion,
      });
      notifyInfo(title, body, 'settings-local-ai-update');
      if (!isComponentUpdateNotificationSuppressed()) {
        void invoke('show_update_notification', { title, body, action: 'settings-local-ai-update' }).catch(() => {});
      }
    }
    return updates ?? [];
  } catch (error) {
    publish({ ...state, phase: 'error', error: String(error) });
    if (!automatic) throw error;
    return state.updates;
  }
}

function automaticCheckShouldRun(): boolean {
  return state.phase !== 'checking'
    && state.phase !== 'downloading'
    && state.phase !== 'available';
}

async function runAutomaticLocalAiUpdateCheck(): Promise<boolean> {
  if (!automaticCheckShouldRun()) return false;
  await checkForLocalAiUpdates(true);
  return true;
}

const automaticLocalAiUpdateScheduler = createAutomaticUpdateScheduler({
  intervalMs: LOCAL_AI_UPDATE_CHECK_INTERVAL_MS,
  run: runAutomaticLocalAiUpdateCheck,
});

export function stopAutomaticLocalAiUpdateCheck(): void {
  automaticLocalAiUpdateScheduler.stop();
}

export async function startAutomaticLocalAiUpdateCheck(enabled: boolean): Promise<void> {
  if (!enabled) {
    stopAutomaticLocalAiUpdateCheck();
    return;
  }
  await automaticLocalAiUpdateScheduler.start(true);
}

export async function installLocalAiUpdate(): Promise<LocalAiInstallResult> {
  if (state.phase === 'downloading') throw new Error('local AI update already in progress');
  if (progressUnlisten) {
    progressUnlisten();
    progressUnlisten = undefined;
  }
  publish({ ...state, phase: 'downloading', downloadedBytes: 0, error: undefined });
  progressUnlisten = await listen<LocalAiInstallProgressPayload>('local-ai-install-progress', (event) => {
    const payload = event.payload;
    publish({
      ...state,
      phase: 'downloading',
      downloadedBytes: payload.downloadedBytes,
      totalBytes: payload.totalBytes,
      stage: payload.stage,
    });
  }).catch(() => undefined);
  try {
    const result = await invoke<LocalAiInstallResult>('local_ai_update_install', { component: 'bundle' });
    if (progressUnlisten) {
      progressUnlisten();
      progressUnlisten = undefined;
    }
    const nextStatus = await invoke<LocalAiStatus>('local_ai_status');
    publish({
      phase: 'installed',
      updates: state.updates.map((item) => (item.component === 'bundle'
        ? { ...item, currentVersion: item.availableVersion, updateAvailable: false }
        : item)),
      downloadedBytes: state.totalBytes ?? state.downloadedBytes,
      totalBytes: state.totalBytes,
      stage: 'activating',
    });
    const title = i18n.t('notification.localAiUpdateInstalled');
    const body = i18n.t('notification.localAiUpdateInstalledBody', { version: result.version });
    notifyInfo(title, body, 'settings-local-ai-update');
    if (!isComponentUpdateNotificationSuppressed()) {
      void invoke('show_update_notification', { title, body, action: 'settings-local-ai-update' }).catch(() => {});
    }
    void nextStatus;
    return result;
  } catch (error) {
    if (progressUnlisten) {
      progressUnlisten();
      progressUnlisten = undefined;
    }
    publish({ ...state, phase: 'error', error: String(error) });
    throw error;
  }
}

export async function rollbackLocalAiUpdate(): Promise<LocalAiStatus> {
  if (state.phase === 'downloading') throw new Error('local AI update in progress');
  publish({ ...state, phase: 'checking', error: undefined });
  try {
    const nextStatus = await invoke<LocalAiStatus>('local_ai_update_rollback', { component: 'bundle' });
    const updates = await invoke<LocalAiUpdateInfo[]>('local_ai_updates_check');
    publishCheckedUpdates(updates ?? []);
    return nextStatus;
  } catch (error) {
    publish({ ...state, phase: 'error', error: String(error) });
    throw error;
  }
}
