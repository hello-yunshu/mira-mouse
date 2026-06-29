// SPDX-License-Identifier: AGPL-3.0-or-later
import { fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import { notifyError } from './notify';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

const originalUserAgent = navigator.userAgent;

beforeEach(() => {
  invokeMock.mockRejectedValue(new Error('not mocked'));
});

afterEach(() => {
  invokeMock.mockReset();
  window.history.pushState({}, '', '/');
  Object.defineProperty(navigator, 'userAgent', { configurable: true, value: originalUserAgent });
});

describe('Mira shell', () => {
  it('shows foreground errors inside the app and lets the user dismiss them', async () => {
    render(<App />);
    notifyError('刷新失败', '设备暂时不可用');
    expect(await screen.findByRole('alert')).toHaveTextContent('刷新失败设备暂时不可用');
    fireEvent.click(screen.getByRole('button', { name: '关闭通知' }));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });
  it('shows a quiet no-device state without stale numbers', () => {
    render(<App />);
    expect(screen.getByText('还没找到支持的鼠标呢')).toBeInTheDocument();
    expect(screen.queryByText(/0 DPI|--%/)).not.toBeInTheDocument();
  });
  it('shows native-style window controls in the Windows web preview', () => {
    Object.defineProperty(navigator, 'userAgent', { configurable: true, value: 'Linux jsdom' });
    window.history.pushState({}, '', '?platform=windows');
    render(<App />);
    const controls = document.querySelector('.windows-preview-controls') as HTMLElement;
    expect(controls).toBeInTheDocument();
    expect(document.querySelector('.windows-window-controls')).not.toBeInTheDocument();
    expect(within(controls).getByRole('button', { name: '最小化窗口' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '最大化窗口' })).not.toBeInTheDocument();
    expect(within(controls).getByRole('button', { name: '关闭窗口' })).toBeInTheDocument();
  });
  it('hides to tray from the Windows close control and keeps maximize absent', () => {
    Object.defineProperty(navigator, 'userAgent', { configurable: true, value: 'Windows' });
    render(<App />);
    expect(document.querySelector('.windows-drag-strip')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '最大化窗口' })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '关闭窗口' }));
    expect(invokeMock).toHaveBeenCalledWith('hide_to_tray');
  });
  it('renders capability data and labels the application-layer link', () => {
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));
    expect(document.documentElement.style.getPropertyValue('--accent')).toBe('#ffb3b3');
    expect(screen.getAllByText('82%')).toHaveLength(2);
    expect(screen.getByLabelText('当前 DPI：1000，点击编辑')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    expect(screen.queryByText('fixture-verified')).not.toBeInTheDocument();
    expect(document.querySelector('[data-animation="realtime-deformation"]')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '全部读数' }));
    expect(screen.getByRole('dialog', { name: '全部读数' })).toBeInTheDocument();
    expect(screen.getByText('传感器与连接')).toBeInTheDocument();
    expect(screen.getByText('按键映射')).toBeInTheDocument();
    expect(screen.getByText('接收器灯光固件')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '关闭读数详情' }));
    expect(screen.queryByRole('dialog', { name: '全部读数' })).not.toBeInTheDocument();
  });
  it('shows the multi-mouse switcher in the demo fixture', () => {
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));
    expect(screen.getByRole('heading', { name: 'Mira Example Wireless Mouse' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '切换鼠标' }));
    fireEvent.click(screen.getByText('Mira Example USB Mouse').closest('button')!);
    expect(screen.getByRole('heading', { name: 'Mira Example USB Mouse' })).toBeInTheDocument();
    expect(screen.getByLabelText('当前 DPI：1600，点击编辑')).toBeInTheDocument();
  });
  it('returns to the dashboard when exiting demo mode from another page', () => {
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    expect(screen.getByRole('heading', { name: '设置' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '退出演示' }));
    expect(screen.getByText('还没找到支持的鼠标呢')).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: '设置' })).not.toBeInTheDocument();
  });
});
