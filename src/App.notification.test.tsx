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

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

beforeEach(() => {
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
  await screen.findByLabelText('当前 DPI：1000，点击编辑');
  return result;
}

/// 等待通知出现并返回通知元素。
async function dispatchAndWait(action: Parameters<typeof notifyInfo>[2]) {
  notifyInfo('更新提示', '发现新版本', action);
  return screen.findByRole('status');
}

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
