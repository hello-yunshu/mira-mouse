// SPDX-License-Identifier: AGPL-3.0-or-later
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from '../App';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn().mockResolvedValue(null) }));

const originalUserAgent = navigator.userAgent;

beforeEach(() => {
  invokeMock.mockImplementation((command: string) => {
    if (command === 'log_status') {
      return Promise.resolve({
        sessionId: 'test-session',
        minLevel: 'info',
        bufferCount: 0,
        bufferCapacity: 4000,
        storageDirDisplay: '${HOME}/logs',
        diskUsageBytes: 0,
        diskQuotaBytes: 20971520,
        recentErrorCount: 0,
        recentWarnCount: 0,
        filePersistenceEnabled: true,
        diagnosticSession: null,
      });
    }
    if (command === 'log_query') {
      return Promise.resolve({ entries: [], hasMore: false, oldestId: null, totalInSession: 0 });
    }
    return Promise.reject(new Error(`unmocked: ${command}`));
  });
});

afterEach(() => {
  invokeMock.mockReset();
  Object.defineProperty(navigator, 'userAgent', { configurable: true, value: originalUserAgent });
});

describe('About page logs card ordering', () => {
  it('renders the logs & diagnostics card as the second card in the about page', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'about_info') {
        return Promise.resolve({
          name: 'Mira', version: '0.1.0', identifier: 'run.hey.mira', platform: 'macos', architecture: 'aarch64',
          rustVersion: '1.82', buildDate: '2026-06-23', gitCommit: 'test', bundledPlugins: [], contact: {}, updaterActive: false,
        });
      }
      if (command === 'log_status') {
        return Promise.resolve({
          sessionId: 's', minLevel: 'info', bufferCount: 0, bufferCapacity: 4000,
          storageDirDisplay: '${HOME}/logs', diskUsageBytes: 0, diskQuotaBytes: 20971520,
          recentErrorCount: 0, recentWarnCount: 0, filePersistenceEnabled: true, diagnosticSession: null,
        });
      }
      if (command === 'log_query') {
        return Promise.resolve({ entries: [], hasMore: false, oldestId: null, totalInSession: 0 });
      }
      return Promise.resolve(undefined);
    });
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: '关于 Mira' }));

    const aboutMain = await screen.findByRole('main');
    const cards = aboutMain.querySelectorAll('section.card');
    expect(cards.length).toBeGreaterThanOrEqual(2);
    // 第 1 块是品牌卡片，第 2 块必须是日志与诊断卡片
    const secondCard = cards[1];
    expect(secondCard).toBeTruthy();
    expect(within(secondCard as HTMLElement).getByRole('heading', { name: '日志与诊断' })).toBeInTheDocument();
    expect(within(secondCard as HTMLElement).getByRole('button', { name: '打开日志与诊断' })).toBeInTheDocument();
  });
});

