// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  check: vi.fn(),
  relaunch: vi.fn(),
  downloadAndInstall: vi.fn(),
}));

vi.mock('@tauri-apps/plugin-updater', () => ({ check: mocks.check }));
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: mocks.relaunch }));

import {
  appUpdateState,
  checkForAppUpdate,
  installAppUpdate,
  relaunchAfterUpdate,
} from './updater';

describe('application updater', () => {
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
    expect(mocks.relaunch).toHaveBeenCalledOnce();
  });
});
