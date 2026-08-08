// SPDX-License-Identifier: AGPL-3.0-or-later
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import i18n from './i18n';
import { checkForLocalAiUpdates } from './local-ai-updater';

const { invokeMock, notifyInfoMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  notifyInfoMock: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('./notify', () => ({ notifyInfo: notifyInfoMock }));

describe('local AI update notifications', () => {
  beforeEach(async () => {
    invokeMock.mockReset();
    notifyInfoMock.mockReset();
    await i18n.changeLanguage('en');
  });

  afterEach(async () => {
    await i18n.changeLanguage('zh-CN');
  });

  it('aggregates simultaneous component updates into one localized notification', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'local_ai_updates_check') {
        return Promise.resolve([
          { component: 'runtime', currentVersion: '1.0.0', availableVersion: '1.2.0', updateAvailable: true },
          { component: 'model', currentVersion: '1.0.0', availableVersion: '1.3.0', updateAvailable: true },
          { component: 'handler', currentVersion: '1.0.0', availableVersion: '1.0.0', updateAvailable: false },
        ]);
      }
      if (command === 'show_update_notification') return Promise.resolve(undefined);
      return Promise.reject(new Error(`not mocked: ${command}`));
    });

    await checkForLocalAiUpdates(true);

    expect(notifyInfoMock).toHaveBeenCalledTimes(1);
    expect(notifyInfoMock).toHaveBeenCalledWith(
      'Local AI updates available',
      'Updates are available for Local AI engine v1.2.0 and Local AI model v1.3.0. Update them in Settings → Plugins.',
      'settings-local-ai-update',
    );
    expect(invokeMock).toHaveBeenCalledWith('show_update_notification', {
      title: 'Local AI updates available',
      body: 'Updates are available for Local AI engine v1.2.0 and Local AI model v1.3.0. Update them in Settings → Plugins.',
      action: 'settings-local-ai-update',
    });
  });
});
