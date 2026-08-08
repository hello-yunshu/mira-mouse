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
import { friendlyUpdateError } from './update-errors';
import type { LocalAiComponent, LocalAiInstallResult, LocalAiStatus, LocalAiUpdateInfo } from './types';

export type LocalAiUpdatePhase = 'idle' | 'checking' | 'rolling-back' | 'up-to-date' | 'available' | 'downloading' | 'installed' | 'error';

export type LocalAiInstallStage = 'runtime' | 'model' | 'handler' | 'verifying' | 'activating';

export interface LocalAiUpdateState {
  phase: LocalAiUpdatePhase;
  updates: LocalAiUpdateInfo[];
  downloadedBytes: number;
  totalBytes?: number;
  stage?: LocalAiInstallStage;
  /** 最近一次安装实际更新的组件（用于安装完成后的徽章与光束）。 */
  updatedComponents?: LocalAiComponent[];
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

/** 返回本地 AI 组件（runtime/model/handler）的用户可读标签。 */
export function localAiComponentLabel(component: LocalAiComponent): string {
  return i18n.t(`settings.localAi.component.${component}`);
}

function formatLocalAiComponentList(parts: string[]): string {
  const locale = i18n.resolvedLanguage ?? i18n.language ?? 'en';
  return new Intl.ListFormat(locale, { style: 'long', type: 'conjunction' }).format(parts);
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
      // 应用内只有一个通知表面；runtime/model/handler 同时可更新时必须
      // 聚合成一条，否则前两条 Toast 会被覆盖，却仍把对应 Beam 排全局队列。
      const title = i18n.t('notification.localAiUpdateFound.title');
      const components = formatLocalAiComponentList(available.map(
        (item) => `${localAiComponentLabel(item.component)} v${item.availableVersion}`,
      ));
      const body = i18n.t('notification.localAiUpdateFound.body', { components });
      notifyInfo(title, body, 'settings-local-ai-update');
      if (!isComponentUpdateNotificationSuppressed()) {
        void invoke('show_update_notification', { title, body, action: 'settings-local-ai-update' }).catch(() => {});
      }
    }
    return updates ?? [];
  } catch (error) {
    publish({ ...state, phase: 'error', error: friendlyUpdateError(error) });
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
    const updatedUpdates = state.updates.filter((item) => item.updateAvailable);
    const updatedComponents = updatedUpdates.map((item) => item.component);
    publish({
      phase: 'installed',
      updates: state.updates.map((item) => (item.updateAvailable
        ? { ...item, currentVersion: item.availableVersion, updateAvailable: false }
        : item)),
      updatedComponents,
      downloadedBytes: state.totalBytes ?? state.downloadedBytes,
      totalBytes: state.totalBytes,
      stage: 'activating',
    });
    const title = i18n.t('notification.localAiUpdateInstalled.title');
    const updatedParts = updatedUpdates.map((item) => `${localAiComponentLabel(item.component)} v${item.availableVersion}`);
    const body = i18n.t('notification.localAiUpdateInstalled.body', {
      components: formatLocalAiComponentList(updatedParts),
    });
    // 应用内 toast 不带 action：用户已在设置页看到更新完成，带 action 会让 toast 可点击，
    // 点击后触发 openSettingsLocalAiUpdate → section focus，出现莫名的 focus outline。
    // 系统级通知保留 action，供不在应用内的用户点击跳转。
    notifyInfo(title, body);
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
    publish({ ...state, phase: 'error', error: friendlyUpdateError(error) });
    throw error;
  }
}

export async function rollbackLocalAiUpdate(): Promise<LocalAiStatus> {
  if (state.phase === 'downloading') throw new Error('local AI update in progress');
  publish({ ...state, phase: 'rolling-back', error: undefined });
  try {
    const nextStatus = await invoke<LocalAiStatus>('local_ai_update_rollback', { component: 'bundle' });
    const updates = await invoke<LocalAiUpdateInfo[]>('local_ai_updates_check');
    publishCheckedUpdates(updates ?? []);
    return nextStatus;
  } catch (error) {
    publish({ ...state, phase: 'error', error: friendlyUpdateError(error) });
    throw error;
  }
}
