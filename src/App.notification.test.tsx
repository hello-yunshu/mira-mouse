// SPDX-License-Identifier: AGPL-3.0-or-later
// ITERATION-009 §4.1 / §4.2：通知 action 完整链路测试。
// 覆盖：
// - Local AI toast → settings local AI section
// - Plugin toast → settings plugin section
// - App update toast → About update
// - Battery toast → battery usage
// - Relaunch → relaunch
// - close button → only close
// - modal open → action disabled
// 验证 §4.2 方案 B：macOS 伪点击机制已移除（窗口聚焦不再消费 pending action）。
import { act, fireEvent, render, screen, cleanup, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import { OVERLAY_ROOT_ID } from './overlay';
import { notifyInfo } from './notify';

const { eventCallbacks, invokeMock, listenMock } = vi.hoisted(() => ({
  eventCallbacks: new Map<string, (event: { payload: unknown }) => void>(),
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }));

beforeEach(() => {
  eventCallbacks.clear();
  listenMock.mockImplementation(async (
    eventName: string,
    callback: (event: { payload: unknown }) => void,
  ) => {
    eventCallbacks.set(eventName, callback);
    return () => eventCallbacks.delete(eventName);
  });
  invokeMock.mockRejectedValue(new Error('not mocked'));
  // About 页面在非 preview 模式下调用 about_info；mock 为有效响应避免 error 分支。
  invokeMock.mockImplementation((command: string) => {
    if (command === 'about_info') {
      return Promise.resolve({
        name: 'Mira Mouse',
        version: '0.1.0-test',
        identifier: 'app.mira.test',
        platform: 'test',
        architecture: 'test',
        rustVersion: 'test',
        buildDate: 'test',
        gitCommit: 'test',
        bundledPlugins: [],
        contact: {
          github: 'https://github.com/hello-yunshu',
          repository: 'https://github.com/hello-yunshu/mira-mouse',
          x: 'https://x.com/yunyunshu',
          telegram: 'https://t.me/yunyunshu',
          developerName: 'test',
          copyright: 'test',
        },
        updaterActive: false,
      });
    }
    if (command === 'relaunch_app') {
      return Promise.resolve();
    }
    return Promise.reject(new Error('not mocked'));
  });
});

afterEach(() => {
  eventCallbacks.clear();
  listenMock.mockReset();
  invokeMock.mockReset();
  window.history.pushState({}, '', '/');
  cleanup();
  document.getElementById(OVERLAY_ROOT_ID)?.remove();
  document.getElementById('root')?.remove();
});

/// 把 App 渲染到一个 id="root" 的容器中，让 Modal 的 inert 逻辑能找到背景根。
function renderAppInRoot() {
  const rootDiv = document.createElement('div');
  rootDiv.id = 'root';
  document.body.appendChild(rootDiv);
  const result = render(<App />, { container: rootDiv });
  return { ...result, rootDiv };
}

/// 进入演示模式并等待 dashboard 就绪。
async function enterDemoMode() {
  const result = renderAppInRoot();
  fireEvent.click(screen.getByText('查看演示'));
  // 等待 DPI 编辑按钮出现，标志 dashboard 已渲染完成
  await screen.findByLabelText('当前 DPI：1600，点击编辑');
  return result;
}

/// 等待通知出现并返回通知元素。
async function dispatchAndWait(action: Parameters<typeof notifyInfo>[2]) {
  notifyInfo('更新提示', '发现新版本', action);
  return screen.findByRole('status');
}

async function emitTauriEvent(eventName: string) {
  await waitFor(() => expect(eventCallbacks.has(eventName)).toBe(true));
  act(() => eventCallbacks.get(eventName)?.({ payload: null }));
}

describe('托盘导航状态重置', () => {
  it('Windows 式延迟 IPC 下从托盘打开关于仍立即显示完整页面骨架', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'about_info') return new Promise(() => {});
      return Promise.reject(new Error('not mocked'));
    });
    await enterDemoMode();

    await emitTauriEvent('navigate-about');

    expect(screen.getByRole('heading', { name: '关于' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '返回' })).toBeInTheDocument();
    expect(screen.getByLabelText('加载中…')).toHaveAttribute('aria-busy', 'true');
    expect(document.querySelector('.about-loading-content')).toBeInTheDocument();
  });

  it('从电量弹窗打开关于时先关闭电量弹窗', async () => {
    await enterDemoMode();
    await emitTauriEvent('open-battery-usage');
    await screen.findByRole('dialog', { name: '电量使用情况' });

    await emitTauriEvent('navigate-about');

    await waitFor(() => {
      expect(screen.queryByRole('dialog', { name: '电量使用情况' })).not.toBeInTheDocument();
      expect(screen.getByRole('button', { name: '关于 Mira' })).toHaveClass('nav-link nav-about active');
    });
  });

  it('从关于页点击打开 Mira 时返回设备首页', async () => {
    await enterDemoMode();
    await emitTauriEvent('navigate-about');
    await waitFor(() => {
      expect(screen.getByRole('button', { name: '关于 Mira' })).toHaveClass('nav-link nav-about active');
    });

    await emitTauriEvent('navigate-dashboard');

    await waitFor(() => {
      expect(screen.getByRole('button', { name: '设备' })).toHaveClass('nav-link active');
      expect(screen.getByRole('button', { name: '关于 Mira' })).not.toHaveClass('active');
    });
  });

  it('已经在首页时点击打开 Mira 也会关闭页面内弹窗', async () => {
    await enterDemoMode();
    fireEvent.click(screen.getByRole('button', { name: '全部读数' }));
    await screen.findByRole('dialog', { name: '全部读数' });

    await emitTauriEvent('navigate-dashboard');

    await waitFor(() => {
      expect(screen.queryByRole('dialog', { name: '全部读数' })).not.toBeInTheDocument();
      expect(screen.getByRole('button', { name: '设备' })).toHaveClass('nav-link active');
    });
  });

  it('页面内弹窗打开时从托盘进入电量不会叠加双 Modal', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      await enterDemoMode();
      fireEvent.click(screen.getByRole('button', { name: '全部读数' }));
      await screen.findByRole('dialog', { name: '全部读数' });

      await emitTauriEvent('open-battery-usage');

      await screen.findByRole('dialog', { name: '电量使用情况' });
      expect(screen.queryByRole('dialog', { name: '全部读数' })).not.toBeInTheDocument();
      expect(screen.getAllByRole('dialog')).toHaveLength(1);
      expect(document.getElementById('root')).toHaveAttribute('inert');
      expect(document.getElementById('root')).toHaveAttribute('aria-hidden', 'true');
      expect(warn).not.toHaveBeenCalledWith('[Mira Overlay] Multiple modal layers are open simultaneously.');
    } finally {
      warn.mockRestore();
    }
  });

  it('托盘动作会关闭自定义电量 Popover 且不会在 Modal 关闭后复现', async () => {
    await enterDemoMode();
    fireEvent.click(document.querySelector('.battery-state') as HTMLButtonElement);
    await screen.findByRole('region', { name: '设备电量' });

    await emitTauriEvent('open-battery-usage');
    await screen.findByRole('dialog', { name: '电量使用情况' });
    expect(screen.queryByRole('region', { name: '设备电量' })).not.toBeInTheDocument();

    fireEvent.click(document.querySelector('.battery-usage-close-icon') as HTMLButtonElement);
    await waitFor(() => {
      expect(screen.queryByRole('dialog', { name: '电量使用情况' })).not.toBeInTheDocument();
      expect(screen.queryByRole('region', { name: '设备电量' })).not.toBeInTheDocument();
    });
  });

  it('页面导航期间托盘监听保持单次订阅', async () => {
    await enterDemoMode();
    await waitFor(() => expect(eventCallbacks.has('navigate-dashboard')).toBe(true));
    const subscriptionCount = listenMock.mock.calls.length;

    await emitTauriEvent('navigate-about');
    await waitFor(() => {
      expect(screen.getByRole('button', { name: '关于 Mira' })).toHaveClass('active');
    });
    await emitTauriEvent('navigate-dashboard');

    expect(listenMock).toHaveBeenCalledTimes(subscriptionCount);
  });
});

