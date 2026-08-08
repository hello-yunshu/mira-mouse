// SPDX-License-Identifier: AGPL-3.0-or-later
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import { notifyError } from './notify';
import { announceAttentionRequest, resetAttentionBusForTests } from './attention';

const { invokeMock, currentWindowMock, windowHandlers } = vi.hoisted(() => {
  const handlers: {
    focus?: (event: { payload: boolean }) => void;
    resized?: (event: { payload: { width: number; height: number } }) => void;
  } = {};
  return {
    invokeMock: vi.fn(),
    windowHandlers: handlers,
    currentWindowMock: {
      isVisible: vi.fn(),
      isMinimized: vi.fn(),
      onFocusChanged: vi.fn(),
      onResized: vi.fn(),
      minimize: vi.fn(),
    },
  };
});
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => currentWindowMock }));
vi.mock('thinking-orbs', () => ({
  ThinkingOrb: ({ state }: { state: string }) => (
    <span data-testid="thinking-orb" data-state={state} />
  ),
}));

const originalUserAgent = navigator.userAgent;

beforeEach(() => {
  invokeMock.mockRejectedValue(new Error('not mocked'));
  currentWindowMock.isVisible.mockReset().mockResolvedValue(true);
  currentWindowMock.isMinimized.mockReset().mockResolvedValue(false);
  currentWindowMock.onFocusChanged.mockReset().mockImplementation(async (
    handler: (event: { payload: boolean }) => void,
  ) => {
    windowHandlers.focus = handler;
    return vi.fn();
  });
  currentWindowMock.onResized.mockReset().mockImplementation(async (
    handler: (event: { payload: { width: number; height: number } }) => void,
  ) => {
    windowHandlers.resized = handler;
    return vi.fn();
  });
  currentWindowMock.minimize.mockReset().mockResolvedValue(undefined);
  delete windowHandlers.focus;
  delete windowHandlers.resized;
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
  it('pauses the dashboard aura while minimized even when the native window stays visible', async () => {
    render(<App />);
    await waitFor(() => expect(windowHandlers.focus).toBeTypeOf('function'));
    await waitFor(() => expect(windowHandlers.resized).toBeTypeOf('function'));

    // macOS 最小化后 isVisible 仍可能为 true；focus 回调还可能早于 minimized 状态落定。
    windowHandlers.focus?.({ payload: false });
    currentWindowMock.isMinimized.mockResolvedValue(true);
    await act(async () => {
      windowHandlers.resized?.({ payload: { width: 0, height: 0 } });
    });

    await waitFor(() => expect(document.querySelector('.device-aura')).toHaveClass('is-paused'));

    currentWindowMock.isMinimized.mockResolvedValue(false);
    await act(async () => {
      windowHandlers.focus?.({ payload: true });
    });
    await waitFor(() => expect(document.querySelector('.device-aura')).not.toHaveClass('is-paused'));
  });
  it('freezes the exact aura frame in the leaving page snapshot', async () => {
    render(<App />);
    const aura = document.querySelector<HTMLElement>('.device-aura')!;
    expect(aura.style.getPropertyValue('--aura-phase-offset')).toMatch(/^-\d+ms$/);

    const cloud = aura.querySelector<HTMLElement>('.aura-cloud-1')!;
    cloud.style.transform = 'translate(7px, -3px) scale(1.08)';
    cloud.style.opacity = '0.67';
    cloud.style.borderRadius = '55% 45% 60% 40%';
    const stars = aura.querySelector<HTMLElement>('.aura-stars')!;
    stars.style.opacity = '0.82';
    const star = stars.querySelector<HTMLElement>('.aura-star-2')!;
    star.style.transform = 'scale(1.14)';
    star.style.opacity = '0.74';

    fireEvent.click(screen.getByRole('button', { name: '设置' }));

    await waitFor(() => expect(document.querySelector('.page-layer-leaving .device-aura')).toBeInTheDocument());
    const frozenCloud = document.querySelector<HTMLElement>('.page-layer-leaving .aura-cloud-1')!;
    expect(frozenCloud.style.animation).toBe('none');
    expect(frozenCloud.style.transform).toBe('translate(7px, -3px) scale(1.08)');
    expect(frozenCloud.style.opacity).toBe('0.67');
    expect(frozenCloud.style.borderRadius).toBe('55% 45% 60% 40%');
    const frozenStars = document.querySelector<HTMLElement>('.page-layer-leaving .aura-stars')!;
    const frozenStar = frozenStars.querySelector<HTMLElement>('.aura-star-2')!;
    expect(frozenStars.style.animation).toBe('none');
    expect(frozenStars.style.opacity).toBe('0.82');
    expect(frozenStar.style.animation).toBe('none');
    expect(frozenStar.style.transform).toBe('scale(1.14)');
    expect(frozenStar.style.opacity).toBe('0.74');
  });
  it('freezes an in-progress settings card before the page fades out', async () => {
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: '设置' }));

    const card = document.querySelector<HTMLElement>('.settings-scroll-content > section')!;
    card.style.transform = 'translateY(3px) scale(0.99)';
    card.style.opacity = '0.42';
    card.style.filter = 'saturate(0.91)';

    fireEvent.click(screen.getByRole('button', { name: '关于 Mira' }));

    await waitFor(() => expect(document.querySelector('.page-layer-leaving .settings-scroll-content > section')).toBeInTheDocument());
    const frozenCard = document.querySelector<HTMLElement>('.page-layer-leaving .settings-scroll-content > section')!;
    expect(frozenCard.style.animation).toBe('none');
    expect(frozenCard.style.transform).toBe('translateY(3px) scale(0.99)');
    expect(frozenCard.style.opacity).toBe('0.42');
    expect(frozenCard.style.filter).toBe('saturate(0.91)');
  });
  it('keeps one persistent Mira Mouse eyebrow while switching settings and about', () => {
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: '设置' }));

    const eyebrow = document.querySelector('.page-persistent-eyebrow');
    expect(eyebrow).toHaveTextContent('Mira Mouse');

    fireEvent.click(screen.getByRole('button', { name: '关于' }));
    expect(document.querySelector('.page-persistent-eyebrow')).toBe(eyebrow);
    expect(document.querySelector('.page-persistent-eyebrow')).toHaveTextContent('Mira Mouse');
  });
  it('crossfades page titles through one fixed title slot', async () => {
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: '设置' }));

    const titleSlot = document.querySelector('.page-persistent-title');
    const settingsFace = titleSlot?.querySelector('.page-persistent-title-face');
    expect(settingsFace).toHaveTextContent('设置');
    expect(settingsFace).toHaveClass('is-active');
    expect(settingsFace).toHaveAttribute('data-title', '设置');

    fireEvent.click(screen.getByRole('button', { name: '关于 Mira' }));
    expect(document.querySelector('.page-persistent-title')).toBe(titleSlot);
    await waitFor(() => expect(titleSlot?.querySelectorAll('.page-persistent-title-face')).toHaveLength(2));
    const aboutFace = [...titleSlot!.querySelectorAll('.page-persistent-title-face')]
      .find((face) => face.textContent === '关于');
    await waitFor(() => expect(aboutFace).toHaveClass('is-active'));
    expect(aboutFace).toHaveClass('is-forming');
    expect(aboutFace).toHaveAttribute('data-title', '关于');
    expect(settingsFace).not.toHaveClass('is-active');
    expect(settingsFace).toHaveClass('is-exiting');
    await waitFor(() => expect(titleSlot?.querySelectorAll('.page-persistent-title-face')).toHaveLength(1));
    expect(titleSlot?.querySelector('.page-persistent-title-face')).toHaveTextContent('关于');
  });
  it('settles rapid title reversals without replaying or stacking faces', async () => {
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: '设置' }));

    const titleSlot = document.querySelector('.page-persistent-title');
    const settingsFace = titleSlot?.querySelector('.page-persistent-title-face');
    fireEvent.click(screen.getByRole('button', { name: '关于 Mira' }));
    await waitFor(() => expect(titleSlot?.querySelectorAll('.page-persistent-title-face')).toHaveLength(2));
    await waitFor(() => expect(settingsFace).not.toHaveClass('is-active'));
    fireEvent.click(screen.getByRole('button', { name: '设置' }));

    await waitFor(() => expect(settingsFace).toHaveClass('is-active'));
    await waitFor(() => expect(titleSlot?.querySelectorAll('.page-persistent-title-face')).toHaveLength(1));
    expect(titleSlot?.querySelector('.page-persistent-title-face')).toBe(settingsFace);
  });
  it('routes logs through the shared page transition while keeping Settings active', async () => {
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    fireEvent.click(await screen.findByRole('button', { name: '关于' }));
    fireEvent.click(await screen.findByRole('button', { name: '打开日志与诊断' }));

    expect(document.querySelector('.page-swap')).toHaveAttribute('data-page-view', 'logs');
    expect(document.querySelector('.page-swap')).toHaveClass('is-transitioning');
    expect(document.querySelector('.page-layer-leaving')).toBeInTheDocument();
    expect(await screen.findByRole('heading', { name: '日志与诊断' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '设置' })).toHaveClass('active');

    fireEvent.click(screen.getByRole('button', { name: '返回' }));
    expect(document.querySelector('.page-swap')).toHaveAttribute('data-page-view', 'settings');
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
  it('can hold the writing status open for visual review in the web preview', () => {
    window.history.pushState({}, '', '?preview=writing');
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));
    expect(screen.getByText('正在和鼠标沟通…')).toBeInTheDocument();
    expect(screen.getByTestId('thinking-orb')).toHaveAttribute('data-state', 'working');
    expect(document.querySelector('.mira-inline-activity')).toHaveClass('is-visible');
    expect(document.querySelector('.control-stage')).toHaveClass('has-preview-message');
  });
  it('never renders a device Beam over the dashboard control stage', () => {
    resetAttentionBusForTests();
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));

    act(() => {
      announceAttentionRequest({
        eventKey: 'device-reconnected:test:1',
        scope: 'device:app',
        variant: 'line',
        color: '#ffb3b3',
        durationMs: 1100,
        strength: 0.6,
        cycles: 1,
        priority: 60,
      });
    });

    expect(document.querySelector('.control-stage .attention-beam')).not.toBeInTheDocument();
    resetAttentionBusForTests();
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
    expect(screen.getByLabelText('当前 DPI：1600，点击编辑')).toBeInTheDocument();
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
  it('updates the lighting swatch color when switching lighting zones', () => {
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));
    fireEvent.click(screen.getByRole('tab', { name: '灯光' }));

    // 灯带位于 lighting-group 上方，通过 --light-color CSS 变量承载当前区域颜色。
    const swatch = document.querySelector<HTMLElement>('.lighting-swatch')!;
    expect(swatch.style.getPropertyValue('--light-color')).toBe('#ffb3b3');

    fireEvent.click(screen.getByRole('tab', { name: '接收器灯光' }));

    // 切换区域后同一 swatch 元素的 CSS 变量更新为接收器灯光色，无残留旧值。
    expect(swatch.style.getPropertyValue('--light-color')).toBe('#4BBFB1');
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
    const metricValue = metricLayer?.querySelector('.shared-control-metric-value');
    expect(metricValue?.querySelector('.shared-control-metric-face.is-current')).toHaveTextContent('1600DPI');
    expect(metricValue?.querySelector('.shared-control-metric-face.is-next')).not.toBeInTheDocument();
    let incomingMetricFace: Element | null = null;
    await waitFor(() => {
      expect(metricValue).toHaveAttribute('data-transition', 'flip');
      incomingMetricFace = metricValue?.querySelector('.shared-control-metric-face.is-next') ?? null;
      expect(incomingMetricFace).toBeInTheDocument();
    });
    const pollingTerminalDigit = metricValue?.querySelector(
      '.shared-control-metric-face.is-next [data-flip-last="true"]',
    );
    expect(pollingTerminalDigit).toBeInTheDocument();
    fireEvent.animationEnd(pollingTerminalDigit!, { animationName: 'metric-digit-settle' });
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

    fireEvent.click(screen.getByRole('tab', { name: 'DPI' }));
    expect(metricValue?.querySelector('.shared-control-metric-face.is-current')).toHaveTextContent('1000Hz');
    expect(metricValue?.querySelector('.shared-control-metric-face.is-next')).not.toBeInTheDocument();
    await waitFor(() => {
      expect(metricValue?.querySelector('.shared-control-metric-face.is-current')).toHaveTextContent('1600DPI');
    });

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
    // mouse zone 仅 status 一个子块（color 由灯带渲染），切换到接收器灯光验证多子块错落延迟。
    fireEvent.click(screen.getByRole('tab', { name: '接收器灯光' }));
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

    fireEvent.click(screen.getByRole('button', { name: '当前 DPI：1600，点击编辑' }));
    let dialog = await screen.findByRole('dialog', { name: '编辑第 4 档 DPI' });
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
    dialog = await screen.findByRole('dialog', { name: '编辑第 4 档 DPI' });
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
    // 配置控制通过 candidate + optionalPosition=leading 放在核心三项之前；
    // 可见短标签不超过 3 个中文字符，完整名称保留为 tab 的无障碍名称。
    expect(screen.getAllByRole('tab').map((tab) => tab.textContent))
      .toEqual(['配置', 'DPI', '回报率', '灯光']);
    expect(screen.getAllByRole('tab').every((tab) => Array.from(tab.textContent ?? '').length <= 3))
      .toBe(true);
    expect(screen.getByRole('tab', { name: '配置控制' })).toHaveTextContent('配置');
    expect(screen.getByRole('tab', { name: '配置控制' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('group', { name: '配置控制' })).toBeInTheDocument();

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
  it('remembers the active settings tab when returning from the about page', async () => {
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    // 切到设置页的「关于」标签，再进入完整的关于页
    fireEvent.click(screen.getByRole('button', { name: '关于' }));
    fireEvent.click(await screen.findByRole('button', { name: '打开关于页' }));
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

// P0-B: DPI/回报率数字翻牌时序测试
// 上下文切换时翻牌在缩放早期（contextTransitionDelay=50ms）启动。
// 缓动曲线 cubic-bezier(.32, .72, 0, 1) 极激进，50ms 时缩放已推进约
// 60% 视觉距离，翻牌与缩放剩余部分并行，入场自然融入缩放过程。
describe('MorphingMetricValue context flip timing (P0-B)', () => {
  beforeEach(() => {
    invokeMock.mockRejectedValue(new Error('not mocked'));
  });
  afterEach(() => {
    invokeMock.mockReset();
    vi.useRealTimers();
  });

  it('does not start flip before 50ms (49ms still idle)', async () => {
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));

    const metricValue = document.querySelector('.shared-control-metric-value')!;
    expect(metricValue.querySelector('.shared-control-metric-face.is-current')).toHaveTextContent('1600DPI');

    // 切换到回报率（metric-to-metric context change）
    vi.useFakeTimers();
    fireEvent.click(screen.getByRole('tab', { name: '回报率' }));

    // 49ms 时：尚未开始翻牌，next face 不存在
    await vi.advanceTimersByTimeAsync(49);
    expect(metricValue.querySelector('.shared-control-metric-face.is-next')).not.toBeInTheDocument();
  });

  it('starts flip at ~50ms during zoom (is-transitioning appears)', async () => {
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));

    const metricValue = document.querySelector('.shared-control-metric-value')!;

    vi.useFakeTimers();
    fireEvent.click(screen.getByRole('tab', { name: '回报率' }));

    // 50ms 时：setTimeout 触发 prepareTransition，rAF 注册
    await vi.advanceTimersByTimeAsync(50);
    // 推进 rAF 队列让 next face 和 is-transitioning 生效（rAF 嵌套 2 帧 + 渲染）
    await vi.advanceTimersByTimeAsync(60);
    expect(metricValue.querySelector('.shared-control-metric-face.is-next')).toBeInTheDocument();
    expect(metricValue).toHaveClass('is-transitioning');
  });

  it('starts flip well before zoom completes (50ms << 340ms geometry)', async () => {
    // 缩放几何变形时长 340ms，翻牌在 50ms（缩放早期）启动，
    // 与缩放大部分时段重叠，而不是等缩放完全结束。
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));

    const metricValue = document.querySelector('.shared-control-metric-value')!;

    vi.useFakeTimers();
    fireEvent.click(screen.getByRole('tab', { name: '回报率' }));

    // 推进到 120ms：翻牌已开始；缩放（340ms）仍在进行
    await vi.advanceTimersByTimeAsync(120);
    await vi.advanceTimersByTimeAsync(60); // 推进 rAF
    expect(metricValue).toHaveClass('is-transitioning');
  });

  it('overlaps flip with most of the zoom (~290ms shared)', async () => {
    // 翻牌在 50ms 启动，缩放在 340ms 结束，重叠约 290ms。
    // 验证 50ms 时翻牌已开始（缩放还有大量视觉距离）。
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));

    const metricValue = document.querySelector('.shared-control-metric-value')!;

    vi.useFakeTimers();
    fireEvent.click(screen.getByRole('tab', { name: '回报率' }));

    // 推进 50ms + rAF：翻牌已开始（此时缩放还有 ~290ms 才结束）
    await vi.advanceTimersByTimeAsync(50);
    await vi.advanceTimersByTimeAsync(60);
    expect(metricValue).toHaveClass('is-transitioning');
  });

  it('cancels old animation on rapid reverse DPI -> polling -> DPI', async () => {
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));

    const metricValue = document.querySelector('.shared-control-metric-value')!;
    expect(metricValue.querySelector('.shared-control-metric-face.is-current')).toHaveTextContent('1600DPI');

    vi.useFakeTimers();
    // DPI -> 回报率（contextKey 改变，触发 50ms 延迟翻牌）
    fireEvent.click(screen.getByRole('tab', { name: '回报率' }));
    // 25ms 后（< 50ms，翻牌尚未开始）快速切回 DPI
    await vi.advanceTimersByTimeAsync(25);
    fireEvent.click(screen.getByRole('tab', { name: 'DPI' }));

    // 推进所有 timers，确保旧动画被取消，无残留 nextValue
    await vi.advanceTimersByTimeAsync(600);

    // 不应有残留的 next face（旧动画已取消）
    expect(metricValue.querySelector('.shared-control-metric-face.is-next')).not.toBeInTheDocument();
    // current face 应仍然是 DPI（未因旧 timeout 提交错误值）
    expect(metricValue.querySelector('.shared-control-metric-face.is-current')).toHaveTextContent('1600DPI');
  });

  it('preserves flip behavior under prefers-reduced-motion without crashing', async () => {
    // prefers-reduced-motion 由 CSS 媒体查询处理（styles.css 把动画时长压到 0.01ms），
    // JS 端 contextTransitionDelay 仍为 50ms。验证翻牌时序不受影响且不崩溃。
    // jsdom 不实现 matchMedia，App.tsx 也不依赖它，所以无需 polyfill。
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));

    const metricValue = document.querySelector('.shared-control-metric-value')!;

    vi.useFakeTimers();
    fireEvent.click(screen.getByRole('tab', { name: '回报率' }));

    // 49ms 时未开始（与正常模式相同）
    await vi.advanceTimersByTimeAsync(49);
    expect(metricValue.querySelector('.shared-control-metric-face.is-next')).not.toBeInTheDocument();

    // 50ms 后开始（CSS 动画时长不影响 JS 时序）
    await vi.advanceTimersByTimeAsync(1);
    await vi.advanceTimersByTimeAsync(60);
    expect(metricValue.querySelector('.shared-control-metric-face.is-next')).toBeInTheDocument();
  });

  it('DPI → 回报率 → DPI → 回报率 最终状态正确', async () => {
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));
    const metricValue = document.querySelector('.shared-control-metric-value')!;

    vi.useFakeTimers();
    // DPI → 回报率
    fireEvent.click(screen.getByRole('tab', { name: '回报率' }));
    await act(async () => { await vi.advanceTimersByTimeAsync(700); });

    // 回报率 → DPI
    fireEvent.click(screen.getByRole('tab', { name: 'DPI' }));
    await act(async () => { await vi.advanceTimersByTimeAsync(700); });

    // DPI → 回报率（最终状态）
    fireEvent.click(screen.getByRole('tab', { name: '回报率' }));
    await act(async () => { await vi.advanceTimersByTimeAsync(700); });

    // 最终应为回报率值（非 DPI 残留）
    const currentFace = metricValue.querySelector('.shared-control-metric-face.is-current');
    expect(currentFace).toBeInTheDocument();
    expect(currentFace?.textContent).not.toContain('DPI');
    expect(currentFace?.textContent).toMatch(/Hz/);
    expect(metricValue.querySelector('.shared-control-metric-face.is-next')).not.toBeInTheDocument();
  });

  it('翻牌后最终数字和单位正确', async () => {
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));
    const metricValue = document.querySelector('.shared-control-metric-value')!;

    vi.useFakeTimers();
    // 切换到回报率
    fireEvent.click(screen.getByRole('tab', { name: '回报率' }));
    // 完成翻牌（50ms 延迟 + 动画 + fallback timeout）
    await act(async () => { await vi.advanceTimersByTimeAsync(700); });

    // 最终值应包含 Hz 单位（回报率），不包含 DPI
    const currentFace = metricValue.querySelector('.shared-control-metric-face.is-current');
    expect(currentFace).toBeInTheDocument();
    expect(currentFace?.textContent).toMatch(/\d+\s*Hz/);
    expect(currentFace?.textContent).not.toContain('DPI');
  });

  it('旧 fallback timeout 不得提交过期值', async () => {
    render(<App />);
    fireEvent.click(screen.getByText('查看演示'));
    const metricValue = document.querySelector('.shared-control-metric-value')!;

    vi.useFakeTimers();
    // DPI → 回报率（触发 50ms 延迟翻牌）
    fireEvent.click(screen.getByRole('tab', { name: '回报率' }));
    // 推进到 50ms 后翻牌开始，fallback timeout 已调度
    await act(async () => { await vi.advanceTimersByTimeAsync(50); });
    await act(async () => { await vi.advanceTimersByTimeAsync(60); }); // rAF 推进

    // 快速切回 DPI（应在 fallback timeout 触发前）
    fireEvent.click(screen.getByRole('tab', { name: 'DPI' }));
    // 推进足够时间让旧 fallback timeout 本应触发（如果未被取消）
    await act(async () => { await vi.advanceTimersByTimeAsync(600); });

    // current face 应为 DPI（旧 timeout 未提交回报率的过期值）
    expect(metricValue.querySelector('.shared-control-metric-face.is-current')).toHaveTextContent('1600DPI');
    // 不应有残留的回报率 next face
    expect(metricValue.querySelector('.shared-control-metric-face.is-next')).not.toBeInTheDocument();
  });
});
