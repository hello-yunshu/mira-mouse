// SPDX-License-Identifier: AGPL-3.0-or-later
import { check, type DownloadEvent, type Update } from '@tauri-apps/plugin-updater';
import { invoke } from '@tauri-apps/api/core';
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
export const APP_UPDATE_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
let pendingUpdate: Update | null = null;
let state: AppUpdateState = { phase: 'idle', downloadedBytes: 0 };
let automaticCheckStarted = false;
let automaticCheckTimer: ReturnType<typeof window.setInterval> | undefined;
let automaticInstallRequested = false;
let lastAutomaticCheckAt: number | undefined;

function handleVisibilityChange(): void {
  if (document.visibilityState === 'visible') runAutomaticAppUpdateCheckIfDue();
}

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
  if (automatic) lastAutomaticCheckAt = Date.now();
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
      void invoke('show_update_notification', { title, body }).catch(() => {});
    }
  } catch (error) {
    publish({ phase: 'error', downloadedBytes: 0, error: String(error) });
    if (!automatic) throw error;
  }
}

function automaticCheckShouldRun(): boolean {
  return state.phase !== 'checking'
    && state.phase !== 'downloading'
    && state.phase !== 'available'
    && state.phase !== 'installed';
}

async function runAutomaticAppUpdateCheck(): Promise<void> {
  if (!automaticCheckShouldRun()) return;
  await checkForAppUpdate(true);
  if (automaticInstallRequested && state.phase === 'available') {
    try {
      await installAppUpdate();
      notifyInfo(i18n.t('notification.updateInstalled.title'), i18n.t('notification.updateInstalled.body'), 'relaunch');
    } catch {
      // The error state is already published for the About page.
    }
  }
}

function runAutomaticAppUpdateCheckIfDue(): void {
  if (lastAutomaticCheckAt === undefined || Date.now() - lastAutomaticCheckAt >= APP_UPDATE_CHECK_INTERVAL_MS) {
    void runAutomaticAppUpdateCheck();
  }
}

function ensureAutomaticAppUpdateSchedule(): void {
  if (automaticCheckTimer !== undefined || typeof window === 'undefined') return;
  automaticCheckTimer = window.setInterval(() => {
    void runAutomaticAppUpdateCheck();
  }, APP_UPDATE_CHECK_INTERVAL_MS);
  window.addEventListener('online', runAutomaticAppUpdateCheckIfDue);
  window.addEventListener('focus', runAutomaticAppUpdateCheckIfDue);
  document.addEventListener('visibilitychange', handleVisibilityChange);
}

export function stopAutomaticAppUpdateCheck(): void {
  if (typeof window === 'undefined') return;
  if (automaticCheckTimer !== undefined) {
    window.clearInterval(automaticCheckTimer);
    automaticCheckTimer = undefined;
  }
  window.removeEventListener('online', runAutomaticAppUpdateCheckIfDue);
  window.removeEventListener('focus', runAutomaticAppUpdateCheckIfDue);
  document.removeEventListener('visibilitychange', handleVisibilityChange);
  automaticCheckStarted = false;
  automaticInstallRequested = false;
  lastAutomaticCheckAt = undefined;
}

export async function startAutomaticAppUpdateCheck(enabled: boolean, installAutomatically = false): Promise<void> {
  if (!enabled) {
    stopAutomaticAppUpdateCheck();
    return;
  }
  automaticInstallRequested = installAutomatically;
  ensureAutomaticAppUpdateSchedule();
  if (automaticCheckStarted) return;
  automaticCheckStarted = true;
  await runAutomaticAppUpdateCheck();
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
  await invoke('relaunch_app');
}
