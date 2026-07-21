// SPDX-License-Identifier: AGPL-3.0-or-later
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
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
    expect(screen.getAllByText('82%')).toHaveLength(1);
    fireEvent.click(document.querySelector('.battery-state') as HTMLButtonElement);
    expect(screen.getAllByText('82%')).toHaveLength(2);
    expect(screen.getByLabelText('当前 DPI：1000，点击编辑')).toBeInTheDocument();
    const dpiItems = [...document.querySelectorAll<HTMLElement>('.dpi-stage-item')];
    expect(dpiItems[0]?.style.getPropertyValue('--dpi-stage-delay')).toBe('60ms');
    expect(dpiItems[1]?.style.getPropertyValue('--dpi-stage-delay')).toBe('86ms');
    const lightingControlTab = screen.getByRole('tab', { name: '灯光' });
    fireEvent.click(lightingControlTab);
    const lightingTabs = screen.getByRole('tablist', { name: '灯光对象' });
    expect(lightingTabs).toHaveAttribute('data-active-index', '0');
    expect(lightingTabs.style.getPropertyValue('--segmented-indicator-left')).toBe('calc(0% + 3px)');
    fireEvent.click(screen.getByRole('tab', { name: '接收器灯光' }));
    expect(lightingTabs).toHaveAttribute('data-active-index', '1');
    expect(lightingTabs.style.getPropertyValue('--segmented-indicator-left')).toBe('calc(50% + 1.5px)');
    expect(lightingTabs.style.getPropertyValue('--segmented-indicator-accent')).toBe('#4BBFB1');
    expect(document.documentElement.style.getPropertyValue('--accent')).toBe('#ffb3b3');
    expect(screen.queryByText('fixture-verified')).not.toBeInTheDocument();
    expect(document.querySelector('[data-animation="realtime-deformation"]')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '全部读数' }));
    expect(screen.getByRole('dialog', { name: '全部读数' })).toBeInTheDocument();
    expect(screen.getByText('传感器与连接')).toBeInTheDocument();
    expect(screen.getByText('按键映射')).toBeInTheDocument();
    expect(screen.getByText('接收器灯光固件')).toBeInTheDocument();
    const detailsDialog = screen.getByRole('dialog', { name: '全部读数' });
    const detailsHeader = detailsDialog.querySelector(':scope > header');
    expect(detailsHeader).toBeInTheDocument();
    expect(within(detailsHeader as HTMLElement).getByRole('button', { name: '关闭读数详情' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '关闭读数详情' }));
    expect(screen.queryByRole('dialog', { name: '全部读数' })).not.toBeInTheDocument();
  });
  it('does not crossfade color codes when switching lighting zones', () => {
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));

    const mouseColorValue = document.querySelector<HTMLElement>('.lighting-group-mouse .color-value')!;
    expect(mouseColorValue.querySelector('.live-value-current')).toHaveTextContent('#ffb3b3');

    fireEvent.click(screen.getByRole('tab', { name: '接收器灯光' }));

    const receiverColorValue = document.querySelector<HTMLElement>('.lighting-group-receiver .color-value')!;
    expect(document.body.contains(mouseColorValue)).toBe(false);
    expect(receiverColorValue.querySelector('.live-value-current')).toHaveTextContent('#4BBFB1');
    expect(receiverColorValue.querySelector('.live-value-next')).not.toBeInTheDocument();
  });
  it('opens the active lighting color editor from the color indicator', () => {
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));

    fireEvent.click(screen.getByRole('button', { name: '颜色' }));
    const mouseColorDialog = screen.getByRole('dialog', { name: '鼠标灯光颜色' });
    expect(mouseColorDialog).toBeInTheDocument();
    expect(within(mouseColorDialog).getByLabelText('颜色')).toHaveValue('#ffb3b3');
    fireEvent.click(screen.getByRole('button', { name: '取消' }));

    fireEvent.click(screen.getByRole('tab', { name: '接收器灯光' }));
    fireEvent.click(screen.getByRole('button', { name: '颜色' }));
    const receiverColorDialog = screen.getByRole('dialog', { name: '编辑接收器颜色' });
    expect(receiverColorDialog).toBeInTheDocument();
    expect(within(receiverColorDialog).getByLabelText('颜色')).toHaveValue('#4bbfb1');
  });
  it('reuses the metric and table geometry while switching dashboard controls', async () => {
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));

    const metricLayer = document.querySelector('.shared-control-metric');
    expect(metricLayer).toHaveAttribute('data-variant', 'dpi');
    expect(metricLayer).toHaveAttribute('data-visible', 'true');
    expect(metricLayer).toHaveAttribute('data-positioned', 'true');
    const metricText = metricLayer?.querySelector('.shared-control-metric-text');
    const surfaceLayer = document.querySelector('.shared-control-surface');
    expect(surfaceLayer).toHaveAttribute('data-visible', 'false');
    const contextLayer = document.querySelector('.shared-control-context');
    expect(contextLayer).toHaveAttribute('data-visible', 'false');

    fireEvent.click(screen.getByRole('tab', { name: '回报率' }));
    const stage = document.querySelector('.control-stage')!;
    expect(stage).toHaveAttribute('data-control-mode', 'polling');
    expect(document.querySelector('.shared-control-metric')).toBe(metricLayer);
    expect(metricLayer?.querySelector('.shared-control-metric-text')).toBe(metricText);
    expect(metricLayer).toHaveAttribute('data-variant', 'hertz');
    expect(metricLayer).toHaveAttribute('data-positioned', 'true');
    let incomingMetricFace: Element | null = null;
    await waitFor(() => {
      const metricValue = metricLayer?.querySelector('.shared-control-metric-value');
      expect(metricValue).toHaveAttribute('data-transition', 'crossfade');
      incomingMetricFace = metricValue?.querySelector('.shared-control-metric-face.is-next') ?? null;
      expect(incomingMetricFace).toBeInTheDocument();
    });
    await waitFor(() => {
      expect(metricLayer?.querySelector('.shared-control-metric-face.is-next')).not.toBeInTheDocument();
    });
    expect(metricLayer?.querySelector('.shared-control-metric-face.is-current')).toBe(incomingMetricFace);
    expect(document.querySelector('.shared-control-context')).toBe(contextLayer);
    expect(contextLayer).toHaveAttribute('data-visible', 'true');
    expect(contextLayer).toHaveAttribute('data-sync', 'metric');
    expect(contextLayer).toHaveAttribute('data-positioned', 'true');
    expect(contextLayer).toHaveTextContent('当前回报率');
    expect(document.querySelector('.shared-control-surface')).toBe(surfaceLayer);
    expect(surfaceLayer).toHaveAttribute('data-kind', 'summary');
    expect(surfaceLayer).toHaveAttribute('data-visible', 'true');
    expect(surfaceLayer).toHaveAttribute('data-positioned', 'true');
    const summaryDelays = [...document.querySelectorAll<HTMLElement>('.capability-summary > .secondary-control-item')]
      .map((item) => Number.parseInt(item.style.getPropertyValue('--control-detail-delay'), 10));
    expect(summaryDelays.length).toBeGreaterThan(1);
    expect(summaryDelays.every((delay) => delay >= 165 && delay < 210)).toBe(true);
    expect(new Set(summaryDelays).size).toBeGreaterThan(1);

    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    expect(document.querySelector('.shared-control-metric')).toBe(metricLayer);
    expect(metricLayer).toHaveAttribute('data-visible', 'false');
    expect(document.querySelector('.shared-control-context')).toBe(contextLayer);
    expect(contextLayer).toHaveAttribute('data-visible', 'false');
    expect(contextLayer).toHaveAttribute('data-sync', 'surface');
    expect(contextLayer).toHaveTextContent('当前回报率');
    expect(document.querySelector('.shared-control-surface')).toBe(surfaceLayer);
    expect(surfaceLayer).toHaveAttribute('data-kind', 'lighting');
    expect(surfaceLayer).toHaveAttribute('data-positioned', 'true');
    const lightingDelays = [...document.querySelectorAll<HTMLElement>('.lighting-row-slot.secondary-control-item')]
      .map((item) => Number.parseInt(item.style.getPropertyValue('--control-detail-delay'), 10));
    expect(lightingDelays.length).toBeGreaterThan(1);
    expect(lightingDelays.every((delay) => delay >= 165 && delay < 210)).toBe(true);
    expect(new Set(lightingDelays).size).toBeGreaterThan(1);

    fireEvent.click(screen.getByRole('tab', { name: '回报率' }));
    expect(document.querySelector('.shared-control-metric')).toBe(metricLayer);
    expect(metricLayer).toHaveAttribute('data-visible', 'true');
    expect(metricLayer).toHaveAttribute('data-sync', 'surface');
    expect(document.querySelector('.shared-control-context')).toBe(contextLayer);
    expect(contextLayer).toHaveAttribute('data-visible', 'true');
    expect(contextLayer).toHaveAttribute('data-sync', 'surface');
    expect(document.querySelector('.shared-control-surface')).toBe(surfaceLayer);
  });
  it('settles on the selected metric after rapid DPI and polling switches', async () => {
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));

    const dpiTab = screen.getByRole('tab', { name: 'DPI' });
    const pollingTab = screen.getByRole('tab', { name: '回报率' });
    fireEvent.click(pollingTab);
    fireEvent.click(dpiTab);
    fireEvent.click(pollingTab);

    const metricLayer = document.querySelector('.shared-control-metric')!;
    expect(document.querySelector('.control-stage')).toHaveAttribute('data-control-mode', 'polling');
    await waitFor(() => {
      expect(metricLayer.querySelector('.shared-control-metric-face.is-next')).not.toBeInTheDocument();
      expect(metricLayer.querySelector('.shared-control-metric-face.is-current')).toHaveTextContent('1000Hz');
    });
  });
  it('commits an edited metric on the real flip boundary', async () => {
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));

    fireEvent.click(screen.getByRole('button', { name: '当前 DPI：1000，点击编辑' }));
    let dialog = await screen.findByRole('dialog', { name: '编辑第 3 档 DPI' });
    fireEvent.change(within(dialog).getByLabelText('DPI 数值'), { target: { value: '9300' } });
    fireEvent.click(within(dialog).getByRole('button', { name: '应用' }));

    const metricValue = document.querySelector('.shared-control-metric-value')!;
    let terminalDigit: Element | null = null;
    await waitFor(() => {
      terminalDigit = metricValue.querySelector('[data-flip-last="true"]');
      expect(terminalDigit).toBeInTheDocument();
    });
    fireEvent.animationEnd(terminalDigit!, { animationName: 'metric-digit-settle' });
    await waitFor(() => {
      expect(metricValue.querySelector('.shared-control-metric-face.is-current')).toHaveTextContent('9300DPI');
    });

    fireEvent.click(screen.getByRole('button', { name: '当前 DPI：9300，点击编辑' }));
    dialog = await screen.findByRole('dialog', { name: '编辑第 3 档 DPI' });
    fireEvent.change(within(dialog).getByLabelText('DPI 数值'), { target: { value: '500' } });
    fireEvent.click(within(dialog).getByRole('button', { name: '应用' }));

    let incomingFace: Element | null = null;
    await waitFor(() => {
      expect(metricValue).toHaveAttribute('data-transition', 'flip');
      incomingFace = metricValue.querySelector('.shared-control-metric-face.is-next');
      expect(incomingFace?.querySelector('.metric-flip-sizer')).toHaveTextContent('500');
      expect(incomingFace?.querySelector('em')).toHaveTextContent('DPI');
    });

    const digitSlots = [...incomingFace!.querySelectorAll<HTMLElement>('.metric-flip-digit')];
    expect(digitSlots).toHaveLength(4);
    expect([...digitSlots[0].querySelectorAll('.metric-flip-digit-face')].map((face) => face.textContent))
      .toEqual(['9', '8', '7', '6', '5', '4', '3', '2', '1', '0', ' ']);
    expect([...digitSlots[1].querySelectorAll('.metric-flip-digit-face')].map((face) => face.textContent))
      .toEqual(['3', '4', '5']);
    expect([...digitSlots[2].querySelectorAll('.metric-flip-digit-face')].map((face) => face.textContent))
      .toEqual(['0']);
    expect([...digitSlots[3].querySelectorAll('.metric-flip-digit-face')].map((face) => face.textContent))
      .toEqual(['0']);
    expect(digitSlots[2].querySelector('.metric-flip-digit-face')).toHaveClass('is-static');
    expect(digitSlots[3].querySelector('.metric-flip-digit-face')).toHaveClass('is-static');
    const lastFinalDigit = incomingFace!.querySelector('[data-flip-last="true"]');
    expect(lastFinalDigit).toBeInTheDocument();
    expect(lastFinalDigit?.textContent).toBe(' ');
    expect(lastFinalDigit).toHaveAttribute('style', '--metric-digit-delay: 520ms;');
    fireEvent.animationEnd(lastFinalDigit!, { animationName: 'metric-digit-settle' });
    await waitFor(() => {
      expect(metricValue.querySelector('.shared-control-metric-face.is-next')).not.toBeInTheDocument();
    });
    expect(metricValue.querySelector('.shared-control-metric-face.is-current')).toBe(incomingFace);
    expect(metricValue.querySelector('.shared-control-metric-face.is-current')).toHaveTextContent('500DPI');
    expect(document.querySelector('.primary-reading > .live-value')).not.toBeInTheDocument();
    expect(document.querySelector('.primary-reading > strong')).toHaveTextContent('500');
  });
  it('shows the multi-mouse switcher in the demo fixture', () => {
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));
    expect(screen.getByRole('heading', { name: 'Mira Example Wireless Mouse' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '切换鼠标' }));
    fireEvent.click(screen.getByText('Mira Example USB Mouse').closest('button')!);
    expect(screen.getByRole('heading', { name: 'Mira Example USB Mouse' })).toBeInTheDocument();
    expect(screen.getAllByRole('tab').map((tab) => tab.textContent))
      .toEqual(['配置控制', 'DPI', '回报率', '灯光']);
    expect(screen.getByRole('tab', { name: '配置控制' })).toHaveAttribute('aria-selected', 'true');
    fireEvent.click(screen.getByRole('tab', { name: 'DPI' }));
    expect(screen.getByLabelText('当前 DPI：1600，点击编辑')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));
    fireEvent.click(screen.getByRole('tab', { name: '配置控制' }));
    const stage = document.querySelector('.control-stage')!;
    expect(stage).toHaveAttribute('data-control-transition', 'standard-to-segmented');
    expect(stage.querySelector('.control-stage-page.is-leaving')).not.toBeInTheDocument();
    const enteringPage = stage.querySelector('.control-stage-page.is-entering')!;
    expect(enteringPage).toHaveAttribute('data-page-kind', 'segmented');
    expect(within(enteringPage as HTMLElement).getByRole('group', { name: '配置控制' })).toBeInTheDocument();
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
  it('remembers the active settings tab when returning from the about page', () => {
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    // 切到设置页的「关于」标签，再进入完整的关于页
    fireEvent.click(screen.getByRole('button', { name: '关于' }));
    fireEvent.click(screen.getByRole('button', { name: '打开关于页' }));
    // 从关于页返回后，应停留在原先的「关于」标签，而非每次都落回首个标签
    fireEvent.click(screen.getByRole('button', { name: '返回' }));
    expect(screen.getByRole('heading', { name: '设置' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '关于' })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('button', { name: '通用' })).toHaveAttribute('aria-pressed', 'false');
  });
  it('applies demo mutations locally without calling device_mutate or showing errors', async () => {
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));
    fireEvent.click(screen.getByRole('tab', { name: '回报率' }));

    fireEvent.click(screen.getByRole('button', { name: '当前回报率：1000 Hz，点击编辑' }));
    const dialog = await screen.findByRole('dialog', { name: '设置回报率' });
    fireEvent.change(within(dialog).getByLabelText('回报率'), { target: { value: '500' } });
    fireEvent.click(within(dialog).getByRole('button', { name: '应用' }));

    // 演示模式下不应调用真实 Tauri device_mutate
    expect(invokeMock).not.toHaveBeenCalledWith('device_mutate', expect.anything());
    // 不应弹出错误通知
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    // 应该看到「搞定啦」成功通知
    expect(await screen.findByText('搞定啦')).toBeInTheDocument();
    // UI 反映新的回报率
    expect(screen.getByRole('button', { name: '当前回报率：500 Hz，点击编辑' })).toBeInTheDocument();
  });
});
