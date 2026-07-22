// SPDX-License-Identifier: AGPL-3.0-or-later
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from '../App';
import { LogPage } from './LogPage';
import { LOG_BATCH_EVENT, type LogEntry } from './log-types';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn().mockResolvedValue(null) }));

// LogClient.subscribe 内部调用 listen 订阅 mira://logs/batch。mock listen 既能避免
// 真实 Tauri event 在 jsdom 中抛错，又允许我们在测试中捕获批次回调、推送实时事件。
const { listenMock } = vi.hoisted(() => ({ listenMock: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }));

const originalUserAgent = navigator.userAgent;

/** 构造一个仅处理 log_status/log_query/log_subscribe/log_unsubscribe 的 invoke 实现。 */
function makeInvokeImpl(opts: { entries?: LogEntry[] } = {}) {
  const entries = opts.entries ?? [];
  return (command: string) => {
    if (command === 'log_status') {
      return Promise.resolve({
        sessionId: 'test-session',
        minLevel: 'info',
        bufferCount: entries.length,
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
      const oldestId = entries.length ? entries[entries.length - 1].id : null;
      return Promise.resolve({ entries, hasMore: false, oldestId, totalInSession: entries.length });
    }
    if (command === 'log_subscribe' || command === 'log_unsubscribe') {
      return Promise.resolve(undefined);
    }
    return Promise.reject(new Error(`unmocked: ${command}`));
  };
}

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
    if (command === 'log_subscribe' || command === 'log_unsubscribe') {
      return Promise.resolve(undefined);
    }
    return Promise.reject(new Error(`unmocked: ${command}`));
  });
  // 默认让 listen 返回一个 no-op unlisten，避免 App 中其它 listen 调用返回 undefined 而抛错。
  listenMock.mockResolvedValue(() => undefined);
});

afterEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
  Object.defineProperty(navigator, 'userAgent', { configurable: true, value: originalUserAgent });
});

