// SPDX-License-Identifier: AGPL-3.0-or-later
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { BatteryUsageModal } from './BatteryUsage';
import type { BatteryHistoryResponse } from './types';
import { MOCK_BATTERY_HISTORY_24H, MOCK_BATTERY_HISTORY_10D } from './mock';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn().mockResolvedValue(null) }));
vi.mock('./notify', () => ({
  notifyError: vi.fn(),
  notifyInfo: vi.fn(),
  notifySuccess: vi.fn(),
}));

const settingsEnabled = { batteryHistoryEnabled: true };
const settingsDisabled = { batteryHistoryEnabled: false };

const emptyResponse: BatteryHistoryResponse = {
  range: '24h',
  devices: [],
  series: [],
  insights: [],
  generatedAt: new Date().toISOString(),
};

function mockInvoke(opts: {
  settings?: Record<string, unknown>;
  response?: BatteryHistoryResponse;
} = {}) {
  const settings = opts.settings ?? settingsEnabled;
  const response = opts.response ?? MOCK_BATTERY_HISTORY_24H;
  invokeMock.mockImplementation((command: string) => {
    if (command === 'settings_get') return Promise.resolve(settings);
    if (command === 'battery_history_get') return Promise.resolve(response);
    if (command === 'battery_history_clear') return Promise.resolve(undefined);
    if (command === 'battery_history_export') return Promise.resolve('');
    return Promise.resolve(undefined);
  });
}

describe('BatteryUsageModal', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('shows disabled state when history is disabled', async () => {
    mockInvoke({ settings: settingsDisabled });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('settings_get'));
    await waitFor(() => {
      expect(screen.getByText('电量使用情况已关闭')).toBeInTheDocument();
    });
  });

  it('shows empty state when no devices in response', async () => {
    mockInvoke({ response: emptyResponse });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('battery_history_get', { range: '24h' }));
    await waitFor(() => {
      expect(screen.getByText('还没有足够的电量记录')).toBeInTheDocument();
    });
  });

  it('renders chart with follow-up information blocks for 24h data', async () => {
    mockInvoke({ response: MOCK_BATTERY_HISTORY_24H });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('battery_history_get', { range: '24h' }));
    expect(screen.getByRole('heading', { name: '电量使用情况' })).toBeInTheDocument();
    expect(screen.getByText('本地 AI')).toBeInTheDocument();
    expect(screen.getByText('由趋势建模、异常掉电检测与充电习惯推断生成')).toBeInTheDocument();
    const range24h = screen.getByRole('tab', { name: '24 小时' });
    expect(range24h).toHaveAttribute('aria-selected', 'true');
    const chartCard = document.querySelector('.battery-chart-card');
    const summaryGrid = document.querySelector('.battery-summary-grid');
    expect(chartCard).not.toBeNull();
    expect(summaryGrid).not.toBeNull();
    if (!chartCard || !summaryGrid) throw new Error('battery chart or summary grid missing');
    expect(chartCard.compareDocumentPosition(summaryGrid) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.getByText('当前电量')).toBeInTheDocument();
    expect(screen.getByText('充电习惯')).toBeInTheDocument();
  });

  it('switches to 10d range and refetches', async () => {
    let callCount = 0;
    invokeMock.mockImplementation((command: string, payload?: { range?: string }) => {
      if (command === 'settings_get') return Promise.resolve(settingsEnabled);
      if (command === 'battery_history_get') {
        callCount += 1;
        return Promise.resolve(payload?.range === '10d' ? MOCK_BATTERY_HISTORY_10D : MOCK_BATTERY_HISTORY_24H);
      }
      return Promise.resolve(undefined);
    });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(callCount).toBe(1));
    const range10d = screen.getByRole('tab', { name: '10 天' });
    fireEvent.click(range10d);
    await waitFor(() => expect(callCount).toBe(2));
    expect(invokeMock).toHaveBeenCalledWith('battery_history_get', { range: '10d' });
  });

  it('switches selected device from the status strip menu', async () => {
    mockInvoke({ response: MOCK_BATTERY_HISTORY_24H });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('battery_history_get', { range: '24h' }));
    const switcher = screen.getByRole('button', { name: '切换设备' });
    fireEvent.click(switcher);
    const receiverItem = screen.getByRole('menuitemradio', { name: /接收器/ });
    expect(receiverItem).toHaveAttribute('aria-checked', 'false');
    fireEvent.click(receiverItem);
    await waitFor(() => expect(document.querySelector('.battery-status-metric strong')).toHaveTextContent('96%'));
  });

  it('shows unsupported state when device has no battery', async () => {
    mockInvoke();
    render(<BatteryUsageModal open onClose={() => {}} hasBattery={false} />);
    await waitFor(() => {
      expect(screen.getByText('这台设备暂未报告电量')).toBeInTheDocument();
    });
  });

  it('calls clear and reloads', async () => {
    let clearCount = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settingsEnabled);
      if (command === 'battery_history_get') return Promise.resolve(MOCK_BATTERY_HISTORY_24H);
      if (command === 'battery_history_clear') {
        clearCount += 1;
        return Promise.resolve(undefined);
      }
      return Promise.resolve(undefined);
    });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('battery_history_get', { range: '24h' }));
    // 点击清除
    const clearBtn = screen.getByRole('button', { name: '清除电量历史' });
    fireEvent.click(clearBtn);
    // 确认
    const confirmBtn = await screen.findByRole('button', { name: '确认清除' });
    fireEvent.click(confirmBtn);
    await waitFor(() => expect(clearCount).toBe(1));
    expect(invokeMock).toHaveBeenCalledWith('battery_history_clear', { deviceKey: 'mouse:abc123:mouse' });
  });

  it('returns null when closed', () => {
    const { container } = render(<BatteryUsageModal open={false} onClose={() => {}} hasBattery />);
    expect(container.firstChild).toBeNull();
  });
});
