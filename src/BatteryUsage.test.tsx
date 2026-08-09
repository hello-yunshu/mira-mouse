// SPDX-License-Identifier: AGPL-3.0-or-later
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
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
vi.mock('thinking-orbs', () => ({
  ThinkingOrb: ({ state }: { state: string }) => (
    <span data-testid="thinking-orb" data-state={state} />
  ),
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
    expect(screen.getByText('根据本地电量历史生成趋势、耗电与充电摘要')).toBeInTheDocument();
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
    expect(screen.queryByText('根据趋势、异常掉电与充电记录生成')).toBeNull();
  });

  it('shows active-use prediction separately from natural remaining time', async () => {
    mockInvoke({
      response: {
        ...MOCK_BATTERY_HISTORY_24H,
        insights: [
          {
            type: 'estimatedRemaining',
            severity: 'info',
            title: 'estimatedRemaining',
            message: 'remainingDaysHours|6|0',
            deviceKey: 'mouse:abc123:mouse',
          },
          {
            type: 'estimatedActiveRemaining',
            severity: 'info',
            title: 'estimatedActiveRemaining',
            message: 'remainingDaysHours|2|8',
            deviceKey: 'mouse:abc123:mouse',
          },
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

    expect(document.querySelector('.battery-summary-grid')).toHaveTextContent('预计剩余时间6 天 0 小时');
    expect(screen.getByText('预计活跃使用')).toBeInTheDocument();
    expect(screen.getByText('2 天 8 小时')).toBeInTheDocument();
  });

  it('renders the four summary blocks in one row above the 24h chart', async () => {
    mockInvoke({ response: MOCK_BATTERY_HISTORY_24H });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('battery_history_get', { range: '24h' }));
    expect(screen.getByRole('heading', { name: '电量使用情况' })).toBeInTheDocument();
    expect(screen.getByText('本地 AI')).toBeInTheDocument();
    expect(screen.getByText('根据趋势、异常掉电与充电记录生成')).toBeInTheDocument();
    const range24h = screen.getByRole('tab', { name: '24 小时' });
    expect(range24h).toHaveAttribute('aria-selected', 'true');
    const chartCard = document.querySelector('.battery-chart-card');
    const summaryGrid = document.querySelector('.battery-summary-grid');
    expect(chartCard).not.toBeNull();
    expect(summaryGrid).not.toBeNull();
    if (!chartCard || !summaryGrid) throw new Error('battery chart or summary grid missing');
    expect(chartCard.querySelector('.battery-chart-header')).toHaveTextContent('近 24 小时 · 累计使用');
    expect(summaryGrid).toHaveTextContent('近 24 小时状态');
    expect(document.querySelectorAll('.battery-chart g[role="button"]')).toHaveLength(48);
    const usageGridLines = Array.from(document.querySelectorAll<SVGLineElement>('.battery-chart-x-grid'));
    const usageExtensions = Array.from(document.querySelectorAll<SVGLineElement>('.battery-chart-x-extension'));
    const usageLabels = Array.from(document.querySelectorAll<SVGTextElement>('.battery-chart-x-label'));
    expect(usageGridLines).toHaveLength(8);
    expect(usageExtensions).toHaveLength(8);
    expect(usageLabels).toHaveLength(8);
    expect(usageGridLines.every((line, index) => {
      const tx = line.style.transform.match(/translateX\(([\d.]+)px\)/);
      return tx && Number(tx[1]) === Number(usageLabels[index].getAttribute('x'));
    })).toBe(true);
    expect(usageExtensions.every((line) => line.style.transform.includes('144px') && line.style.transform.includes('scaleY(4)'))).toBe(true);
    const usageLabelText = usageLabels.map((label) => label.textContent);
    expect(usageLabelText).toEqual(['0', '0.5', '1 小时', '1.5', '2 小时', '2.5', '3 小时', '3.5']);
    expect(document.querySelectorAll('.battery-chart-x-grid.major')).toHaveLength(3);
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

    const firstUsagePoint = document.querySelector<SVGGElement>('.battery-chart g[role="button"]');
    if (!firstUsagePoint) throw new Error('24-hour chart has no usage point');
    fireEvent.mouseEnter(firstUsagePoint);
    expect(document.querySelector('.battery-chart-tooltip')).toHaveTextContent('累计使用: 0m');
    expect(document.querySelector('.battery-chart-tooltip')).toHaveTextContent('实际时间:');
  });

  it('keeps a cumulative-usage unit visible when the recorded range has no natural major tick', async () => {
    const shortUsageResponse: BatteryHistoryResponse = {
      ...MOCK_BATTERY_HISTORY_24H,
      series: MOCK_BATTERY_HISTORY_24H.series.map((series) => ({
        ...series,
        points: series.points.slice(0, 4).map((point, index) => ({
          ...point,
          usageElapsedMinutes: index * 5,
        })),
      })),
    };
    mockInvoke({ response: shortUsageResponse });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('battery_history_get', { range: '24h' }));

    const usageLabels = Array.from(document.querySelectorAll<SVGTextElement>('.battery-chart-x-label'))
      .map((label) => label.textContent);
    expect(usageLabels).toEqual(['0', '5', '10', '15 分钟']);
    expect(document.querySelectorAll('.battery-chart-x-grid.major')).toHaveLength(1);
  });

  it('switches to 10d range using cached response without refetching', async () => {
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
    // 打开时并行拉取 24h + 10d 两个 range（callCount=2），切换不再触发请求。
    await waitFor(() => expect(callCount).toBe(2));
    const range10d = screen.getByRole('tab', { name: '10 天' });
    fireEvent.click(range10d);
    // 切换后立即命中缓存渲染 10d 图表，callCount 仍为 2。
    await waitFor(() => expect(document.querySelectorAll('.battery-chart g[role="button"]')).toHaveLength(30));
    expect(callCount).toBe(2);
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
    expect(dateExtensions.every((line) => line.style.transform.includes('116px') && line.style.transform.includes('scaleY(28)'))).toBe(true);
    expect(weekdayExtensions.every((line) => line.style.transform.includes('116px') && line.style.transform.includes('scaleY(13)'))).toBe(true);
    const gridTx = dateDividers[0].style.transform.match(/translateX\(([\d.]+)px\)/);
    expect(Number(dateLabels[0].getAttribute('x'))).toBe(Number(gridTx?.[1]) + 4);
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

  it('leaves 24h slots blank when they have no battery data', async () => {
    const response24h: BatteryHistoryResponse = {
      ...MOCK_BATTERY_HISTORY_24H,
      series: MOCK_BATTERY_HISTORY_24H.series.map((series, seriesIndex) => ({
        ...series,
        points: series.points.map((point, pointIndex) => (
          seriesIndex === 0 && pointIndex === 0
            ? {
              ...point,
              // Rust `Option::None` 经 Tauri IPC 序列化为 null；必须按真实数据形态
              // 验证，避免 null 被当作 0% 后画出最小高度短柱。
              percentage: null,
              minPercentage: null,
              maxPercentage: null,
              charging: null,
              lowBattery: null,
              sampleCount: 0,
            }
            : point
        )),
      })),
    };
    invokeMock.mockImplementation((command: string, payload?: { range?: string }) => {
      if (command === 'settings_get') return Promise.resolve(settingsEnabled);
      if (command === 'battery_history_get') {
        return Promise.resolve(payload?.range === '10d' ? MOCK_BATTERY_HISTORY_10D : response24h);
      }
      return Promise.resolve(undefined);
    });

    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(document.querySelectorAll('.battery-chart g[role="button"]')).toHaveLength(48));
    expect(document.querySelectorAll('.battery-chart-bar')).toHaveLength(47);
    expect(document.querySelector('.battery-chart-empty')).toBeNull();
  });

  it('does not remount stable blocks when switching range', async () => {
    let callCount = 0;
    invokeMock.mockImplementation((command: string, payload?: { range?: string }) => {
      if (command === 'settings_get') return Promise.resolve(settingsEnabled);
      if (command === 'battery_history_get') {
        callCount += 1;
        // 两次返回的 device 数据完全相同，模拟「切换 range 但当前电量未变」
        return Promise.resolve(payload?.range === '10d' ? MOCK_BATTERY_HISTORY_10D : MOCK_BATTERY_HISTORY_24H);
      }
      return Promise.resolve(undefined);
    });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    // 打开时并行拉取 24h + 10d（callCount=2），切换命中缓存不再触发请求。
    await waitFor(() => expect(callCount).toBe(2));

    // 缓存切换前的关键 DOM 节点引用
    const stripBefore = document.querySelector('.battery-status-strip');
    const primaryBefore = document.querySelector('.battery-summary-item.primary');
    const batteryIconBefore = document.querySelector('.battery-summary-item.primary .battery-level-icon');
    const rangeToggleBefore = document.querySelector('.battery-range-toggle');

    fireEvent.click(screen.getByRole('tab', { name: '10 天' }));
    // 等待 10d 图表渲染完成（命中缓存即渲染，无须等待新请求）。
    await waitFor(() => expect(document.querySelectorAll('.battery-chart g[role="button"]')).toHaveLength(30));
    expect(callCount).toBe(2);

    // 切换后再次查询，外部容器与电池图标节点必须是同一个 DOM 引用，
    // 证明没有卸载重建，CSS 入场动画/transition 不会被重新触发。
    const stripAfter = document.querySelector('.battery-status-strip');
    const primaryAfter = document.querySelector('.battery-summary-item.primary');
    const batteryIconAfter = document.querySelector('.battery-summary-item.primary .battery-level-icon');
    const rangeToggleAfter = document.querySelector('.battery-range-toggle');

    expect(stripBefore).toBe(stripAfter);
    expect(primaryBefore).toBe(primaryAfter);
    expect(batteryIconBefore).toBe(batteryIconAfter);
    expect(rangeToggleBefore).toBe(rangeToggleAfter);
  });

  it('formats English cumulative usage, weekday, and date labels compactly', async () => {
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
    // 打开时并行拉取 24h + 10d（callCount=2）。
    await waitFor(() => expect(callCount).toBe(2));
    const usageLabels = Array.from(document.querySelectorAll<SVGTextElement>('.battery-chart-x-label'))
      .map((label) => label.textContent);
    expect(usageLabels).toEqual(['0', '0.5', '1 hr', '1.5', '2 hr', '2.5', '3 hr', '3.5']);
    expect(document.querySelector('.battery-chart-header')).toHaveTextContent('Last 24 hours · Usage elapsed');

    fireEvent.click(screen.getByRole('tab', { name: '10 days' }));
    // 切换命中缓存，callCount 仍为 2；等待 10d 标签渲染完成。
    await waitFor(() => expect(document.querySelectorAll('.battery-chart-x-label')).toHaveLength(10));
    expect(callCount).toBe(2);
    // chart header 标题经 FadeText 过渡，需等待文本更新完成。
    await waitFor(() => expect(document.querySelector('.battery-chart-header')).toHaveTextContent('Past 10 days'));
    const weekdayLabels = Array.from(document.querySelectorAll<SVGTextElement>('.battery-chart-x-label'))
      .map((label) => label.textContent ?? '');
    const dateLabels = Array.from(document.querySelectorAll<SVGTextElement>('.battery-chart-x-date'))
      .map((label) => label.textContent ?? '');
    expect(weekdayLabels).toHaveLength(10);
    expect(weekdayLabels.every((label) => /^[A-Z]$/.test(label))).toBe(true);
    expect(dateLabels.every((label) => /^\d{1,2}\/\d{1,2}$/.test(label))).toBe(true);
  });

  it('keeps the chart loading until both ranges arrive in parallel', async () => {
    // 新架构下打开时并行拉取 24h + 10d，两者都到齐前保持 loading。
    // 这里验证：10d 响应延迟时，loading 期间不渲染图表；10d 到齐后才渲染。
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
    // 24h 已到齐但 10d 未到，loading 期间不渲染图表。
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('battery_history_get', { range: '10d' }));
    expect(document.querySelector('.battery-chart')).toBeNull();

    resolveTenDay?.(MOCK_BATTERY_HISTORY_10D);
    await waitFor(() => expect(document.querySelectorAll('.battery-chart g[role="button"]')).toHaveLength(48));
    expect(document.querySelector('.battery-chart')).toHaveAttribute('viewBox', '0 0 520 162');
  });

  it('caps insight card count to the minimum of 24h and 10d visible counts', async () => {
    // 24h 有 4 个 basic insights（chargingHabit/batteryConsistency/averageDailyDrain/chargingCount），
    // 经 filterInsightsForCards 规整后为 4 张卡片。10d 只有 2 个 basic insights，规整后为 2 张。
    // minInsightCount = min(4, 2) = 2，两个 range 都只渲染 2 张卡片，
    // 从源头避免「切换 range 时卡片增减」造成布局抖动。
    const response24h: BatteryHistoryResponse = {
      ...MOCK_BATTERY_HISTORY_24H,
      insights: [
        { type: 'chargingHabit', severity: 'info', title: 'chargingHabit', message: 'chargingHabitStartEnd|18|92|3', deviceKey: 'mouse:abc123:mouse' },
        { type: 'batteryConsistency', severity: 'info', title: 'batteryConsistency', message: 'consistencyStable', deviceKey: 'mouse:abc123:mouse' },
        { type: 'averageDailyDrain', severity: 'info', title: 'averageDailyDrain', message: 'averageDailyDrain|2.3', deviceKey: 'mouse:abc123:mouse' },
        { type: 'chargingCount', severity: 'info', title: 'chargingCount', message: 'chargingCount|1', deviceKey: 'mouse:abc123:mouse' },
      ],
    };
    const response10d: BatteryHistoryResponse = {
      ...MOCK_BATTERY_HISTORY_10D,
      insights: [
        { type: 'chargingHabit', severity: 'info', title: 'chargingHabit', message: 'chargingHabitStartEnd|18|92|3', deviceKey: 'mouse:abc123:mouse' },
        { type: 'batteryConsistency', severity: 'info', title: 'batteryConsistency', message: 'consistencyStable', deviceKey: 'mouse:abc123:mouse' },
      ],
    };
    invokeMock.mockImplementation((command: string, payload?: { range?: string }) => {
      if (command === 'settings_get') return Promise.resolve(settingsEnabled);
      if (command === 'battery_history_get') {
        return Promise.resolve(payload?.range === '10d' ? response10d : response24h);
      }
      return Promise.resolve(undefined);
    });

    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    // 两个 range 都到齐后，取最小卡片数（2），24h 只渲染 2 张（虽然实际有 4 个 insights）。
    await waitFor(() => expect(document.querySelectorAll('.battery-insight-card')).toHaveLength(2));

    // 切换到 10d，卡片数仍为 2（与 24h 一致，无增减）。
    fireEvent.click(screen.getByRole('tab', { name: '10 天' }));
    await waitFor(() => expect(document.querySelector('.battery-chart-plot-content')).toHaveClass('range-10d'));
    expect(document.querySelectorAll('.battery-insight-card')).toHaveLength(2);

    // 切换回 24h，卡片数仍为 2（被 maxCount 限制）。
    fireEvent.click(screen.getByRole('tab', { name: '24 小时' }));
    await waitFor(() => expect(document.querySelector('.battery-chart-plot-content')).toHaveClass('range-24h'));
    expect(document.querySelectorAll('.battery-insight-card')).toHaveLength(2);
  });

  it('hides the entire insight section when min count drops to zero', async () => {
    // 边界场景：24h 只有 1 个洞察（奇数截断为 0），10d 有 4 个洞察（保留 4 个）。
    // minInsightCount = min(0, 4) = 0，整个洞察 section（标题、副标题、卡片）都不渲染。
    // 切换到 10d 时同理：minInsightCount 仍是 0，10d 即使有 4 个洞察也不显示。
    const response24h: BatteryHistoryResponse = {
      ...MOCK_BATTERY_HISTORY_24H,
      insights: [
        { type: 'chargingHabit', severity: 'info', title: 'chargingHabit', message: 'chargingHabitStartEnd|18|92|3', deviceKey: 'mouse:abc123:mouse' },
      ],
    };
    const response10d: BatteryHistoryResponse = {
      ...MOCK_BATTERY_HISTORY_10D,
      insights: [
        { type: 'chargingHabit', severity: 'info', title: 'chargingHabit', message: 'chargingHabitStartEnd|18|92|3', deviceKey: 'mouse:abc123:mouse' },
        { type: 'batteryConsistency', severity: 'info', title: 'batteryConsistency', message: 'consistencyStable', deviceKey: 'mouse:abc123:mouse' },
        { type: 'averageDailyDrain', severity: 'info', title: 'averageDailyDrain', message: 'averageDailyDrain|1.8', deviceKey: 'mouse:abc123:mouse' },
        { type: 'chargingCount', severity: 'info', title: 'chargingCount', message: 'chargingCount|6', deviceKey: 'mouse:abc123:mouse' },
      ],
    };
    invokeMock.mockImplementation((command: string, payload?: { range?: string }) => {
      if (command === 'settings_get') return Promise.resolve(settingsEnabled);
      if (command === 'battery_history_get') {
        return Promise.resolve(payload?.range === '10d' ? response10d : response24h);
      }
      return Promise.resolve(undefined);
    });

    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    // 两个 range 都到齐后，minInsightCount=0，洞察 section 完全不渲染。
    await waitFor(() => expect(document.querySelector('.battery-chart')).not.toBeNull());
    expect(document.querySelector('.battery-insight-section')).toBeNull();
    expect(document.querySelector('.battery-insight-card')).toBeNull();
    expect(screen.queryByText('本地 AI 洞察')).toBeNull();
    expect(screen.queryByText('用电洞察')).toBeNull();
    expect(screen.queryByText('根据趋势、异常掉电与充电记录生成')).toBeNull();
    expect(screen.queryByText('根据本地电量历史生成趋势、耗电与充电摘要')).toBeNull();

    // 切换到 10d：minInsightCount 仍是 0（来自缓存），10d 即使有 4 个洞察也不显示。
    fireEvent.click(screen.getByRole('tab', { name: '10 天' }));
    await waitFor(() => expect(document.querySelector('.battery-chart-plot-content')).toHaveClass('range-10d'));
    expect(document.querySelector('.battery-insight-section')).toBeNull();
    expect(document.querySelector('.battery-insight-card')).toBeNull();
    expect(screen.queryByText('本地 AI 洞察')).toBeNull();
    expect(screen.queryByText('用电洞察')).toBeNull();
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
      // FadeText 在文本变化时触发 160ms 淡入淡出过渡，过渡结束后 currentValue 才更新为新值。
      await waitFor(() => expect(document.querySelector('.battery-status-metric strong'))
        .toHaveTextContent(selectReceiver ? '96%' : '82%'));
      expect(switcher).toHaveAttribute('aria-expanded', 'false');
    }
  });

  it('only offers currently connected battery targets in the device switcher', async () => {
    const historicalOnlyDevice = {
      ...MOCK_BATTERY_HISTORY_24H.devices[0],
      key: 'mouse:old-device:mouse',
      deviceId: 'old-device',
      deviceName: 'Old Offline Mouse',
      latestPercentage: 55,
    };
    mockInvoke({
      response: {
        ...MOCK_BATTERY_HISTORY_24H,
        devices: [...MOCK_BATTERY_HISTORY_24H.devices, historicalOnlyDevice],
      },
    });

    render(
      <BatteryUsageModal
        open
        onClose={() => {}}
        hasBattery
        connectedTargets={[
          { deviceName: 'Mira Example Wireless Mouse', componentId: 'mouse' },
          { deviceName: 'Mira Example Wireless Mouse', componentId: 'receiver' },
        ]}
      />,
    );
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('battery_history_get', { range: '24h' }));

    fireEvent.click(screen.getByRole('button', { name: '切换设备' }));
    expect(screen.getAllByRole('menuitemradio')).toHaveLength(2);
    expect(screen.queryByRole('menuitemradio', { name: /Old Offline Mouse/ })).toBeNull();
  });

  it('overlays main-screen battery values without triggering an extra device read', async () => {
    mockInvoke();
    const { rerender } = render(
      <BatteryUsageModal
        open
        onClose={() => {}}
        hasBattery
        connectedTargets={[
          {
            deviceName: 'Mira Example Wireless Mouse',
            componentId: 'mouse',
            latestPercentage: 67,
            latestCharging: true,
            latestAt: new Date().toISOString(),
            lowBattery: false,
          },
          {
            deviceName: 'Mira Example Wireless Mouse',
            componentId: 'receiver',
            latestPercentage: 91,
            latestCharging: false,
            latestAt: new Date().toISOString(),
            lowBattery: false,
          },
        ]}
      />,
    );

    await waitFor(() => expect(document.querySelector('.battery-status-metric strong')).toHaveTextContent('67%'));
    expect(invokeMock).not.toHaveBeenCalledWith('device_refresh_battery');
    expect(screen.getAllByText('67%').length).toBeGreaterThan(0);

    fireEvent.click(screen.getByRole('button', { name: '切换设备' }));
    expect(screen.getByRole('menuitemradio', { name: /接收器/ })).toHaveTextContent('91%');

    fireEvent.click(screen.getByRole('menuitemradio', { name: /鼠标/ }));
    rerender(
      <BatteryUsageModal
        open
        onClose={() => {}}
        hasBattery
        connectedTargets={[
          {
            deviceName: 'Mira Example Wireless Mouse',
            componentId: 'mouse',
            latestPercentage: 12,
            latestCharging: false,
            latestAt: new Date().toISOString(),
            lowBattery: true,
          },
          {
            deviceName: 'Mira Example Wireless Mouse',
            componentId: 'receiver',
            latestPercentage: 91,
            latestCharging: false,
            latestAt: new Date().toISOString(),
            lowBattery: false,
          },
        ]}
      />,
    );
    await waitFor(() => expect(document.querySelector('.battery-status-metric strong')).toHaveTextContent('12%'));
    expect(document.querySelector('.battery-status-strip')).toHaveClass('low');
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
    // FadeText 在文本变化时触发 160ms 淡入淡出过渡，过渡结束后 currentValue 才更新为新值。
    await waitFor(() => expect(document.querySelector('.battery-status-metric strong')).toHaveTextContent('96%'));

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

// ─── P0-D 滚动淡出测试 ───────────────────────────────────────────────────────
// 验证 BatteryUsageModal 的滚动区在内容溢出/缩短/滚动/切换等场景下，
// 正确应用 scroll-fade-top / scroll-fade-bottom 类，且 scrollHeight ==
// clientHeight 时不显示任何淡出。

/** 可变滚动度量引用，便于测试中动态修改。 */
interface ScrollMetrics {
  scrollHeight: number;
  clientHeight: number;
  scrollTop: number;
}

/** 给 .battery-usage-scroll-region 注入可控的 scrollHeight/clientHeight/scrollTop。 */
function injectBatteryScrollMetrics(metrics: ScrollMetrics): HTMLElement {
  const el = document.querySelector('.battery-usage-scroll-region') as HTMLElement;
  if (!el) throw new Error('battery-usage-scroll-region not found');
  Object.defineProperty(el, 'scrollHeight', { configurable: true, get: () => metrics.scrollHeight });
  Object.defineProperty(el, 'clientHeight', { configurable: true, get: () => metrics.clientHeight });
  Object.defineProperty(el, 'scrollTop', { configurable: true, get: () => metrics.scrollTop, set: (v: number) => { metrics.scrollTop = v; } });
  return el;
}

/** 触发 scroll 事件让 hook 重新测量。 */
function triggerScroll(el: HTMLElement): void {
  el.dispatchEvent(new Event('scroll', { bubbles: false }));
}

/** 等待 rAF + React 重渲染完成。 */
function waitForRaf(): Promise<void> {
  return new Promise((resolve) => requestAnimationFrame(() => setTimeout(resolve, 0)));
}

describe('BatteryUsageModal scroll fade (P0-D)', () => {
  beforeEach(async () => {
    invokeMock.mockReset();
    await i18n.changeLanguage('zh-CN');
  });

  it('打开弹窗首帧无溢出，不应用任何 scroll-fade 类', async () => {
    mockInvoke({ response: MOCK_BATTERY_HISTORY_24H });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('battery_history_get', { range: '24h' }));
    await waitFor(() => expect(document.querySelector('.battery-chart')).not.toBeNull());
    const scrollRegion = document.querySelector('.battery-usage-scroll-region') as HTMLElement;
    expect(scrollRegion).not.toBeNull();
    // 首帧无溢出（jsdom 默认 scrollHeight=clientHeight=0）
    expect(scrollRegion.classList.contains('scroll-fade-top')).toBe(false);
    expect(scrollRegion.classList.contains('scroll-fade-bottom')).toBe(false);
  });

  it('loading → loaded 后无溢出', async () => {
    mockInvoke({ response: MOCK_BATTERY_HISTORY_24H });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(document.querySelector('.battery-chart')).not.toBeNull());
    const scrollRegion = document.querySelector('.battery-usage-scroll-region') as HTMLElement;
    // 等待 rAF 让 hook 测量完成
    await waitForRaf();
    expect(scrollRegion.classList.contains('scroll-fade-top')).toBe(false);
    expect(scrollRegion.classList.contains('scroll-fade-bottom')).toBe(false);
  });

  it('初始溢出 → insight 高度收缩后不溢出', async () => {
    mockInvoke({ response: MOCK_BATTERY_HISTORY_24H });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(document.querySelector('.battery-chart')).not.toBeNull());
    const metrics: ScrollMetrics = { scrollHeight: 500, clientHeight: 200, scrollTop: 0 };
    const el = injectBatteryScrollMetrics(metrics);
    triggerScroll(el);
    // 初始溢出，在顶部 → 只有底部淡出
    await waitFor(() => expect(el.classList.contains('scroll-fade-bottom')).toBe(true));
    expect(el.classList.contains('scroll-fade-top')).toBe(false);
    // 内容收缩到不溢出
    metrics.scrollHeight = 200;
    triggerScroll(el);
    await waitFor(() => expect(el.classList.contains('scroll-fade-bottom')).toBe(false));
    expect(el.classList.contains('scroll-fade-top')).toBe(false);
  });

  it('到顶部 → 只有底部淡出', async () => {
    mockInvoke({ response: MOCK_BATTERY_HISTORY_24H });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(document.querySelector('.battery-chart')).not.toBeNull());
    const metrics: ScrollMetrics = { scrollHeight: 500, clientHeight: 200, scrollTop: 0 };
    const el = injectBatteryScrollMetrics(metrics);
    triggerScroll(el);
    await waitFor(() => expect(el.classList.contains('scroll-fade-bottom')).toBe(true));
    expect(el.classList.contains('scroll-fade-top')).toBe(false);
  });

  it('滚动中间 → 上下都有淡出', async () => {
    mockInvoke({ response: MOCK_BATTERY_HISTORY_24H });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(document.querySelector('.battery-chart')).not.toBeNull());
    const metrics: ScrollMetrics = { scrollHeight: 500, clientHeight: 200, scrollTop: 0 };
    const el = injectBatteryScrollMetrics(metrics);
    triggerScroll(el);
    // 先到顶部
    await waitFor(() => expect(el.classList.contains('scroll-fade-bottom')).toBe(true));
    // 滚到中间
    metrics.scrollTop = 150;
    triggerScroll(el);
    await waitFor(() => expect(el.classList.contains('scroll-fade-top')).toBe(true));
    expect(el.classList.contains('scroll-fade-bottom')).toBe(true);
  });

  it('到底部 → 只有顶部淡出', async () => {
    mockInvoke({ response: MOCK_BATTERY_HISTORY_24H });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(document.querySelector('.battery-chart')).not.toBeNull());
    const metrics: ScrollMetrics = { scrollHeight: 500, clientHeight: 200, scrollTop: 0 };
    const el = injectBatteryScrollMetrics(metrics);
    triggerScroll(el);
    // 先到顶部
    await waitFor(() => expect(el.classList.contains('scroll-fade-bottom')).toBe(true));
    // 滚到底部
    metrics.scrollTop = 300; // 300 + 200 = 500 = scrollHeight
    triggerScroll(el);
    await waitFor(() => expect(el.classList.contains('scroll-fade-top')).toBe(true));
    expect(el.classList.contains('scroll-fade-bottom')).toBe(false);
  });

  it('24h → 10d 切换后淡出状态正确更新', async () => {
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
    await waitFor(() => expect(callCount).toBe(2));
    const metrics: ScrollMetrics = { scrollHeight: 500, clientHeight: 200, scrollTop: 0 };
    const el = injectBatteryScrollMetrics(metrics);
    triggerScroll(el);
    await waitFor(() => expect(el.classList.contains('scroll-fade-bottom')).toBe(true));
    // 切换到 10d
    fireEvent.click(screen.getByRole('tab', { name: '10 天' }));
    await waitFor(() => expect(document.querySelectorAll('.battery-chart g[role="button"]')).toHaveLength(30));
    // 切换后淡出状态仍正确（容器元素不变，度量仍注入）
    expect(el.classList.contains('scroll-fade-bottom')).toBe(true);
    expect(el.classList.contains('scroll-fade-top')).toBe(false);
  });

  it('10d → 24h 切换后淡出状态正确更新', async () => {
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
    await waitFor(() => expect(callCount).toBe(2));
    // 先切到 10d
    fireEvent.click(screen.getByRole('tab', { name: '10 天' }));
    await waitFor(() => expect(document.querySelectorAll('.battery-chart g[role="button"]')).toHaveLength(30));
    const metrics: ScrollMetrics = { scrollHeight: 600, clientHeight: 200, scrollTop: 0 };
    const el = injectBatteryScrollMetrics(metrics);
    triggerScroll(el);
    await waitFor(() => expect(el.classList.contains('scroll-fade-bottom')).toBe(true));
    // 切回 24h
    fireEvent.click(screen.getByRole('tab', { name: '24 小时' }));
    await waitFor(() => expect(document.querySelectorAll('.battery-chart g[role="button"]')).toHaveLength(48));
    expect(el.classList.contains('scroll-fade-bottom')).toBe(true);
    expect(el.classList.contains('scroll-fade-top')).toBe(false);
  });

  it('切换设备后淡出状态正确', async () => {
    const multiDeviceResponse: BatteryHistoryResponse = {
      ...MOCK_BATTERY_HISTORY_24H,
      devices: [
        ...MOCK_BATTERY_HISTORY_24H.devices,
        {
          ...MOCK_BATTERY_HISTORY_24H.devices[0],
          key: 'mouse:xyz789:mouse',
          deviceId: 'xyz789',
          deviceName: 'Another Mouse',
        },
      ],
    };
    invokeMock.mockImplementation((command: string, payload?: { range?: string }) => {
      if (command === 'settings_get') return Promise.resolve(settingsEnabled);
      if (command === 'battery_history_get') {
        return Promise.resolve(payload?.range === '10d' ? MOCK_BATTERY_HISTORY_10D : multiDeviceResponse);
      }
      return Promise.resolve(undefined);
    });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(document.querySelector('.battery-chart')).not.toBeNull());
    const metrics: ScrollMetrics = { scrollHeight: 500, clientHeight: 200, scrollTop: 0 };
    const el = injectBatteryScrollMetrics(metrics);
    triggerScroll(el);
    await waitFor(() => expect(el.classList.contains('scroll-fade-bottom')).toBe(true));
    // 切换设备（点击第二个设备 pill）
    const deviceButtons = document.querySelectorAll('.battery-status-strip-device');
    if (deviceButtons.length > 1) {
      fireEvent.click(deviceButtons[1]);
      await waitForRaf();
      // 淡出状态仍正确
      expect(el.classList.contains('scroll-fade-bottom')).toBe(true);
    }
  });

  it('清空历史后淡出状态正确', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settingsEnabled);
      if (command === 'battery_history_get') return Promise.resolve(MOCK_BATTERY_HISTORY_24H);
      if (command === 'battery_history_clear') return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(document.querySelector('.battery-chart')).not.toBeNull());
    const metrics: ScrollMetrics = { scrollHeight: 500, clientHeight: 200, scrollTop: 0 };
    const el = injectBatteryScrollMetrics(metrics);
    triggerScroll(el);
    await waitFor(() => expect(el.classList.contains('scroll-fade-bottom')).toBe(true));
    // 点击清除
    const clearBtn = screen.getByRole('button', { name: '清除电量历史' });
    fireEvent.click(clearBtn);
    const confirmBtn = await screen.findByRole('button', { name: '确认清除' });
    fireEvent.click(confirmBtn);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('battery_history_clear', expect.anything()));
    // 清空后度量调整为不溢出
    metrics.scrollHeight = 200;
    triggerScroll(el);
    await waitFor(() => expect(el.classList.contains('scroll-fade-bottom')).toBe(false), { timeout: 2000 });
    expect(el.classList.contains('scroll-fade-top')).toBe(false);
  });

  it('AI 开启/关闭造成卡片数量变化后淡出状态正确', async () => {
    // 先以 AI 关闭模式打开
    invokeMock.mockImplementation((command: string, payload?: { range?: string }) => {
      if (command === 'settings_get') return Promise.resolve(settingsEnabled);
      if (command === 'battery_history_get') {
        return Promise.resolve(payload?.range === '10d' ? MOCK_BATTERY_HISTORY_10D : MOCK_BATTERY_HISTORY_24H);
      }
      return Promise.resolve(undefined);
    });
    const { rerender } = render(
      <BatteryUsageModal open onClose={() => {}} hasBattery batteryHistoryEnabled aiAnalysisEnabled={false} />,
    );
    await waitFor(() => expect(document.querySelector('.battery-chart')).not.toBeNull());
    const metrics: ScrollMetrics = { scrollHeight: 500, clientHeight: 200, scrollTop: 0 };
    const el = injectBatteryScrollMetrics(metrics);
    triggerScroll(el);
    await waitFor(() => expect(el.classList.contains('scroll-fade-bottom')).toBe(true));
    // AI 开启（卡片数量变化但容器不变，淡出状态保持）
    rerender(
      <BatteryUsageModal open onClose={() => {}} hasBattery batteryHistoryEnabled aiAnalysisEnabled />,
    );
    await waitForRaf();
    expect(el.classList.contains('scroll-fade-bottom')).toBe(true);
    expect(el.classList.contains('scroll-fade-top')).toBe(false);
  });

  it('窗口高度改变后重新测量', async () => {
    mockInvoke({ response: MOCK_BATTERY_HISTORY_24H });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(document.querySelector('.battery-chart')).not.toBeNull());
    // 初始不溢出
    const metrics: ScrollMetrics = { scrollHeight: 200, clientHeight: 200, scrollTop: 0 };
    const el = injectBatteryScrollMetrics(metrics);
    triggerScroll(el);
    await waitForRaf();
    expect(el.classList.contains('scroll-fade-bottom')).toBe(false);
    // 窗口缩小（clientHeight 变小）→ 内容溢出
    metrics.clientHeight = 100;
    window.dispatchEvent(new Event('resize'));
    await waitFor(() => expect(el.classList.contains('scroll-fade-bottom')).toBe(true), { timeout: 2000 });
    expect(el.classList.contains('scroll-fade-top')).toBe(false);
  });

  it('洞察卡片高度过渡结束后重新测量', async () => {
    mockInvoke({ response: MOCK_BATTERY_HISTORY_24H });
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);
    await waitFor(() => expect(document.querySelector('.battery-chart')).not.toBeNull());
    const metrics: ScrollMetrics = { scrollHeight: 200, clientHeight: 200, scrollTop: 0 };
    const el = injectBatteryScrollMetrics(metrics);
    triggerScroll(el);
    await waitForRaf();
    expect(el.classList.contains('scroll-fade-bottom')).toBe(false);
    // 模拟 CSS height transition 结束：内容变高
    metrics.scrollHeight = 500;
    // 派发 transition 事件（target 为容器内子元素）
    const content = document.querySelector('.battery-usage-scroll-content') as HTMLElement;
    const transitionEvent = new TransitionEvent('transitionend', { bubbles: true });
    Object.defineProperty(transitionEvent, 'target', { value: content, configurable: true });
    el.dispatchEvent(transitionEvent);
    await waitFor(() => expect(el.classList.contains('scroll-fade-bottom')).toBe(true), { timeout: 2000 });
  });
});

