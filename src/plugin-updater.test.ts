// SPDX-License-Identifier: AGPL-3.0-or-later
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { invokeMock, notifyInfoMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  notifyInfoMock: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('./notify', () => ({ notifyInfo: notifyInfoMock }));

describe('plugin updater', () => {
  beforeEach(() => {
    vi.resetModules();
    invokeMock.mockReset();
    notifyInfoMock.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('keeps automatic check results available for the settings page', async () => {
    invokeMock.mockResolvedValue([{
      pluginId: 'mira.example', currentVersion: '0.2.0', availableVersion: '0.3.0', updateAvailable: true,
    }]);
    const { checkForPluginUpdates, pluginUpdateState } = await import('./plugin-updater');

    await checkForPluginUpdates(true);

    expect(pluginUpdateState()).toMatchObject({
      phase: 'available',
      updates: [{ pluginId: 'mira.example', availableVersion: '0.3.0', updateAvailable: true }],
    });
    expect(notifyInfoMock).toHaveBeenCalledWith(expect.any(String), expect.any(String), 'settings-plugin-update');
    expect(invokeMock).toHaveBeenCalledWith('show_update_notification', expect.objectContaining({ action: 'settings-plugin-update' }));
  });

  it('refreshes cached results after installing a plugin update', async () => {
    invokeMock
      .mockResolvedValueOnce({ pluginId: 'mira.example', version: '0.3.0', previousVersion: '0.2.0', restartedRuntime: true })
      .mockResolvedValueOnce([{
        pluginId: 'mira.example', currentVersion: '0.3.0', updateAvailable: false,
      }])
      .mockResolvedValue(undefined);
    const { installPluginUpdate, pluginUpdateState } = await import('./plugin-updater');

    await installPluginUpdate('mira.example');

    expect(invokeMock).toHaveBeenNthCalledWith(1, 'plugin_update_install', { pluginId: 'mira.example' });
    expect(invokeMock).toHaveBeenNthCalledWith(2, 'plugin_updates_check');
    expect(pluginUpdateState()).toMatchObject({ phase: 'installed', updates: [{ pluginId: 'mira.example', updateAvailable: false }] });
  });

  it('keeps automatic checks alive for long-running sessions', async () => {
    vi.useFakeTimers();
    invokeMock.mockResolvedValue([]);
    const { PLUGIN_UPDATE_CHECK_INTERVAL_MS, startAutomaticPluginUpdateCheck, stopAutomaticPluginUpdateCheck } = await import('./plugin-updater');

    await startAutomaticPluginUpdateCheck(true);
    expect(invokeMock).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(PLUGIN_UPDATE_CHECK_INTERVAL_MS);
    expect(invokeMock).toHaveBeenCalledTimes(2);
    stopAutomaticPluginUpdateCheck();
  });
});