describe('ITERATION-009 §4.1: 通知 action 路由', () => {
  it('Local AI toast → settings local AI section', async () => {
    await enterDemoMode();
    const notification = await dispatchAndWait('settings-local-ai-update');
    fireEvent.click(notification);
    // 切到 Settings 页
    await waitFor(() => {
      expect(screen.getByRole('button', { name: '设置' })).toHaveClass('nav-link active');
    });
    // 滚动目标元素存在（id 由 Settings.tsx focus effect 渲染）
    await waitFor(() => {
      expect(document.getElementById('settings-local-ai-section')).not.toBeNull();
    });
  });

  it('Plugin toast → settings plugin section', async () => {
    await enterDemoMode();
    const notification = await dispatchAndWait('settings-plugin-update');
    fireEvent.click(notification);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: '设置' })).toHaveClass('nav-link active');
    });
    await waitFor(() => {
      expect(document.getElementById('settings-plugin-update-section')).not.toBeNull();
    });
  });

  it('App update toast → About update', async () => {
    await enterDemoMode();
    const notification = await dispatchAndWait('about-update');
    fireEvent.click(notification);
    // 切到 About 页（nav-about 按钮变为 active）
    await waitFor(() => {
      expect(screen.getByRole('button', { name: '关于 Mira' })).toHaveClass('nav-link nav-about active');
    });
    // 滚动目标元素存在
    await waitFor(() => {
      expect(document.getElementById('about-update-section')).not.toBeNull();
    });
  });

  it('Battery toast → battery usage', async () => {
    await enterDemoMode();
    const notification = await dispatchAndWait('battery-usage');
    fireEvent.click(notification);
    // BatteryUsageModal 打开
    await screen.findByRole('dialog', { name: '电量使用情况' });
  });

  it('Relaunch → relaunch', async () => {
    await enterDemoMode();
    const notification = await dispatchAndWait('relaunch');
    fireEvent.click(notification);
    // relaunchAfterUpdate 内部调用自定义的 relaunch_app 命令
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('relaunch_app');
    });
  });

  it('close button → only close, no navigation', async () => {
    await enterDemoMode();
    const notification = await dispatchAndWait('about-update');
    // 当前在 dashboard
    expect(screen.getByRole('button', { name: '设备' })).toHaveClass('nav-link active');
    // 点击关闭按钮
    const closeBtn = notification.querySelector('button[aria-label="关闭通知"]') as HTMLButtonElement;
    expect(closeBtn).not.toBeNull();
    fireEvent.click(closeBtn);
    // 通知消失
    await waitFor(() => {
      expect(screen.queryByRole('status')).not.toBeInTheDocument();
    });
    // 仍在 dashboard，未跳转到 About
    expect(screen.getByRole('button', { name: '设备' })).toHaveClass('nav-link active');
    expect(screen.getByRole('button', { name: '关于 Mira' })).not.toHaveClass('active');
  });

  it('modal open → action disabled, click does not navigate', async () => {
    await enterDemoMode();
    // 打开 DeviceDetails Modal
    fireEvent.click(screen.getByRole('button', { name: '全部读数' }));
    await screen.findByRole('dialog', { name: '全部读数' });
    // 发送带跳转动作的通知
    notifyInfo('更新提示', '发现新版本', 'about-update');
    const notification = await screen.findByRole('status');
    expect(notification.hasAttribute('data-action-disabled')).toBe(true);
    // 点击通知不应触发跳转
    fireEvent.click(notification);
    // Modal 仍在（未跳转），About 页面未渲染（无 about-update-section）
    expect(screen.getByRole('dialog', { name: '全部读数' })).toBeInTheDocument();
    expect(document.getElementById('about-update-section')).toBeNull();
  });
});

describe('ITERATION-009 §4.2: macOS 伪点击机制已移除', () => {
  it('窗口聚焦不再触发 take_pending_notification_action 调用', async () => {
    await enterDemoMode();
    // 派发一条带 action 的通知
    notifyInfo('更新提示', '发现新版本', 'about-update');
    await screen.findByRole('status');
    // 模拟窗口聚焦事件
    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    // 不应调用 take_pending_notification_action（命令已被移除）
    expect(invokeMock).not.toHaveBeenCalledWith('take_pending_notification_action');
  });
});
