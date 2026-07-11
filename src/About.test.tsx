// SPDX-License-Identifier: AGPL-3.0-or-later
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { AboutPage } from './About';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('./updater', () => ({
  appUpdateState: () => ({ phase: 'available', version: '0.2.0', notes: '更新说明', downloadedBytes: 0 }),
  onAppUpdateState: (listener: (state: unknown) => void) => {
    listener({ phase: 'available', version: '0.2.0', notes: '更新说明', downloadedBytes: 0 });
    return () => undefined;
  },
  checkForAppUpdate: vi.fn(),
  installAppUpdate: vi.fn(),
  relaunchAfterUpdate: vi.fn(),
}));

describe('AboutPage', () => {
  it('renders the complete host skeleton in web preview mode', () => {
    render(<AboutPage previewMode onBack={vi.fn()} />);
    expect(screen.getByRole('heading', { name: '关于' })).toBeInTheDocument();
    expect(screen.getByText('0.1.0-preview')).toBeInTheDocument();
    expect(screen.getByText('Web Preview')).toBeInTheDocument();
    expect(screen.queryByText('Bundle Identifier')).not.toBeInTheDocument();
    expect(screen.queryByText(/加载关于信息失败/)).not.toBeInTheDocument();
  });

  it('shows the checked application version and release notes', async () => {
    invokeMock.mockResolvedValue({
      name: 'Mira', version: '0.1.0', identifier: 'run.hey.mira', platform: 'macos', architecture: 'aarch64',
      rustVersion: '1.82', buildDate: '2026-06-23', gitCommit: 'test', bundledPlugins: [], contact: {}, updaterActive: true,
    });
    render(<AboutPage onBack={vi.fn()} />);
    expect(await screen.findByRole('button', { name: '下载并安装 v0.2.0' })).toBeInTheDocument();
    expect(screen.getByText('更新说明')).toBeInTheDocument();
    expect(screen.queryByText('Bundle Identifier')).not.toBeInTheDocument();
  });

  it('explains when the current build is not release-ready for automatic updates', async () => {
    invokeMock.mockResolvedValue({
      name: 'Mira', version: '0.1.0', identifier: 'run.hey.mira', platform: 'macos', architecture: 'aarch64',
      rustVersion: '1.82', buildDate: '2026-06-23', gitCommit: 'test', bundledPlugins: [], contact: {}, updaterActive: false,
    });
    render(<AboutPage onBack={vi.fn()} />);

    expect(await screen.findByText(/releaseReady: false/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '检查更新' })).not.toBeInTheDocument();
  });

  it('opens donate link through the native browser bridge in Tauri', async () => {
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true });
    invokeMock.mockImplementation((command: string) => {
      if (command === 'about_info') {
        return Promise.resolve({
          name: 'Mira', version: '0.1.0', identifier: 'run.hey.mira', platform: 'macos', architecture: 'aarch64',
          rustVersion: '1.82', buildDate: '2026-06-23', gitCommit: 'test', bundledPlugins: [], contact: {}, updaterActive: false,
        });
      }
      return Promise.resolve(undefined);
    });

    render(<AboutPage onBack={vi.fn()} />);
    fireEvent.click(await screen.findByRole('link', { name: '打赏支持' }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('open_external_url', { url: 'https://hey.run/donate/' }));
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });
});
