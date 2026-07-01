// SPDX-License-Identifier: AGPL-3.0-or-later
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  check: vi.fn(),
  invoke: vi.fn(),
  downloadAndInstall: vi.fn(),
}));

vi.mock('@tauri-apps/plugin-updater', () => ({ check: mocks.check }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

import {
  APP_UPDATE_CHECK_INTERVAL_MS,
  appUpdateState,
  checkForAppUpdate,
  installAppUpdate,
  relaunchAfterUpdate,
  startAutomaticAppUpdateCheck,
  stopAutomaticAppUpdateCheck,
} from './updater';

describe('application updater', () => {
  beforeEach(() => {
    mocks.check.mockReset();
    mocks.invoke.mockReset();
    mocks.downloadAndInstall.mockReset();
  });

  afterEach(() => {
    stopAutomaticAppUpdateCheck();
    vi.useRealTimers();
  });

  it('sends a system notification during automatic checks when an update is available', async () => {
    mocks.invoke.mockResolvedValue(undefined);
    mocks.check.mockResolvedValue({
      version: '0.3.0',
      body: 'Release notes',
      date: '2026-06-23T00:00:00Z',
      downloadAndInstall: mocks.downloadAndInstall,
      close: vi.fn().mockResolvedValue(undefined),
    });
    await checkForAppUpdate(true);
    expect(mocks.invoke).toHaveBeenCalledWith('show_update_notification', {
      title: '发现新版本',
      body: 'v0.3.0 已可用，可在「关于」页查看并安装。',
    });
  });

  it('keeps the checked update, reports progress, installs, and relaunches', async () => {
    mocks.downloadAndInstall.mockImplementation(async (onEvent) => {
      onEvent({ event: 'Started', data: { contentLength: 100 } });
      onEvent({ event: 'Progress', data: { chunkLength: 40 } });
      onEvent({ event: 'Progress', data: { chunkLength: 60 } });
      onEvent({ event: 'Finished' });
    });
    mocks.check.mockResolvedValue({
      version: '0.2.0',
      body: 'Release notes',
      date: '2026-06-23T00:00:00Z',
      downloadAndInstall: mocks.downloadAndInstall,
      close: vi.fn().mockResolvedValue(undefined),
    });
    await checkForAppUpdate();
    expect(appUpdateState()).toMatchObject({ phase: 'available', version: '0.2.0' });
    await installAppUpdate();
    expect(mocks.check).toHaveBeenCalledTimes(1);
    expect(appUpdateState()).toMatchObject({ phase: 'installed', downloadedBytes: 100, totalBytes: 100 });
    await relaunchAfterUpdate();
    expect(mocks.invoke).toHaveBeenCalledWith('relaunch_app');
  });

  it('does not send a native update notification when a manual download fails', async () => {
    mocks.downloadAndInstall.mockRejectedValue(new Error('network down'));
    mocks.check.mockResolvedValue({
      version: '0.3.9',
      body: 'Release notes',
      date: '2026-07-01T00:00:00Z',
      downloadAndInstall: mocks.downloadAndInstall,
      close: vi.fn().mockResolvedValue(undefined),
    });
    await checkForAppUpdate();
    await expect(installAppUpdate()).rejects.toThrow('network down');
    expect(appUpdateState()).toMatchObject({ phase: 'error', error: 'Error: network down' });
    expect(mocks.invoke).not.toHaveBeenCalledWith('show_update_notification', expect.anything());
  });

  it('continues automatic checks while the app stays open in the background', async () => {
    vi.useFakeTimers();
    mocks.check.mockResolvedValue(null);

    await startAutomaticAppUpdateCheck(true);
    expect(mocks.check).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(APP_UPDATE_CHECK_INTERVAL_MS - 1);
    expect(mocks.check).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(1);
    expect(mocks.check).toHaveBeenCalledTimes(2);
  });

  it('skips the update-found notification when automatic install is enabled', async () => {
    mocks.invoke.mockResolvedValue(undefined);
    mocks.downloadAndInstall.mockImplementation(async (onEvent) => {
      onEvent({ event: 'Started', data: { contentLength: 100 } });
      onEvent({ event: 'Finished' });
    });
    mocks.check.mockResolvedValue({
      version: '0.3.0',
      body: 'Release notes',
      date: '2026-06-23T00:00:00Z',
      downloadAndInstall: mocks.downloadAndInstall,
      close: vi.fn().mockResolvedValue(undefined),
    });
    await startAutomaticAppUpdateCheck(true, true);
    expect(mocks.invoke).not.toHaveBeenCalledWith('show_update_notification', expect.anything());
    expect(appUpdateState()).toMatchObject({ phase: 'installed' });
  });
});
