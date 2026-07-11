// SPDX-License-Identifier: AGPL-3.0-or-later
import { invoke } from '@tauri-apps/api/core';
import i18n from './i18n';
import { notifyInfo } from './notify';
import type { PluginInstallResult, PluginUpdateInfo } from './types';
import { createAutomaticUpdateScheduler } from './update-check-scheduler';

export type PluginUpdatePhase = 'idle' | 'checking' | 'up-to-date' | 'available' | 'installing' | 'error';

export interface PluginUpdateState {
  phase: PluginUpdatePhase;
  updates: PluginUpdateInfo[];
  installingPluginId?: string;
  error?: string;
}

const target = new EventTarget();
export const PLUGIN_UPDATE_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
let state: PluginUpdateState = { phase: 'idle', updates: [] };

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
  });
}

export async function checkForPluginUpdates(automatic = false): Promise<PluginUpdateInfo[]> {
  if (state.phase === 'checking' || state.phase === 'installing') return state.updates;
  publish({ ...state, phase: 'checking', error: undefined });
  try {
    const updates = await invoke<PluginUpdateInfo[]>('plugin_updates_check');
    publishCheckedUpdates(updates);
    const available = updates.filter((item) => item.updateAvailable);
    if (automatic && available.length > 0) {
      const title = i18n.t('dashboard.pluginUpdateFound');
      const body = i18n.t('dashboard.pluginUpdateFoundBody', { count: available.length });
      notifyInfo(title, body, 'settings-plugin-update');
      void invoke('show_update_notification', { title, body, action: 'settings-plugin-update' }).catch(() => {});
    }
    return updates;
  } catch (error) {
    publish({ ...state, phase: 'error', error: String(error), installingPluginId: undefined });
    if (!automatic) throw error;
    return state.updates;
  }
}

function automaticCheckShouldRun(): boolean {
  return state.phase !== 'checking'
    && state.phase !== 'installing'
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
  if (state.phase === 'installing') throw new Error('plugin update already in progress');
  publish({ ...state, phase: 'installing', installingPluginId: pluginId, error: undefined });
  try {
    const result = await invoke<PluginInstallResult>('plugin_update_install', { pluginId });
    const updates = await invoke<PluginUpdateInfo[]>('plugin_updates_check');
    publishCheckedUpdates(updates);
    return result;
  } catch (error) {
    publish({ ...state, phase: 'error', error: String(error), installingPluginId: undefined });
    throw error;
  }
}
