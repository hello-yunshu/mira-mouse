// SPDX-License-Identifier: AGPL-3.0-or-later
import { check, type DownloadEvent, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { notifyInfo } from './notify';
import i18n from './i18n';

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
let pendingUpdate: Update | null = null;
let state: AppUpdateState = { phase: 'idle', downloadedBytes: 0 };
let automaticCheckStarted = false;

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
    if (automatic) notifyInfo(i18n.t('notification.updateFound.title'), i18n.t('notification.updateFound.body', { version: update.version }));
  } catch (error) {
    publish({ phase: 'error', downloadedBytes: 0, error: String(error) });
    if (!automatic) throw error;
  }
}

export async function startAutomaticAppUpdateCheck(enabled: boolean, installAutomatically = false): Promise<void> {
  if (!enabled || automaticCheckStarted) return;
  automaticCheckStarted = true;
  await checkForAppUpdate(true);
  if (installAutomatically && state.phase === 'available') {
    try {
      await installAppUpdate();
      notifyInfo(i18n.t('notification.updateInstalled.title'), i18n.t('notification.updateInstalled.body'));
    } catch {
      // The error state is already published for the About page.
    }
  }
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
  } catch (error) {
    publish({ ...state, phase: 'error', downloadedBytes, totalBytes, error: String(error) });
    throw error;
  }
}

export async function relaunchAfterUpdate(): Promise<void> {
  await relaunch();
}
