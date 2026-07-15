// SPDX-License-Identifier: AGPL-3.0-or-later
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { BatteryUsageModal } from './BatteryUsage';
import type { BatteryHistoryResponse } from './types';
import { MOCK_BATTERY_HISTORY_24H, MOCK_BATTERY_HISTORY_10D } from './mock';
import i18n from './i18n';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn().mockResolvedValue(null) }));
vi.mock('./notify', () => ({
  notifyError: vi.fn(),
  notifyInfo: vi.fn(),
  notifySuccess: vi.fn(),
}));

const settingsEnabled = {
  batteryHistoryEnabled: true,
  localAiAnalysisEnabled: true,
  localAiFeatures: { batteryUsage: true },
};
const settingsDisabled = {
  batteryHistoryEnabled: false,
  localAiAnalysisEnabled: false,
  localAiFeatures: { batteryUsage: true },
};

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
  beforeEach(async () => {
    invokeMock.mockReset();
    await i18n.changeLanguage('zh-CN');
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

  it('hides AI labels when the battery feature scope is off even if the engine is on', async () => {
    mockInvoke({
      settings: {
        batteryHistoryEnabled: true,
        localAiAnalysisEnabled: true,
        localAiFeatures: { batteryUsage: false },
      },
      response: MOCK_BATTERY_HISTORY_24H,
    });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('battery_history_get', { range: '24h' }));

    expect(screen.queryByText('本地 AI')).toBeNull();
    expect(screen.queryByText('本地 AI 洞察')).toBeNull();
    expect(screen.getByText('用电洞察')).toBeInTheDocument();
    expect(screen.getByText('根据本地电量历史生成趋势、耗电与充电习惯摘要')).toBeInTheDocument();
  });

  it('uses caller-provided settings so an already-open settings page cannot drift from the modal', async () => {
    mockInvoke({
      settings: {
        batteryHistoryEnabled: true,
        localAiAnalysisEnabled: false,
        localAiFeatures: { batteryUsage: true },
      },
      response: MOCK_BATTERY_HISTORY_24H,
    });
    render(
      <BatteryUsageModal
        open
        onClose={() => {}}
        hasBattery
        batteryHistoryEnabled
        aiAnalysisEnabled
      />,
    );
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('battery_history_get', { range: '24h' }));

    expect(invokeMock).not.toHaveBeenCalledWith('settings_get');
    expect(screen.getByText('本地 AI')).toBeInTheDocument();
    expect(screen.getByText('本地 AI 洞察')).toBeInTheDocument();
  });

  it('hides the insight heading when no insight cards are rendered', async () => {
    mockInvoke({
      response: {
        ...MOCK_BATTERY_HISTORY_24H,
        insights: [
          {
            type: 'chargingHabit',
            severity: 'info',
            title: 'chargingHabit',
            message: 'chargingHabitStartEnd|18|92|3',
            deviceKey: 'mouse:abc123:mouse',
          },
        ],
      },
    });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('battery_history_get', { range: '24h' }));

    expect(document.querySelector('.battery-insight-card')).toBeNull();
    expect(screen.queryByText('本地 AI 洞察')).toBeNull();
    expect(screen.queryByText('由趋势建模、异常掉电检测与充电习惯推断生成')).toBeNull();
  });

  it('renders the four summary blocks in one row above the 24h chart', async () => {
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
    expect(document.querySelectorAll('.battery-chart g[role="button"]')).toHaveLength(48);
    const usageGridLines = Array.from(document.querySelectorAll<SVGLineElement>('.battery-chart-x-grid'));
    const usageExtensions = Array.from(document.querySelectorAll<SVGLineElement>('.battery-chart-x-extension'));
    const usageLabels = Array.from(document.querySelectorAll<SVGTextElement>('.battery-chart-x-label'));
    expect(usageGridLines).toHaveLength(8);
    expect(usageExtensions).toHaveLength(8);
    expect(usageLabels).toHaveLength(8);
    expect(usageGridLines.every((line, index) => line.getAttribute('x1') === usageLabels[index].getAttribute('x'))).toBe(true);
    expect(usageGridLines.every((line) => line.getAttribute('clip-path') === 'url(#battery-chart-plot-clip)')).toBe(true);
    expect(usageExtensions.every((line) => line.getAttribute('y1') === '144' && line.getAttribute('y2') === '159')).toBe(true);
    const usageLabelText = usageLabels.map((label) => label.textContent);
    expect(usageLabelText).toContain('上午12时');
    expect(usageLabelText).toContain('下午12时');
    expect(usageLabelText.every((label) => /^(上午12时|下午12时|3|6|9)$/.test(label ?? ''))).toBe(true);
    expect(document.querySelectorAll('.battery-chart-x-boundary')).toHaveLength(0);
    expect(document.querySelector('.battery-chart')).toHaveAttribute('viewBox', '0 0 520 162');
    expect(document.querySelector('.battery-chart-plot-content')).toHaveClass('range-24h');
    expect(document.querySelector('.battery-chart-y-axis')).toHaveClass('range-24h');
    expect(document.querySelector('.battery-chart-x-axis')).toHaveClass('range-24h');
    const averageLine = document.querySelector<SVGLineElement>('.battery-chart-average-line');
    const averageLayer = document.querySelector<SVGGElement>('.battery-chart-average');
    expect(averageLine).not.toBeNull();
    expect(averageLine).toHaveAttribute('x1', '28');
    expect(averageLine).toHaveAttribute('x2', '512');
    expect(averageLine).toHaveAttribute('y1', '0');
    expect(averageLine).toHaveAttribute('y2', '0');
    expect(averageLayer?.style.transform).toMatch(/^translateY\(\d+(\.\d+)?px\)$/);
    expect(document.querySelector('.battery-chart-average-label')).toHaveTextContent(/^平均 \d+%$/);
    expect(document.querySelector('.battery-chart-average-label')).toHaveAttribute('y', '-6');
    expect(document.querySelector('.battery-chart-average-label')).toHaveAttribute('dominant-baseline', 'text-after-edge');
    expect(document.querySelectorAll('.battery-chart-current')).toHaveLength(1);
    expect(document.querySelector('.battery-chart-legend')).toBeNull();
    expect(summaryGrid.compareDocumentPosition(chartCard) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(summaryGrid.children).toHaveLength(4);
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
    const tenDayBars = Array.from(document.querySelectorAll<SVGGElement>('.battery-chart g[role="button"]'));
    expect(tenDayBars).toHaveLength(30);
    expect(document.querySelectorAll('.battery-chart-x-grid')).toHaveLength(10);
    expect(document.querySelectorAll('.battery-chart-x-label')).toHaveLength(10);
    expect(document.querySelectorAll('.battery-chart-x-date').length).toBeGreaterThanOrEqual(2);
    expect(document.querySelector('.battery-chart-plot-content')).toHaveClass('range-10d');
    expect(document.querySelector('.battery-chart-y-axis')).toHaveClass('range-10d');
    expect(document.querySelector('.battery-chart-x-axis')).toHaveClass('range-10d');
    expect(document.querySelector('.battery-chart-legend')).toHaveTextContent('截至现在全天其余时段');
    expect(document.querySelector('.battery-chart-average-label')).toHaveAttribute('y', '6');
    expect(document.querySelector('.battery-chart-average-label')).toHaveAttribute('dominant-baseline', 'text-before-edge');
    expect(document.querySelectorAll('.battery-chart-current')).toHaveLength(1);
    expect(document.querySelectorAll('.battery-chart-after-now').length).toBeGreaterThan(0);
    expect(document.querySelector('.battery-chart-current')).not.toHaveClass('battery-chart-after-now');
    const dateDividers = Array.from(document.querySelectorAll<SVGLineElement>('.battery-chart-x-grid.major'));
    const dateExtensions = Array.from(document.querySelectorAll<SVGLineElement>('.battery-chart-x-extension.major'));
    const weekdayExtensions = Array.from(document.querySelectorAll<SVGLineElement>('.battery-chart-x-extension:not(.major)'));
    const dateLabels = Array.from(document.querySelectorAll<SVGTextElement>('.battery-chart-x-date'));
    expect(dateDividers).toHaveLength(dateLabels.length);
    expect(dateExtensions).toHaveLength(dateLabels.length);
    expect(dateExtensions.every((line) => line.getAttribute('y1') === '116' && line.getAttribute('y2') === '144')).toBe(true);
    expect(weekdayExtensions.every((line) => line.getAttribute('y1') === '116' && line.getAttribute('y2') === '129')).toBe(true);
    expect(Number(dateLabels[0].getAttribute('x'))).toBe(Number(dateDividers[0].getAttribute('x1')) + 4);
    const datedBar = tenDayBars.find((button) => /\d{2}-\d{2} \d{2}:00–\d{2}:00:/.test(button.getAttribute('aria-label') ?? ''));
    expect(datedBar).toBeDefined();
    if (!datedBar) throw new Error('10-day chart has no dated bar');
    fireEvent.mouseEnter(datedBar);
    expect(document.querySelector('.battery-chart-tooltip strong')).toHaveTextContent('日期');

    const chargingPath = document.querySelector<SVGPathElement>('.battery-chart-charging');
    const chargingBar = chargingPath?.closest<SVGGElement>('g[role="button"]');
    expect(chargingBar).not.toBeNull();
    if (!chargingBar) throw new Error('10-day chart has no charging period');
    fireEvent.mouseEnter(chargingBar);
    expect(document.querySelector('.battery-chart-tooltip')).toHaveTextContent('时段内充电: 是');
  });

  it('formats English hour, weekday, and date labels compactly', async () => {
    await i18n.changeLanguage('en');
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
    const hourLabels = Array.from(document.querySelectorAll<SVGTextElement>('.battery-chart-x-label'))
      .map((label) => label.textContent);
    expect(hourLabels).toContain('12 AM');
    expect(hourLabels).toContain('12 PM');
    expect(hourLabels.every((label) => /^(12 AM|12 PM|3|6|9)$/.test(label ?? ''))).toBe(true);

    fireEvent.click(screen.getByRole('tab', { name: '10 days' }));
    await waitFor(() => expect(callCount).toBe(2));
    const weekdayLabels = Array.from(document.querySelectorAll<SVGTextElement>('.battery-chart-x-label'))
      .map((label) => label.textContent ?? '');
    const dateLabels = Array.from(document.querySelectorAll<SVGTextElement>('.battery-chart-x-date'))
      .map((label) => label.textContent ?? '');
    expect(weekdayLabels).toHaveLength(10);
    expect(weekdayLabels.every((label) => /^[A-Z]$/.test(label))).toBe(true);
    expect(dateLabels.every((label) => /^\d{1,2}\/\d{1,2}$/.test(label))).toBe(true);
  });

  it('keeps the current chart coherent until the next range response arrives', async () => {
    let resolveTenDay: ((response: BatteryHistoryResponse) => void) | undefined;
    invokeMock.mockImplementation((command: string, payload?: { range?: string }) => {
      if (command === 'settings_get') return Promise.resolve(settingsEnabled);
      if (command === 'battery_history_get' && payload?.range === '10d') {
        return new Promise<BatteryHistoryResponse>((resolve) => { resolveTenDay = resolve; });
      }
      if (command === 'battery_history_get') return Promise.resolve(MOCK_BATTERY_HISTORY_24H);
      return Promise.resolve(undefined);
    });

    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(document.querySelectorAll('.battery-chart g[role="button"]')).toHaveLength(48));
    const averageLayer = document.querySelector<SVGGElement>('.battery-chart-average');
    const initialAverageTransform = averageLayer?.style.transform;
    fireEvent.click(screen.getByRole('tab', { name: '10 天' }));

    expect(screen.getByRole('tab', { name: '10 天' })).toHaveAttribute('aria-selected', 'true');
    expect(document.querySelectorAll('.battery-chart g[role="button"]')).toHaveLength(48);
    expect(document.querySelectorAll('.battery-chart-x-label')).toHaveLength(8);
    expect(document.querySelector('.battery-chart')).toHaveAttribute('viewBox', '0 0 520 162');

    resolveTenDay?.(MOCK_BATTERY_HISTORY_10D);
    await waitFor(() => expect(document.querySelectorAll('.battery-chart g[role="button"]')).toHaveLength(30));
    expect(document.querySelector('.battery-chart-average')).toBe(averageLayer);
    expect(document.querySelector<SVGGElement>('.battery-chart-average')?.style.transform).not.toBe(initialAverageTransform);
    expect(document.querySelectorAll('.battery-chart-x-label')).toHaveLength(10);
    expect(document.querySelector('.battery-chart')).toHaveAttribute('viewBox', '0 0 520 162');
  });

  it('keeps the native device switcher clickable across repeated selections', async () => {
    mockInvoke({ response: MOCK_BATTERY_HISTORY_24H });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('battery_history_get', { range: '24h' }));
    const switcher = screen.getByRole('button', { name: '切换设备' });
    expect(switcher.tagName).toBe('BUTTON');
    expect(switcher.querySelector('button')).toBeNull();

    for (let index = 0; index < 12; index += 1) {
      const selectReceiver = index % 2 === 0;
      fireEvent.click(switcher);
      const item = screen.getByRole('menuitemradio', { name: selectReceiver ? /接收器/ : /鼠标/ });
      fireEvent.click(item);
      expect(document.querySelector('.battery-status-metric strong'))
        .toHaveTextContent(selectReceiver ? '96%' : '82%');
      expect(switcher).toHaveAttribute('aria-expanded', 'false');
    }
  });

  it('falls back to an available device when a refreshed range drops the selection', async () => {
    const tenDayWithoutReceiver: BatteryHistoryResponse = {
      ...MOCK_BATTERY_HISTORY_10D,
      devices: MOCK_BATTERY_HISTORY_10D.devices.filter((device) => device.componentId !== 'receiver'),
      series: MOCK_BATTERY_HISTORY_10D.series.filter((series) => series.key !== 'mouse:abc123:receiver'),
    };
    invokeMock.mockImplementation((command: string, payload?: { range?: string }) => {
      if (command === 'settings_get') return Promise.resolve(settingsEnabled);
      if (command === 'battery_history_get') {
        return Promise.resolve(payload?.range === '10d' ? tenDayWithoutReceiver : MOCK_BATTERY_HISTORY_24H);
      }
      return Promise.resolve(undefined);
    });

    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(document.querySelector('.battery-status-metric strong')).toHaveTextContent('82%'));
    fireEvent.click(screen.getByRole('button', { name: '切换设备' }));
    fireEvent.click(screen.getByRole('menuitemradio', { name: /接收器/ }));
    expect(document.querySelector('.battery-status-metric strong')).toHaveTextContent('96%');

    fireEvent.click(screen.getByRole('tab', { name: '10 天' }));
    await waitFor(() => expect(document.querySelector('.battery-status-metric strong')).toHaveTextContent('82%'));
    expect(document.querySelector('.battery-status-strip')).toBeInTheDocument();
  });

  it('defaults to the current mouse instead of the first historical device', async () => {
    const response = {
      ...MOCK_BATTERY_HISTORY_24H,
      devices: [...MOCK_BATTERY_HISTORY_24H.devices].reverse(),
    };
    mockInvoke({ response });
    render(
      <BatteryUsageModal
        open
        onClose={() => {}}
        hasBattery
        preferredDeviceName="Mira Example Wireless Mouse"
        preferredComponentId="mouse"
      />,
    );
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('battery_history_get', { range: '24h' }));
    expect(document.querySelector('.battery-status-metric strong')).toHaveTextContent('82%');
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
