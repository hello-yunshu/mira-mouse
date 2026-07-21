// SPDX-License-Identifier: AGPL-3.0-or-later
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import i18n from './i18n';
import { notifyInfo } from './notify';
import type { PluginInstallResult, PluginUpdateInfo } from './types';
import { createAutomaticUpdateScheduler } from './update-check-scheduler';
import { isComponentUpdateNotificationSuppressed } from './update-priority';

export type PluginUpdatePhase = 'idle' | 'checking' | 'up-to-date' | 'available' | 'downloading' | 'installed' | 'error';

/** 当前安装阶段，对应后端 `PluginInstallProgress.stage`。 */
export type PluginInstallStage = 'downloading' | 'verifying' | 'activating';

export interface PluginUpdateState {
  phase: PluginUpdatePhase;
  updates: PluginUpdateInfo[];
  installingPluginId?: string;
  downloadedBytes: number;
  totalBytes?: number;
  stage?: PluginInstallStage;
  error?: string;
}

interface PluginInstallProgressPayload {
  pluginId: string;
  stage: PluginInstallStage;
  downloadedBytes: number;
  totalBytes?: number;
}

const target = new EventTarget();
export const PLUGIN_UPDATE_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
let state: PluginUpdateState = { phase: 'idle', updates: [], downloadedBytes: 0 };

function publish(next: PluginUpdateState): void {
  state = next;
  target.dispatchEvent(new CustomEvent<PluginUpdateState>('change', { detail: state }));
}

export function pluginUpdateState(): PluginUpdateState {
  return state;
}

export function onPluginUpdateState(listener: (next: PluginUpdateState) => void): () => void {
  const handler = (event: Event) => listener((event as CustomEvent<PluginUpdateState>).detail);
  target.addEventListener('change', handler);
  listener(state);
  return () => target.removeEventListener('change', handler);
}

function publishCheckedUpdates(updates: PluginUpdateInfo[]): void {
  publish({
    phase: updates.some((item) => item.updateAvailable) ? 'available' : 'up-to-date',
    updates,
    downloadedBytes: 0,
  });
}

export async function checkForPluginUpdates(automatic = false): Promise<PluginUpdateInfo[]> {
  if (state.phase === 'checking' || state.phase === 'downloading') return state.updates;
  publish({ ...state, phase: 'checking', error: undefined });
  try {
    const updates = await invoke<PluginUpdateInfo[]>('plugin_updates_check');
    publishCheckedUpdates(updates);
    const available = updates.filter((item) => item.updateAvailable);
    if (automatic && available.length > 0) {
      const title = i18n.t('dashboard.pluginUpdateFound');
      const body = i18n.t('dashboard.pluginUpdateFoundBody', { count: available.length });
      notifyInfo(title, body, 'settings-plugin-update');
      if (!isComponentUpdateNotificationSuppressed()) {
        void invoke('show_update_notification', { title, body, action: 'settings-plugin-update' }).catch(() => {});
      }
    }
    return updates;
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

async function runAutomaticPluginUpdateCheck(): Promise<boolean> {
  if (!automaticCheckShouldRun()) return false;
  await checkForPluginUpdates(true);
  return true;
}

const automaticPluginUpdateScheduler = createAutomaticUpdateScheduler({
  intervalMs: PLUGIN_UPDATE_CHECK_INTERVAL_MS,
  run: runAutomaticPluginUpdateCheck,
});

export function stopAutomaticPluginUpdateCheck(): void {
  automaticPluginUpdateScheduler.stop();
}

export async function startAutomaticPluginUpdateCheck(enabled: boolean): Promise<void> {
  if (!enabled) {
    stopAutomaticPluginUpdateCheck();
    return;
  }
  await automaticPluginUpdateScheduler.start(true);
}

export async function installPluginUpdate(pluginId: string): Promise<PluginInstallResult> {
  if (state.phase === 'downloading') throw new Error('plugin update already in progress');
  let progressUnlisten: UnlistenFn | undefined;
  publish({ ...state, phase: 'downloading', installingPluginId: pluginId, downloadedBytes: 0, error: undefined });
  progressUnlisten = await listen<PluginInstallProgressPayload>('plugin-install-progress', (event) => {
    const payload = event.payload;
    if (payload.pluginId !== pluginId) return;
    publish({
      ...state,
      phase: 'downloading',
      installingPluginId: pluginId,
      downloadedBytes: payload.downloadedBytes,
      totalBytes: payload.totalBytes,
      stage: payload.stage,
    });
  }).catch(() => undefined);
  try {
    const result = await invoke<PluginInstallResult>('plugin_update_install', { pluginId });
    if (progressUnlisten) {
      progressUnlisten();
      progressUnlisten = undefined;
    }
    const updates = await invoke<PluginUpdateInfo[]>('plugin_updates_check');
    publish({
      phase: 'installed',
      updates,
      installingPluginId: undefined,
      downloadedBytes: state.totalBytes ?? state.downloadedBytes,
      totalBytes: state.totalBytes,
      stage: 'activating',
    });
    const title = i18n.t('notification.pluginUpdateInstalled');
    const body = i18n.t('notification.pluginUpdateInstalledBody', {
      pluginId: result.pluginId,
      version: result.version,
    });
    notifyInfo(title, body, 'settings-plugin-update');
    if (!isComponentUpdateNotificationSuppressed()) {
      void invoke('show_update_notification', { title, body, action: 'settings-plugin-update' }).catch(() => {});
    }
    return result;
  } catch (error) {
    if (progressUnlisten) {
      progressUnlisten();
    }
    publish({ ...state, phase: 'error', error: String(error), installingPluginId: undefined });
    throw error;
  }
}