describe('Settings about tab logs card ordering', () => {
  it('renders the logs & diagnostics card as the second section in the settings about tab', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') { return Promise.resolve({}); }
      if (command === 'device_snapshots') { return Promise.resolve([]); }
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
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    const aboutTab = await screen.findByRole('button', { name: /^关于$/ });
    fireEvent.click(aboutTab);

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
      if (command === 'settings_get') { return Promise.resolve({}); }
      if (command === 'device_snapshots') { return Promise.resolve([]); }
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
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    const aboutTab = await screen.findByRole('button', { name: /^关于$/ });
    fireEvent.click(aboutTab);

    const openButton = await screen.findByRole('button', { name: '打开日志与诊断' });
    fireEvent.click(openButton);

    // 日志页标题
    expect(await screen.findByRole('heading', { name: '日志与诊断' })).toBeInTheDocument();
    // 空状态显示
    expect(screen.getByText('没有符合条件的日志')).toBeInTheDocument();

    // 返回关于页
    fireEvent.click(screen.getByRole('button', { name: '返回' }));
    await waitFor(() => expect(screen.getByRole('button', { name: '打开日志与诊断' })).toBeInTheDocument());
  });

  it('renders an empty state when no logs match the filter', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') { return Promise.resolve({}); }
      if (command === 'device_snapshots') { return Promise.resolve([]); }
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
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    const aboutTab = await screen.findByRole('button', { name: /^关于$/ });
    fireEvent.click(aboutTab);
    const openButton = await screen.findByRole('button', { name: '打开日志与诊断' });
    fireEvent.click(openButton);

    expect(await screen.findByText('没有符合条件的日志')).toBeInTheDocument();
  });

  it('clears the current view without invoking log_delete', async () => {
    const entries = [
      { id: 1, timestamp: '2026-07-17T10:00:00+08:00', level: 'info', source: 'app', target: 'test', message: 'hello', sessionId: 's1', fields: {} },
    ];
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') { return Promise.resolve({}); }
      if (command === 'device_snapshots') { return Promise.resolve([]); }
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
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    const aboutTab = await screen.findByRole('button', { name: /^关于$/ });
    fireEvent.click(aboutTab);
    const openButton = await screen.findByRole('button', { name: '打开日志与诊断' });
    fireEvent.click(openButton);

    expect(await screen.findByText('hello')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '更多操作' }));
    fireEvent.click(screen.getByRole('menuitem', { name: '清空当前视图' }));

    // 清空视图后空状态显示，且未调用 log_delete
    await waitFor(() => expect(screen.getByText('没有符合条件的日志')).toBeInTheDocument());
    expect(invokeMock).not.toHaveBeenCalledWith('log_delete', expect.anything());
  });

  it('shows the delete confirmation dialog when delete is clicked', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') { return Promise.resolve({}); }
      if (command === 'device_snapshots') { return Promise.resolve([]); }
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
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    const aboutTab = await screen.findByRole('button', { name: /^关于$/ });
    fireEvent.click(aboutTab);
    const openButton = await screen.findByRole('button', { name: '打开日志与诊断' });
    fireEvent.click(openButton);

    // 打开更多操作菜单，选择「删除 7 天前日志」
    fireEvent.click(screen.getByRole('button', { name: '更多操作' }));
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
      if (command === 'settings_get') { return Promise.resolve({}); }
      if (command === 'device_snapshots') { return Promise.resolve([]); }
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
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    const aboutTab = await screen.findByRole('button', { name: /^关于$/ });
    fireEvent.click(aboutTab);
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

// 直接渲染 LogPage 的单元测试，隔离工具栏 / 对话框 / 实时事件行为。
describe('LogPage toolbar and dialogs', () => {
  function renderLogPage(onBack: () => void = () => {}) {
    return render(<LogPage onBack={onBack} />);
  }

  it('lists three delete-scope options in the delete menu', async () => {
    renderLogPage();
    fireEvent.click(screen.getByRole('button', { name: '更多操作' }));

    expect(await screen.findByRole('menuitem', { name: '删除 7 天前日志' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: '删除本次会话之前的日志' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: '删除全部本地日志' })).toBeInTheDocument();
  });

  it('copies the filtered entries as JSONL to the clipboard', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    const entry: LogEntry = {
      id: 42, timestamp: '2026-07-17T10:00:00+08:00', level: 'info', source: 'app',
      target: 'mod::x', message: 'copy-me', sessionId: 's1', fields: { k: 'v' },
    };
    invokeMock.mockImplementation(makeInvokeImpl({ entries: [entry] }));

    renderLogPage();
    expect(await screen.findByText('copy-me')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '更多操作' }));
    fireEvent.click(screen.getByRole('menuitem', { name: '复制筛选结果' }));
    await waitFor(() => expect(writeText).toHaveBeenCalled());

    const payload = writeText.mock.calls[0][0] as string;
    expect(payload).toContain('"id":42');
    expect(payload).toContain('copy-me');
    // JSONL：每行一条 JSON
    expect(payload.split('\n').length).toBe(1);
  });

  it('keeps newest entries above older entries', async () => {
    const entries: LogEntry[] = [
      { id: 3, timestamp: '2026-07-17T10:00:03+08:00', level: 'info', source: 'app', target: 'm', message: 'newest', sessionId: 's1', fields: {} },
      { id: 2, timestamp: '2026-07-17T10:00:02+08:00', level: 'info', source: 'app', target: 'm', message: 'middle', sessionId: 's1', fields: {} },
      { id: 1, timestamp: '2026-07-17T10:00:01+08:00', level: 'info', source: 'app', target: 'm', message: 'oldest', sessionId: 's1', fields: {} },
    ];
    invokeMock.mockImplementation(makeInvokeImpl({ entries }));

    renderLogPage();
    await screen.findByText('newest');
    const summaries = [...document.querySelectorAll('.log-entry-summary')].map((node) => node.textContent);
    expect(summaries[0]).toContain('newest');
    expect(summaries[1]).toContain('middle');
    expect(summaries[2]).toContain('oldest');
  });

  it('collapses every expanded log from the toolbar', async () => {
    const entries: LogEntry[] = [
      { id: 2, timestamp: '2026-07-17T10:00:02+08:00', level: 'info', source: 'app', target: 'm', message: 'second', sessionId: 's1', fields: { value: 2 } },
      { id: 1, timestamp: '2026-07-17T10:00:01+08:00', level: 'info', source: 'app', target: 'm', message: 'first', sessionId: 's1', fields: { value: 1 } },
    ];
    invokeMock.mockImplementation(makeInvokeImpl({ entries }));

    renderLogPage();
    await screen.findByText('second');
    const expandButtons = screen.getAllByRole('button', { name: '展开详情' });
    fireEvent.click(expandButtons[0]);
    fireEvent.click(expandButtons[1]);

    expect(screen.getAllByRole('button', { name: '收起详情' })).toHaveLength(2);
    const collapseAll = screen.getByRole('button', { name: '收起全部详情（2）' });
    fireEvent.click(collapseAll);

    expect(screen.getAllByRole('button', { name: '展开详情' })).toHaveLength(2);
    expect(screen.queryByRole('button', { name: /收起全部详情/ })).not.toBeInTheDocument();
  });

  it('localizes structured AI events and field labels', async () => {
    const entry: LogEntry = {
      id: 9,
      timestamp: '2026-07-17T10:00:09+08:00',
      level: 'info',
      source: 'local-ai',
      target: 'local_ai::predict',
      message: 'prediction batch ok',
      sessionId: 's1',
      fields: {
        event: 'local-ai-prediction-completed',
        status: 'ok',
        batchCount: 2,
        resultCount: 2,
        fallback: false,
        durationMs: 38,
      },
    };
    invokeMock.mockImplementation(makeInvokeImpl({ entries: [entry] }));

    renderLogPage();
    expect(await screen.findByText('本地 AI 续航预测已完成')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '展开详情' }));

    expect(screen.getByText('结果')).toBeInTheDocument();
    expect(screen.getByText('成功')).toBeInTheDocument();
    expect(screen.getByText('设备数量')).toBeInTheDocument();
    expect(screen.getByText('预测成功')).toBeInTheDocument();
    expect(screen.getByText('是否回退')).toBeInTheDocument();
    expect(screen.getByText('否')).toBeInTheDocument();
    expect(screen.getByText('耗时')).toBeInTheDocument();
  });

  it('does not list frontend as a top-level source filter option', async () => {
    renderLogPage();
    const sourceSelect = await screen.findByRole('combobox', { name: '来源' }) as HTMLSelectElement;
    const optionTexts = Array.from(sourceSelect.options).map((o) => o.textContent);
    expect(optionTexts).toEqual(['全部', '程序', '插件', '本地 AI']);
    expect(optionTexts).not.toContain('前端');
  });

  it('queries with source=app when the 程序 option is selected', async () => {
    renderLogPage();
    const sourceSelect = await screen.findByRole('combobox', { name: '来源' });
    fireEvent.change(sourceSelect, { target: { value: 'app' } });

    await waitFor(() => {
      const queryCalls = invokeMock.mock.calls.filter(([c]) => c === 'log_query');
      expect(queryCalls.length).toBeGreaterThan(0);
      const lastCall = queryCalls[queryCalls.length - 1];
      expect(lastCall[1]).toMatchObject({ query: expect.objectContaining({ source: 'app' }) });
    });
  });

  it('renders an accessible delete confirmation dialog focused on the cancel button', async () => {
    renderLogPage();
    fireEvent.click(screen.getByRole('button', { name: '更多操作' }));
    fireEvent.click(await screen.findByRole('menuitem', { name: '删除 7 天前日志' }));

    const dialog = await screen.findByRole('dialog', { name: '删除本地日志' });
    expect(dialog).toHaveAttribute('aria-modal', 'true');
    expect(within(dialog).getAllByText('删除本地日志')).toHaveLength(1);
    expect(within(dialog).getByRole('heading', { name: '删除本地日志' })).toBeInTheDocument();
    const cancelButton = within(dialog).getByRole('button', { name: '取消' });
    // Modal 通过 requestAnimationFrame 把焦点移入弹窗；等待 rAF 执行后再断言。
    await act(async () => {
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    });
    expect(cancelButton).toHaveFocus();

    // 取消按钮关闭对话框
    fireEvent.click(cancelButton);
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });

  it('buffers new logs while paused and merges them on resume', async () => {
    let batchListener: ((event: { payload: LogEntry[] }) => void) | null = null;
    listenMock.mockImplementation(async (eventName: string, cb: (event: { payload: LogEntry[] }) => void) => {
      if (eventName === LOG_BATCH_EVENT) batchListener = cb;
      return () => undefined;
    });

    renderLogPage();
    await screen.findByText('没有符合条件的日志');
    await waitFor(() => expect(batchListener).not.toBeNull());

    // 暂停刷新
    fireEvent.click(screen.getByRole('button', { name: '暂停刷新' }));

    // 推送一批实时日志
    const batch: LogEntry[] = [
      { id: 1, timestamp: '2026-07-17T10:00:00+08:00', level: 'info', source: 'app', target: 'm', message: 'paused-1', sessionId: 's1', fields: {} },
      { id: 2, timestamp: '2026-07-17T10:00:01+08:00', level: 'info', source: 'app', target: 'm', message: 'paused-2', sessionId: 's1', fields: {} },
    ];
    await act(async () => {
      batchListener!({ payload: batch });
    });

    // 暂停期间日志被缓冲，不进入列表
    expect(screen.queryByText('paused-1')).not.toBeInTheDocument();

    // 恢复：缓冲的日志合并到列表
    fireEvent.click(screen.getByRole('button', { name: '继续刷新' }));
    expect(await screen.findByText('paused-1')).toBeInTheDocument();
    expect(screen.getByText('paused-2')).toBeInTheDocument();
  });

  it('does not force-scroll to the top when the user is reading older logs', async () => {
    const makeEntry = (id: number): LogEntry => ({
      id, timestamp: `2026-07-17T10:00:0${id}+08:00`, level: 'info', source: 'app',
      target: 'm', message: `scroll-msg-${id}`, sessionId: 's1', fields: {},
    });
    invokeMock.mockImplementation(makeInvokeImpl({ entries: [makeEntry(1)] }));
    let batchListener: ((event: { payload: LogEntry[] }) => void) | null = null;
    listenMock.mockImplementation(async (eventName: string, cb: (event: { payload: LogEntry[] }) => void) => {
      if (eventName === LOG_BATCH_EVENT) batchListener = cb;
      return () => undefined;
    });

    const { container } = renderLogPage();
    await screen.findByText('scroll-msg-1');
    await waitFor(() => expect(batchListener).not.toBeNull());

    const scrollEl = container.querySelector('.log-page') as HTMLElement;
    expect(scrollEl).toBeTruthy();
    // 模拟用户向下滚动查看旧日志：scrollTop=400 > 24 → atTop=false
    Object.defineProperties(scrollEl, {
      scrollHeight: { configurable: true, value: 1000 },
      clientHeight: { configurable: true, value: 200 },
      scrollTop: { configurable: true, value: 400, writable: true },
    });
    fireEvent.scroll(scrollEl);

    // 推送新批次
    await act(async () => {
      batchListener!({ payload: [makeEntry(2)] });
    });
    await screen.findByText('scroll-msg-2');

    // 用户查看旧日志时，新日志到达不应强制把 scrollTop 拉回顶部
    expect(scrollEl.scrollTop).toBe(400);
  });

  it('unsubscribes the batch listener on unmount', async () => {
    const unlistenSpy = vi.fn();
    listenMock.mockImplementation(async (eventName: string) => {
      if (eventName === LOG_BATCH_EVENT) return unlistenSpy;
      return () => undefined;
    });

    const { unmount } = renderLogPage();
    await waitFor(() => expect(listenMock).toHaveBeenCalledWith(LOG_BATCH_EVENT, expect.any(Function)));

    unmount();
    await waitFor(() => expect(unlistenSpy).toHaveBeenCalled());
  });

  it('renders interactive sample logs without Tauri internals', () => {
    // 纯 Web 预览（无 Tauri 运行时）：复用正式日志页结构并使用本地示例数据。
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    try {
      const { container } = renderLogPage();
      expect(screen.getByRole('heading', { name: '日志与诊断' })).toBeInTheDocument();
      expect(screen.getByText('当前显示用于调试界面和交互的示例日志。')).toBeInTheDocument();
      expect(screen.getByText('设备会话已恢复，实时读数已更新')).toBeInTheDocument();
      expect(container.querySelector('.log-status-buffer')).toHaveTextContent('10');
      expect(container.querySelector('.log-status-buffer')).not.toHaveTextContent('内存');
      expect(container.querySelector('.log-status-disk')).toHaveTextContent('7.0 MB');
      expect(container.querySelector('.log-status-disk')).not.toHaveTextContent('磁盘');
      expect(container.querySelector('.log-status-disk-quota')).not.toBeInTheDocument();
      expect(screen.getByText('当前 8 条')).toBeInTheDocument();
      fireEvent.click(screen.getByRole('button', { name: '暂停刷新' }));
      expect(within(container.querySelector('.log-toolbar-divider') as HTMLElement)
        .getByRole('button', { name: '有 3 条新日志' })).toBeInTheDocument();
      fireEvent.click(screen.getByRole('button', { name: '继续刷新' }));
      expect(screen.getByText('当前 8 条')).toBeInTheDocument();
      expect(screen.getByRole('combobox', { name: '来源' })).toBeInTheDocument();
      fireEvent.change(screen.getByRole('combobox', { name: '来源' }), { target: { value: 'plugin' } });
      expect(screen.getByText('当前 2 条')).toBeInTheDocument();
      fireEvent.change(screen.getByRole('combobox', { name: '来源' }), { target: { value: 'all' } });
      const expandButtons = screen.getAllByRole('button', { name: '展开详情' });
      expect(expandButtons.length).toBeGreaterThan(0);
      fireEvent.click(expandButtons[0]);
      expect(screen.getByText('device::session')).toBeInTheDocument();
      expect(screen.getByText('device-7f2a31c4')).toBeInTheDocument();
      expect(screen.getByText('无线')).toBeInTheDocument();
      expect(invokeMock.mock.calls.some(([command]) => command === 'log_query')).toBe(false);
    } finally {
      Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} });
    }
  });

  it('shows the new-logs badge during pause and clears it on resume', async () => {
    let batchListener: ((event: { payload: LogEntry[] }) => void) | null = null;
    listenMock.mockImplementation(async (eventName: string, cb: (event: { payload: LogEntry[] }) => void) => {
      if (eventName === LOG_BATCH_EVENT) batchListener = cb;
      return () => undefined;
    });

    const { container } = renderLogPage();
    await screen.findByText('没有符合条件的日志');
    await waitFor(() => expect(batchListener).not.toBeNull());

    // 暂停刷新（只暂停界面合并，不停止后端采集）
    fireEvent.click(screen.getByRole('button', { name: '暂停刷新' }));

    // 推送一批实时日志：暂停期间应缓冲并计数，徽章提示「有 2 条新日志」
    const batch: LogEntry[] = [
      { id: 1, timestamp: '2026-07-17T10:00:00+08:00', level: 'info', source: 'app', target: 'm', message: 'paused-1', sessionId: 's1', fields: {} },
      { id: 2, timestamp: '2026-07-17T10:00:01+08:00', level: 'info', source: 'app', target: 'm', message: 'paused-2', sessionId: 's1', fields: {} },
    ];
    await act(async () => {
      batchListener!({ payload: batch });
    });

    // 暂停期间徽章出现
    const divider = container.querySelector('.log-toolbar-divider');
    expect(divider).toBeTruthy();
    expect(within(divider as HTMLElement).getByRole('button', { name: '有 2 条新日志' })).toBeInTheDocument();
    expect(within(divider as HTMLElement).queryByText(/当前 \d+ 条/)).not.toBeInTheDocument();
    // 暂停期间日志被缓冲，不进入列表
    expect(screen.queryByText('paused-1')).not.toBeInTheDocument();

    // 恢复：合并暂停期间的新日志，徽章消失
    fireEvent.click(screen.getByRole('button', { name: '继续刷新' }));
    await waitFor(() => expect(screen.queryByRole('button', { name: '有 2 条新日志' })).not.toBeInTheDocument());
    expect(screen.getByText('paused-1')).toBeInTheDocument();
    expect(screen.getByText('paused-2')).toBeInTheDocument();
  });

  it('shows the new-logs badge when scrolled up (not paused) and clears on click', async () => {
    const makeEntry = (id: number): LogEntry => ({
      id, timestamp: `2026-07-17T10:00:0${id}+08:00`, level: 'info', source: 'app',
      target: 'm', message: `scroll-msg-${id}`, sessionId: 's1', fields: {},
    });
    invokeMock.mockImplementation(makeInvokeImpl({ entries: [makeEntry(1)] }));
    let batchListener: ((event: { payload: LogEntry[] }) => void) | null = null;
    listenMock.mockImplementation(async (eventName: string, cb: (event: { payload: LogEntry[] }) => void) => {
      if (eventName === LOG_BATCH_EVENT) batchListener = cb;
      return () => undefined;
    });

    const { container } = renderLogPage();
    await screen.findByText('scroll-msg-1');
    await waitFor(() => expect(batchListener).not.toBeNull());

    const scrollEl = container.querySelector('.log-page') as HTMLElement;
    expect(scrollEl).toBeTruthy();
    // 模拟用户向下滚动查看旧日志：scrollTop=400 > 24 → atTop=false
    Object.defineProperties(scrollEl, {
      scrollHeight: { configurable: true, value: 1000 },
      clientHeight: { configurable: true, value: 200 },
      scrollTop: { configurable: true, value: 400, writable: true },
    });
    fireEvent.scroll(scrollEl);

    // 推送新批次（非暂停）：日志仍进入列表，但不强制滚动，徽章提示「有 1 条新日志」
    await act(async () => {
      batchListener!({ payload: [makeEntry(2)] });
    });

    const badge = await screen.findByRole('button', { name: '有 1 条新日志' });
    expect(badge).toBeInTheDocument();

    // 点击入口后滚到最新并恢复跟随：计数清零，徽章消失
    fireEvent.click(badge);
    await waitFor(() => expect(screen.queryByRole('button', { name: '有 1 条新日志' })).not.toBeInTheDocument());
  });
});