describe('Logs page navigation and rendering', () => {
  it('opens the logs page from the about page logs card and returns', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'about_info') {
        return Promise.resolve({
          name: 'Mira', version: '0.1.0', identifier: 'run.hey.mira', platform: 'macos', architecture: 'aarch64',
          rustVersion: '1.82', buildDate: '2026-06-23', gitCommit: 'test', bundledPlugins: [], contact: {}, updaterActive: false,
        });
      }
      if (command === 'log_status') {
        return Promise.resolve({
          sessionId: 's', minLevel: 'info', bufferCount: 0, bufferCapacity: 4000,
          storageDirDisplay: '${HOME}/logs', diskUsageBytes: 0, diskQuotaBytes: 20971520,
          recentErrorCount: 0, recentWarnCount: 0, filePersistenceEnabled: true, diagnosticSession: null,
        });
      }
      if (command === 'log_query') {
        return Promise.resolve({ entries: [], hasMore: false, oldestId: null, totalInSession: 0 });
      }
      return Promise.resolve(undefined);
    });
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: '关于 Mira' }));

    const openButton = await screen.findByRole('button', { name: '打开日志与诊断' });
    fireEvent.click(openButton);

    // 日志页标题
    expect(await screen.findByRole('heading', { name: '日志与诊断' })).toBeInTheDocument();
    // 空状态显示
    expect(screen.getByText('没有符合条件的日志')).toBeInTheDocument();

    // 返回关于页
    fireEvent.click(screen.getByRole('button', { name: '返回' }));
    await waitFor(() => expect(screen.getByRole('heading', { name: '关于' })).toBeInTheDocument());
  });

  it('renders an empty state when no logs match the filter', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'about_info') {
        return Promise.resolve({
          name: 'Mira', version: '0.1.0', identifier: 'run.hey.mira', platform: 'macos', architecture: 'aarch64',
          rustVersion: '1.82', buildDate: '2026-06-23', gitCommit: 'test', bundledPlugins: [], contact: {}, updaterActive: false,
        });
      }
      if (command === 'log_status') {
        return Promise.resolve({
          sessionId: 's', minLevel: 'info', bufferCount: 0, bufferCapacity: 4000,
          storageDirDisplay: '${HOME}/logs', diskUsageBytes: 0, diskQuotaBytes: 20971520,
          recentErrorCount: 0, recentWarnCount: 0, filePersistenceEnabled: true, diagnosticSession: null,
        });
      }
      if (command === 'log_query') {
        return Promise.resolve({ entries: [], hasMore: false, oldestId: null, totalInSession: 0 });
      }
      return Promise.resolve(undefined);
    });
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: '关于 Mira' }));
    const openButton = await screen.findByRole('button', { name: '打开日志与诊断' });
    fireEvent.click(openButton);

    expect(await screen.findByText('没有符合条件的日志')).toBeInTheDocument();
  });

  it('clears the current view without invoking log_delete', async () => {
    const entries = [
      { id: 1, timestamp: '2026-07-17T10:00:00+08:00', level: 'info', source: 'app', target: 'test', message: 'hello', sessionId: 's1', fields: {} },
    ];
    invokeMock.mockImplementation((command: string) => {
      if (command === 'about_info') {
        return Promise.resolve({
          name: 'Mira', version: '0.1.0', identifier: 'run.hey.mira', platform: 'macos', architecture: 'aarch64',
          rustVersion: '1.82', buildDate: '2026-06-23', gitCommit: 'test', bundledPlugins: [], contact: {}, updaterActive: false,
        });
      }
      if (command === 'log_status') {
        return Promise.resolve({
          sessionId: 's', minLevel: 'info', bufferCount: 1, bufferCapacity: 4000,
          storageDirDisplay: '${HOME}/logs', diskUsageBytes: 0, diskQuotaBytes: 20971520,
          recentErrorCount: 0, recentWarnCount: 0, filePersistenceEnabled: true, diagnosticSession: null,
        });
      }
      if (command === 'log_query') {
        return Promise.resolve({ entries, hasMore: false, oldestId: 1, totalInSession: 1 });
      }
      return Promise.resolve(undefined);
    });
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: '关于 Mira' }));
    const openButton = await screen.findByRole('button', { name: '打开日志与诊断' });
    fireEvent.click(openButton);

    expect(await screen.findByText('hello')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '清空当前视图' }));

    // 清空视图后空状态显示，且未调用 log_delete
    await waitFor(() => expect(screen.getByText('没有符合条件的日志')).toBeInTheDocument());
    expect(invokeMock).not.toHaveBeenCalledWith('log_delete', expect.anything());
  });

  it('shows the delete confirmation dialog when delete is clicked', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'about_info') {
        return Promise.resolve({
          name: 'Mira', version: '0.1.0', identifier: 'run.hey.mira', platform: 'macos', architecture: 'aarch64',
          rustVersion: '1.82', buildDate: '2026-06-23', gitCommit: 'test', bundledPlugins: [], contact: {}, updaterActive: false,
        });
      }
      if (command === 'log_status') {
        return Promise.resolve({
          sessionId: 's', minLevel: 'info', bufferCount: 0, bufferCapacity: 4000,
          storageDirDisplay: '${HOME}/logs', diskUsageBytes: 0, diskQuotaBytes: 20971520,
          recentErrorCount: 0, recentWarnCount: 0, filePersistenceEnabled: true, diagnosticSession: null,
        });
      }
      if (command === 'log_query') {
        return Promise.resolve({ entries: [], hasMore: false, oldestId: null, totalInSession: 0 });
      }
      return Promise.resolve(undefined);
    });
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: '关于 Mira' }));
    const openButton = await screen.findByRole('button', { name: '打开日志与诊断' });
    fireEvent.click(openButton);

    // 打开删除菜单
    const deleteMenuButtons = screen.getAllByRole('button').filter((b) => b.textContent?.includes('删除'));
    fireEvent.click(deleteMenuButtons[0]);
    // 选择「删除 7 天前日志」
    const olderOption = await screen.findByRole('menuitem', { name: '删除 7 天前日志' });
    fireEvent.click(olderOption);

    // 确认对话框出现
    const dialog = await screen.findByRole('dialog', { name: '删除本地日志' });
    expect(dialog).toBeInTheDocument();
    expect(within(dialog).getByText('该操作不会删除设置、插件、模型、电量历史或其他业务数据。')).toBeInTheDocument();

    // 取消
    fireEvent.click(within(dialog).getByRole('button', { name: '取消' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: '删除本地日志' })).not.toBeInTheDocument());
  });

  it('expands and collapses a log entry', async () => {
    const entries = [
      {
        id: 1, timestamp: '2026-07-17T10:00:00+08:00', level: 'info', source: 'plugin',
        target: 'plugin::verify', message: 'signature ok', sessionId: 's1',
        correlationId: 'c1', fields: { pluginId: 'amaster', durationMs: 120 },
      },
    ];
    invokeMock.mockImplementation((command: string) => {
      if (command === 'about_info') {
        return Promise.resolve({
          name: 'Mira', version: '0.1.0', identifier: 'run.hey.mira', platform: 'macos', architecture: 'aarch64',
          rustVersion: '1.82', buildDate: '2026-06-23', gitCommit: 'test', bundledPlugins: [], contact: {}, updaterActive: false,
        });
      }
      if (command === 'log_status') {
        return Promise.resolve({
          sessionId: 's', minLevel: 'info', bufferCount: 1, bufferCapacity: 4000,
          storageDirDisplay: '${HOME}/logs', diskUsageBytes: 0, diskQuotaBytes: 20971520,
          recentErrorCount: 0, recentWarnCount: 0, filePersistenceEnabled: true, diagnosticSession: null,
        });
      }
      if (command === 'log_query') {
        return Promise.resolve({ entries, hasMore: false, oldestId: 1, totalInSession: 1 });
      }
      return Promise.resolve(undefined);
    });
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: '关于 Mira' }));
    const openButton = await screen.findByRole('button', { name: '打开日志与诊断' });
    fireEvent.click(openButton);

    const entryButton = await screen.findByRole('button', { name: '展开详情' });
    expect(entryButton).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(entryButton);
    await waitFor(() => expect(entryButton).toHaveAttribute('aria-expanded', 'true'));
    // 详情显示结构化字段
    expect(screen.getByText('plugin::verify')).toBeInTheDocument();
    expect(screen.getByText('amaster')).toBeInTheDocument();
    expect(screen.getByText('120')).toBeInTheDocument();
  });
});