// ─── onLoadingChange 生命周期（P0-1 收口）────────────────────────────────
// 内部 loading 与对外 reportedLoading 分离：弹窗关闭、历史记录禁用、无电池
// 支持时都不得向调用方残留 loading；视觉反馈只在 Modal 内部表达。
describe('BatteryUsageModal onLoadingChange lifecycle', () => {
  beforeEach(async () => {
    invokeMock.mockReset();
    await i18n.changeLanguage('zh-CN');
  });

  function pendingHistory() {
    const resolvers: Array<(response: BatteryHistoryResponse) => void> = [];
    invokeMock.mockImplementation((command: string) => {
      if (command === 'settings_get') return Promise.resolve(settingsEnabled);
      if (command === 'battery_history_get') {
        return new Promise<BatteryHistoryResponse>((resolve) => {
          resolvers.push(resolve);
        });
      }
      return Promise.resolve(undefined);
    });
    return resolvers;
  }

  it('pending 超过 300ms 时只在电量 Modal 内容面显示 Orb', async () => {
    pendingHistory();
    render(<BatteryUsageModal open onClose={() => {}} hasBattery />);

    await waitFor(
      () => expect(screen.getByTestId('thinking-orb')).toHaveAttribute('data-state', 'solving'),
      { timeout: 1200 },
    );

    const embedded = document.querySelector('.mira-embedded-activity');
    expect(embedded).toBeInTheDocument();
    expect(embedded?.parentElement).toHaveClass('battery-usage-scroll-region');
    expect(document.querySelector('.mira-activity-overlay')).not.toBeInTheDocument();
    expect(document.querySelector('.mira-activity-card')).not.toBeInTheDocument();
    expect(screen.queryByText('正在整理电量记录…')).not.toBeInTheDocument();
  });

  it('A. 请求 pending 时关闭：onLoadingChange 落到 false，settle 后不得重新上报 true', async () => {
    const resolvers = pendingHistory();
    const onLoadingChange = vi.fn();
    const { rerender } = render(
      <BatteryUsageModal open onClose={() => {}} hasBattery onLoadingChange={onLoadingChange} />,
    );
    await waitFor(() => expect(onLoadingChange).toHaveBeenLastCalledWith(true));

    rerender(
      <BatteryUsageModal open={false} onClose={() => {}} hasBattery onLoadingChange={onLoadingChange} />,
    );
    await waitFor(() => expect(onLoadingChange).toHaveBeenLastCalledWith(false));
    const trueCountAtClose = onLoadingChange.mock.calls.filter(([v]) => v).length;

    // 请求随后 settle：不得重新报告 true。
    resolvers.forEach((resolve) => resolve(MOCK_BATTERY_HISTORY_24H));
    await act(async () => { await Promise.resolve(); });
    expect(onLoadingChange.mock.calls.filter(([v]) => v)).toHaveLength(trueCountAtClose);
    expect(onLoadingChange).toHaveBeenLastCalledWith(false);
  });

  it('B. stale 后 history disabled 重新打开：不报告 loading / 不产生 battery-analysis', async () => {
    pendingHistory();
    const onLoadingChange = vi.fn();
    const { rerender } = render(
      <BatteryUsageModal open onClose={() => {}} hasBattery onLoadingChange={onLoadingChange} />,
    );
    await waitFor(() => expect(onLoadingChange).toHaveBeenLastCalledWith(true));

    rerender(
      <BatteryUsageModal open={false} onClose={() => {}} hasBattery onLoadingChange={onLoadingChange} />,
    );
    await waitFor(() => expect(onLoadingChange).toHaveBeenLastCalledWith(false));
    const trueCountAtClose = onLoadingChange.mock.calls.filter(([v]) => v).length;

    // 重新打开但历史记录已禁用：内部 loading 即使残留，对外也必须保持 false。
    rerender(
      <BatteryUsageModal
        open
        onClose={() => {}}
        hasBattery
        batteryHistoryEnabled={false}
        aiAnalysisEnabled={false}
        onLoadingChange={onLoadingChange}
      />,
    );
    await act(async () => { await Promise.resolve(); });
    expect(onLoadingChange.mock.calls.filter(([v]) => v)).toHaveLength(trueCountAtClose);
    expect(onLoadingChange).toHaveBeenLastCalledWith(false);
  });

it('C. stale 后不支持电池重新打开：不重新上报', async () => {
    pendingHistory();
    const onLoadingChange = vi.fn();
    const { rerender } = render(
      <BatteryUsageModal open onClose={() => {}} hasBattery onLoadingChange={onLoadingChange} />,
    );
    await waitFor(() => expect(onLoadingChange).toHaveBeenLastCalledWith(true));

    rerender(
      <BatteryUsageModal open={false} onClose={() => {}} hasBattery onLoadingChange={onLoadingChange} />,
    );
    await waitFor(() => expect(onLoadingChange).toHaveBeenLastCalledWith(false));
    const trueCountAtClose = onLoadingChange.mock.calls.filter(([v]) => v).length;

    // 重新打开但无电池且非纯 Web：内部 loading 残留也不得上报。
    rerender(
      <BatteryUsageModal open onClose={() => {}} hasBattery={false} onLoadingChange={onLoadingChange} />,
    );
    await act(async () => { await Promise.resolve(); });
    expect(onLoadingChange.mock.calls.filter(([v]) => v)).toHaveLength(trueCountAtClose);
  });

  it('D. 正常加载：start → true，settle → false', async () => {
    mockInvoke({ response: MOCK_BATTERY_HISTORY_24H });
    const onLoadingChange = vi.fn();
    render(<BatteryUsageModal open onClose={() => {}} hasBattery onLoadingChange={onLoadingChange} />);
    await waitFor(() => expect(onLoadingChange).toHaveBeenLastCalledWith(true));
    await waitFor(() => expect(onLoadingChange).toHaveBeenLastCalledWith(false));
    expect(onLoadingChange.mock.calls.filter(([v]) => v)).toHaveLength(1);
  });

  it('E. 单边失败（24h 成功 / 10d 失败）：settle 后仍为 false', async () => {
    invokeMock.mockImplementation((cmd: string, payload?: { range?: string }) => {
      if (cmd === 'settings_get') return Promise.resolve(settingsEnabled);
      if (cmd === 'battery_history_get' && payload?.range === '10d') {
        return Promise.reject(new Error('10d unavailable'));
      }
      if (cmd === 'battery_history_get') return Promise.resolve(MOCK_BATTERY_HISTORY_24H);
      return Promise.resolve(undefined);
    });
    const onLoadingChange = vi.fn();
    render(<BatteryUsageModal open onClose={() => {}} hasBattery onLoadingChange={onLoadingChange} />);
    await waitFor(() => expect(onLoadingChange).toHaveBeenLastCalledWith(true));
    await waitFor(() => expect(onLoadingChange).toHaveBeenLastCalledWith(false));
    expect(onLoadingChange.mock.calls.filter(([v]) => v)).toHaveLength(1);
  });
});
