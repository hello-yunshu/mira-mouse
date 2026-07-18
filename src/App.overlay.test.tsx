// SPDX-License-Identifier: AGPL-3.0-or-later
// 验证统一浮层架构：所有 Modal / 通知挂到 #mira-overlay-root，
// 不再受业务组件层叠上下文影响；Modal 打开期间 #root 被 inert 冻结；
// 通知在 Modal 之上但仍受 hasOpenModal() 约束。
import { act, fireEvent, render, screen, cleanup } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import { OVERLAY_ROOT_ID } from './overlay';
import { notifyInfo } from './notify';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

beforeEach(() => {
  invokeMock.mockRejectedValue(new Error('not mocked'));
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

describe('Overlay 架构集成', () => {
  it('DeviceDetails 弹窗渲染到 #mira-overlay-root 而非 #root', async () => {
    const { rootDiv } = await enterDemoMode();
    fireEvent.click(screen.getByRole('button', { name: '全部读数' }));

    const dialog = await screen.findByRole('dialog', { name: '全部读数' });
    const overlayRoot = document.getElementById(OVERLAY_ROOT_ID);
    expect(overlayRoot).not.toBeNull();
    expect(overlayRoot?.contains(dialog)).toBe(true);
    // 弹窗不在 #root 内（脱离业务层叠上下文）
    expect(rootDiv.contains(dialog)).toBe(false);
  });

  it('Modal 打开期间 #root 设置 inert 与 aria-hidden', async () => {
    const { rootDiv } = await enterDemoMode();
    fireEvent.click(screen.getByRole('button', { name: '全部读数' }));
    await screen.findByRole('dialog', { name: '全部读数' });

    expect(rootDiv.hasAttribute('inert')).toBe(true);
    expect(rootDiv.getAttribute('aria-hidden')).toBe('true');
  });

  it('Modal 关闭后 #root 的 inert 与 aria-hidden 被清理', async () => {
    const { rootDiv } = await enterDemoMode();
    fireEvent.click(screen.getByRole('button', { name: '全部读数' }));
    await screen.findByRole('dialog', { name: '全部读数' });
    expect(rootDiv.hasAttribute('inert')).toBe(true);

    fireEvent.click(screen.getByRole('button', { name: '关闭读数详情' }));
    // Modal 卸载后，cleanup 在 effect 中恢复，等待 effect 提交
    await act(async () => {
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    });
    expect(screen.queryByRole('dialog', { name: '全部读数' })).not.toBeInTheDocument();
    expect(rootDiv.hasAttribute('inert')).toBe(false);
    expect(rootDiv.hasAttribute('aria-hidden')).toBe(false);
  });

  it('通知渲染到 #mira-overlay-root', async () => {
    const { rootDiv } = renderAppInRoot();
    notifyInfo('更新提示', '发现新版本', 'about-update');
    const alert = await screen.findByRole('status');
    const overlayRoot = document.getElementById(OVERLAY_ROOT_ID);
    expect(overlayRoot).not.toBeNull();
    expect(overlayRoot?.contains(alert)).toBe(true);
    expect(rootDiv.contains(alert)).toBe(false);
  });

  it('Modal 打开期间通知的跳转行为被禁用', async () => {
    await enterDemoMode();
    // 打开 DeviceDetails Modal
    fireEvent.click(screen.getByRole('button', { name: '全部读数' }));
    await screen.findByRole('dialog', { name: '全部读数' });
    // 此时发送一个带跳转动作的通知
    notifyInfo('更新提示', '发现新版本', 'about-update');
    const notification = await screen.findByRole('status');
    // 通知仍可见
    expect(notification).toBeInTheDocument();
    // 但跳转被禁用
    expect(notification.hasAttribute('data-action-disabled')).toBe(true);
    // 点击通知不应触发跳转（onClick 为 undefined）
    fireEvent.click(notification);
    // Modal 仍在，说明没有发生导航跳转
    expect(screen.getByRole('dialog', { name: '全部读数' })).toBeInTheDocument();
  });

  it('Modal 关闭后通知的跳转行为恢复', async () => {
    await enterDemoMode();
    // 打开再关闭 Modal
    fireEvent.click(screen.getByRole('button', { name: '全部读数' }));
    await screen.findByRole('dialog', { name: '全部读数' });
    fireEvent.click(screen.getByRole('button', { name: '关闭读数详情' }));
    await act(async () => {
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    });
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();

    // 发送通知，此时无 Modal，跳转应可用
    notifyInfo('更新提示', '发现新版本', 'about-update');
    const notification = await screen.findByRole('status');
    expect(notification.hasAttribute('data-action-disabled')).toBe(false);
  });

  it('BatteryUsageModal 渲染到 #mira-overlay-root', async () => {
    const { rootDiv } = await enterDemoMode();
    fireEvent.click(document.querySelector('.battery-state') as HTMLButtonElement);
    fireEvent.click(screen.getByRole('button', { name: '查看用电趋势' }));

    const dialog = await screen.findByRole('dialog');
    const overlayRoot = document.getElementById(OVERLAY_ROOT_ID);
    expect(overlayRoot).not.toBeNull();
    expect(overlayRoot?.contains(dialog)).toBe(true);
    expect(rootDiv.contains(dialog)).toBe(false);
  });

  it('首页电量 popover 渲染到 #mira-overlay-root，避免 DPI 合成层穿透', async () => {
    const { rootDiv } = await enterDemoMode();
    fireEvent.click(document.querySelector('.battery-state') as HTMLButtonElement);

    const popover = await screen.findByRole('region', { name: '设备电量' });
    const overlayRoot = document.getElementById(OVERLAY_ROOT_ID);
    expect(overlayRoot).not.toBeNull();
    expect(overlayRoot?.contains(popover)).toBe(true);
    expect(rootDiv.contains(popover)).toBe(false);
  });
});
