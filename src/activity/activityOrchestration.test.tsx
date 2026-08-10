// SPDX-License-Identifier: AGPL-3.0-or-later
// P0-3：Orb 与 Attention Beam 生命周期边界的关键集成测试。
// mock 第三方视觉组件（thinking-orbs），不测试 Canvas 绘制。
import { act, render, screen } from '@testing-library/react';
import { StrictMode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MiraActivityOverlay } from './MiraActivityOverlay';
import { MiraActivityButton } from './MiraActivityButton';
import { MiraEmbeddedActivity } from './MiraEmbeddedActivity';
import { MiraInlineActivity } from './MiraInlineActivity';
import {
  announceAfterOrbExit,
  bumpActivityExitHint,
  isActivityVisible,
  registerVisibleActivity,
  resetActivityCoordinatorForTests,
  unregisterVisibleActivity,
  waitForActivityExit,
} from './activityCoordinator';
import { announceAttentionRequest, finishActiveAttentionRequest, resetAttentionBusForTests } from '../attention/attentionCore';
import type { AttentionBeamRequest } from '../attention/attentionTypes';
import type { MiraActivityKind } from './activityCatalog';

vi.mock('thinking-orbs', () => ({
  ThinkingOrb: ({ state, color }: { state: string; color?: string }) => (
    <span
      data-testid="thinking-orb"
      data-state={state}
      data-color={color}
    />
  ),
}));

function request(
  eventKey: string,
  scope: string,
  priority = 10,
  variant: AttentionBeamRequest['variant'] = 'line',
): AttentionBeamRequest {
  return {
    eventKey,
    scope,
    variant,
    color: '#ffb3b3',
    durationMs: 1200,
    strength: 0.16,
    cycles: 1,
    priority,
  };
}

describe('MiraActivityOverlay 生命周期仲裁（P0-3）', () => {
  beforeEach(() => {
    vi.useFakeTimers({
      toFake: [
        'setTimeout', 'clearTimeout', 'setInterval', 'clearInterval',
        'Date', 'performance', 'requestAnimationFrame', 'cancelAnimationFrame',
      ],
    });
    resetActivityCoordinatorForTests();
    resetAttentionBusForTests();
  });

  afterEach(() => {
    vi.useRealTimers();
    resetActivityCoordinatorForTests();
    resetAttentionBusForTests();
  });

  it('1. 设备初始化超过 0.5 秒才显示 Orb', () => {
    render(<MiraActivityOverlay activity="device-initializing" />);
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    act(() => { vi.advanceTimersByTime(500); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    act(() => { vi.advanceTimersByTime(20); });
    expect(screen.getByTestId('thinking-orb')).toHaveAttribute('data-state', 'connecting');
    expect(screen.getByRole('status')).toHaveTextContent('正在识别并读取鼠标…');
  });

  it('1b. 设备在 0.5 秒内完成读取时完全不显示全局识别卡', () => {
    const { rerender } = render(<MiraActivityOverlay activity="device-initializing" />);
    act(() => { vi.advanceTimersByTime(500); });
    rerender(<MiraActivityOverlay activity={null} />);
    act(() => { vi.advanceTimersByTime(1000); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('1a. 和鼠标沟通的 applying-settings Orb 使用当前主题 accent 着色', () => {
    render(<MiraInlineActivity active activity="applying-settings" delayMs={0} />);

    const orb = screen.getByTestId('thinking-orb');
    expect(orb).toHaveAttribute('data-color', 'var(--accent)');
    expect(orb.closest('.mira-inline-activity')?.querySelector('svg')).toBeNull();
  });

  it('2. 设备 ready 后 Orb 立即退出（不等 420ms 尾段），随后才播放 ready Beam', async () => {
    const announce = vi.fn(() => true);
    render(<MiraActivityOverlay activity="device-initializing" />);
    act(() => { vi.advanceTimersByTime(550); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    act(() => {
      void announceAfterOrbExit('device:app', announce, request('ready-1', 'device:app'));
    });
    expect(announce).not.toHaveBeenCalled();
    // 立即退出提示后，Orb 先退出，Beam 才提交（Orb 真实注销后才提交完成反馈）。
    act(() => { vi.advanceTimersByTime(1); });
    expect(screen.getByRole('status')).toHaveAttribute('data-phase', 'exiting');
    await act(async () => {
      vi.advanceTimersByTime(100);
      await Promise.resolve();
    });
    expect(announce).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    // 业务状态未结束也不得回弹（suppression）。
    act(() => { vi.advanceTimersByTime(1000); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
  });

  it('3. ready Beam 已活跃时，Orb 不出现（渲染层兜底仲裁）', () => {
    act(() => {
      announceAttentionRequest(request('ready-1', 'device:app'));
    });
    render(<MiraActivityOverlay activity="device-initializing" />);
    act(() => { vi.advanceTimersByTime(1000); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('4. 等待鼠标就位显示 connecting Orb 与等待文案', () => {
    render(<MiraActivityOverlay activity="awaiting-mouse" />);
    act(() => { vi.advanceTimersByTime(350); });
    expect(screen.getByTestId('thinking-orb')).toHaveAttribute('data-state', 'connecting');
    expect(screen.getByRole('status')).toHaveTextContent('正在等待鼠标就位…');
  });

  it('5. 更新检查完成、available Beam 播放时，检查 Orb 不存在', () => {
    render(
      <MiraInlineActivity active activity="checking-app-update" />,
    );
    act(() => { vi.advanceTimersByTime(350); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    act(() => {
      announceAttentionRequest(request('app-update-1', 'about-update'));
    });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
  });

  it('6. installed / restart Beam 同样不与 Orb 重叠（pulse-inner 变体）', () => {
    act(() => {
      announceAttentionRequest(request('restart-1', 'about-update', 20, 'pulse-inner'));
    });
    render(<MiraInlineActivity active activity="checking-app-update" />);
    act(() => { vi.advanceTimersByTime(1000); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
  });

  it('7. 不同 scope 的任务互不误伤', () => {
    act(() => {
      announceAttentionRequest(request('plugin-update-1', 'settings-plugin'));
    });
    render(<MiraActivityOverlay activity="device-initializing" />);
    act(() => { vi.advanceTimersByTime(550); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();
  });

  it('8. 卸载后 timer/rAF/observer 清理，协调层状态清空', () => {
    const { unmount } = render(<MiraActivityOverlay activity="device-initializing" />);
    act(() => { vi.advanceTimersByTime(550); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();
    expect(isActivityVisible('device:app')).toBe(true);

    unmount();
    expect(isActivityVisible('device:app')).toBe(false);
    // 继续推进时间：不应再有状态更新或报错。
    expect(() => { act(() => { vi.advanceTimersByTime(5000); }); }).not.toThrow();
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
  });

  it('9. 快任务低于 300ms：Orb 不出现', () => {
    const { rerender } = render(<MiraInlineActivity active activity="scanning-devices" />);
    act(() => { vi.advanceTimersByTime(200); });
    rerender(<MiraInlineActivity active={false} activity="scanning-devices" />);
    act(() => { vi.advanceTimersByTime(1000); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
  });

  it('9a. 显式 0ms 延迟的 Inline Orb 在首帧立即出现', () => {
    render(
      <MiraInlineActivity active activity="applying-settings" delayMs={0} />,
    );
    expect(screen.getByTestId('thinking-orb')).toHaveAttribute('data-state', 'working');
  });

  it('10. 无 Beam 的普通任务仍满足 420ms 最短可见', () => {
    const { rerender } = render(<MiraInlineActivity active activity="scanning-devices" />);
    act(() => { vi.advanceTimersByTime(300); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    rerender(<MiraInlineActivity active={false} activity="scanning-devices" />);
    act(() => { vi.advanceTimersByTime(300); });
    // 已过 600ms，距出现仅 300ms，仍应可见（最短 420ms）。
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    act(() => { vi.advanceTimersByTime(200); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
  });

  it('11. 同 scope 多实例：最后一个注销才判定退出，Beam 等待真实退出', async () => {
    const first = render(<MiraInlineActivity active activity="checking-app-update" />);
    const second = render(<MiraInlineActivity active activity="checking-app-update" />);
    act(() => { vi.advanceTimersByTime(350); });
    expect(isActivityVisible('about-update')).toBe(true);

    // 注销其中一个，另一个仍可见 → scope 仍不可判定为退出。
    first.unmount();
    expect(isActivityVisible('about-update')).toBe(true);

    const announce = vi.fn(() => true);
    act(() => {
      void announceAfterOrbExit('about-update', announce, request('app-update-1', 'about-update'));
    });
    expect(announce).not.toHaveBeenCalled();

    // 最后一个实例注销后，等待中的 Beam 才提交。
    second.unmount();
    expect(isActivityVisible('about-update')).toBe(false);
    await act(async () => {
      await Promise.resolve();
    });
    expect(announce).toHaveBeenCalledTimes(1);
  });

  it('12. 重挂载新实例不被上一轮退出提示污染（delta exit hint）', () => {
    const first = render(<MiraInlineActivity active activity="checking-app-update" />);
    act(() => { vi.advanceTimersByTime(300); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    // 让当前实例退出（提示历史 +1），然后在实例仍在挂载时完成退出。
    act(() => { bumpActivityExitHint('about-update'); });
    act(() => { vi.advanceTimersByTime(50); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    first.unmount();

    // 新实例挂载：历史累计的退出提示必须以“挂载前基线”看待，
    // 不能把上一轮历史当作新事件而整轮抑制。
    render(<MiraInlineActivity active activity="checking-app-update" />);
    act(() => { vi.advanceTimersByTime(300); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();
  });

  it('13. 不可见时 announce/等待保持同步语义（already-hidden）', async () => {
    const announce = vi.fn(() => true);
    act(() => {
      void announceAfterOrbExit('device:app', announce, request('ready-1', 'device:app'));
    });
    expect(announce).toHaveBeenCalledTimes(1);
    await expect(waitForActivityExit('device:app')).resolves.toBe('already-hidden');
  });

  it('14. 注销不发生的 scope 用超时兜底，不提交可能过期的 Beam', async () => {
    const token = Symbol('stuck-orb');
    registerVisibleActivity('device:app', token);
    const announce = vi.fn(() => true);
    act(() => {
      void announceAfterOrbExit('device:app', announce, request('ready-1', 'device:app'));
    });
    expect(announce).not.toHaveBeenCalled();
    await act(async () => { vi.advanceTimersByTime(1001); });
    // 超时不提交：可能过期、会与 Orb 长时间重叠的 Beam 不值得播放。
    expect(announce).not.toHaveBeenCalled();
    unregisterVisibleActivity('device:app', token);
    expect(isActivityVisible('device:app')).toBe(false);
  });

  it('15. 显式 null 不被 DOM 兜底覆盖；未提供 prop 时才允许 DOM 检测', () => {
    const marker = document.createElement('div');
    marker.className = 'dashboard is-initializing';
    document.body.appendChild(marker);
    try {
      const explicit = render(<MiraActivityOverlay activity={null} />);
      act(() => { vi.advanceTimersByTime(350); });
      expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
      expect(screen.queryByRole('status')).not.toBeInTheDocument();
      explicit.unmount();

      render(<MiraActivityOverlay />);
      act(() => { vi.advanceTimersByTime(550); });
      expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();
    } finally {
      marker.remove();
    }
  });

  it('16. device-initializing → null：Orb 立即退出，Beam 在 Orb 注销后提交', async () => {
    const announce = vi.fn(() => {
      // 提交瞬间 scope 必须已经没有可见 Orb（P0-4）。
      expect(isActivityVisible('device:app')).toBe(false);
      return true;
    });
    const { rerender } = render(<MiraActivityOverlay activity="device-initializing" />);
    act(() => { vi.advanceTimersByTime(550); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    // 业务状态变 null，但 Orb 仍处于最短可见尾段。
    rerender(<MiraActivityOverlay activity={null} />);
    act(() => { vi.advanceTimersByTime(50); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    act(() => {
      void announceAfterOrbExit('device:app', announce, request('ready-app-1', 'device:app'));
    });
    expect(announce).not.toHaveBeenCalled();
    act(() => { vi.advanceTimersByTime(1); });
    expect(screen.getByRole('status')).toHaveAttribute('data-phase', 'exiting');
    await act(async () => {
      vi.advanceTimersByTime(100);
      await Promise.resolve();
    });
    // 不等剩余最短可见尾段，Orb 先退出，Beam 随后提交。
    expect(announce).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
  });

  it('17. awaiting-mouse → null：鼠标就绪后 Orb 立即退出再播 ready Beam', async () => {
    const announce = vi.fn(() => true);
    const { rerender } = render(<MiraActivityOverlay activity="awaiting-mouse" />);
    act(() => { vi.advanceTimersByTime(350); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    rerender(<MiraActivityOverlay activity={null} />);
    act(() => {
      void announceAfterOrbExit('device:app', announce, request('ready-mouse', 'device:app'));
    });
    expect(announce).not.toHaveBeenCalled();
    act(() => { vi.advanceTimersByTime(1); });
    expect(screen.getByRole('status')).toHaveAttribute('data-phase', 'exiting');
    await act(async () => {
      vi.advanceTimersByTime(100);
      await Promise.resolve();
    });
    expect(announce).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
  });

  it('18. activity=null 但无完成事件：仍遵守最短可见尾段', () => {
    const { rerender } = render(<MiraActivityOverlay activity="device-initializing" />);
    act(() => { vi.advanceTimersByTime(550); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    rerender(<MiraActivityOverlay activity={null} />);
    act(() => { vi.advanceTimersByTime(300); });
    // 已显示 300ms（< 420ms），无 exit hint 时不得提前隐藏。
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    act(() => { vi.advanceTimersByTime(120); });
    expect(screen.getByRole('status')).toHaveAttribute('data-phase', 'exiting');
    act(() => { vi.advanceTimersByTime(100); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
  });

  it('19. 旧 token 注销后同 scope 新 token 已注册：旧 Beam 不提交且不补播', async () => {
    const tokenA = Symbol('stale-task-a');
    const tokenB = Symbol('new-task-b');
    registerVisibleActivity('device:app', tokenA);
    const announce = vi.fn(() => true);

    act(() => {
      void announceAfterOrbExit('device:app', announce, request('ready-stale', 'device:app'));
    });
    expect(announce).not.toHaveBeenCalled();

    // 旧 token 注销 → wait 解析；promise continuation 执行前，同 scope
    // 新任务注册新 token。
    act(() => {
      unregisterVisibleActivity('device:app', tokenA);
      registerVisibleActivity('device:app', tokenB);
    });
    await act(async () => { await Promise.resolve(); });
    expect(announce).not.toHaveBeenCalled();

    // 新任务结束：旧 Beam 不会自动补播。
    act(() => { unregisterVisibleActivity('device:app', tokenB); });
    await act(async () => { await Promise.resolve(); });
    expect(announce).not.toHaveBeenCalled();
  });

  it('20. 无新 token 时旧完成事件正常提交（对照组）', async () => {
    const tokenA = Symbol('solo-task');
    registerVisibleActivity('device:app', tokenA);
    const announce = vi.fn(() => true);

    act(() => {
      void announceAfterOrbExit('device:app', announce, request('ready-solo', 'device:app'));
    });
    expect(announce).not.toHaveBeenCalled();

    act(() => { unregisterVisibleActivity('device:app', tokenA); });
    await act(async () => { await Promise.resolve(); });
    expect(announce).toHaveBeenCalledTimes(1);
    expect(isActivityVisible('device:app')).toBe(false);
  });

  it('21. 带图标按钮 0–300ms 保持完整标签、Orb 出现替换、结束后恢复', () => {
    const fallback = <span data-testid="download-icon" />;
    const { rerender } = render(
      <MiraActivityButton
        className="action-btn"
        active
        activity="exporting-battery-history"
        leading={fallback}
      >
        导出
      </MiraActivityButton>,
    );
    expect(screen.getByTestId('download-icon')).toBeInTheDocument();
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();

    act(() => { vi.advanceTimersByTime(350); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();
    expect(screen.getByText('导出')).toHaveClass('is-concealed');

    rerender(
      <MiraActivityButton
        className="action-btn"
        active={false}
        activity="exporting-battery-history"
        leading={fallback}
      >
        导出
      </MiraActivityButton>,
    );
    act(() => { vi.advanceTimersByTime(500); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    expect(screen.getByTestId('download-icon')).toBeInTheDocument();
    expect(screen.getByText('导出')).not.toHaveClass('is-concealed');
  });

  it.each<[MiraActivityKind, string]>([
    ['checking-app-update', '检查更新'],
    ['checking-plugin-updates', '检查插件更新'],
    ['checking-local-ai-updates', '检查本地 AI 更新'],
  ])('22. 纯文本按钮 overlay 布局（%s）：空闲与 0–300ms 无图标/空槽，Orb 结构正确，结束后恢复', (activity, label) => {
    const { rerender } = render(
      <MiraActivityButton active activity={activity}>{label}</MiraActivityButton>,
    );
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    expect(document.querySelector('.mira-inline-activity')).not.toBeInTheDocument();
    expect(screen.getByText(label)).not.toHaveClass('is-concealed');

    act(() => { vi.advanceTimersByTime(350); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();
    expect(document.querySelector('.mira-inline-activity')).toHaveClass('mira-inline-activity--overlay');
    // Orb 与原动作文字由同一组件同步切换：文字保留尺寸/accessible name，
    // 但带明确隐藏类，绝不会出现二者同时可见的歧义帧。
    expect(screen.getByText(label)).toHaveClass('is-concealed');
    expect(screen.getByRole('button', { name: label })).toHaveAttribute('data-mira-processing', 'true');
    expect(screen.getByRole('button', { name: label })).toHaveAttribute('aria-busy', 'true');
    expect(screen.getByRole('button', { name: label })).toBeDisabled();

    rerender(
      <MiraActivityButton active={false} activity={activity}>{label}</MiraActivityButton>,
    );
    act(() => { vi.advanceTimersByTime(500); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    expect(document.querySelector('.mira-inline-activity')).not.toBeInTheDocument();
    expect(screen.getByText(label)).not.toHaveClass('is-concealed');
    expect(screen.getByRole('button', { name: label })).not.toHaveAttribute('data-mira-processing');
  });

  it('22a. 关于页检查完成仲裁时，Orb 退出到 busy 结束之间不跳回“检查更新”', async () => {
    const announce = vi.fn(() => true);
    const { rerender } = render(
      <MiraActivityButton active activity="checking-app-update">
        检查更新
      </MiraActivityButton>,
    );
    act(() => { vi.advanceTimersByTime(350); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();
    expect(screen.getByText('检查更新')).toHaveClass('is-concealed');

    // 更新结果先到：完成 Beam 请求 Orb 立即退出，但 About 的 finally 尚未
    // 把 manualCheckBusy 归 false。此间原文案必须继续隐藏，不能跳闪一帧。
    act(() => {
      void announceAfterOrbExit(
        'about-update',
        announce,
        request('app-update-ready', 'about-update'),
      );
    });
    act(() => { vi.advanceTimersByTime(0); });
    await act(async () => { await Promise.resolve(); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    expect(screen.getByText('检查更新')).toHaveClass('is-concealed');
    expect(screen.getByRole('button', { name: '检查更新' })).toHaveAttribute(
      'data-mira-processing',
      'true',
    );

    // 业务 busy 真正结束后才恢复原文案，并使用仅退出侧的淡入动画。
    rerender(
      <MiraActivityButton active={false} activity="checking-app-update">
        检查更新
      </MiraActivityButton>,
    );
    expect(screen.getByText('检查更新')).not.toHaveClass('is-concealed');
    expect(screen.getByText('检查更新')).toHaveClass('is-restoring');
  });

  it('22b. 电量分析 Orb 嵌入现有 Modal 表面，视觉上不重复显示动作文字', () => {
    render(<MiraEmbeddedActivity active activity="battery-analysis" aiAnalysisEnabled />);
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();

    act(() => { vi.advanceTimersByTime(350); });

    expect(screen.getByTestId('thinking-orb')).toHaveAttribute('data-state', 'solving');
    expect(screen.getByRole('status')).toHaveAttribute(
      'aria-label',
      '正在整理电量记录并生成本地分析…',
    );
    expect(screen.queryByText('正在整理电量记录并生成本地分析…')).not.toBeInTheDocument();
    expect(document.querySelector('.mira-activity-overlay')).not.toBeInTheDocument();
    expect(document.querySelector('.mira-activity-card')).not.toBeInTheDocument();
  });

  it('23. StrictMode 双挂载卸载后 scope 无残留', () => {
    const { unmount } = render(
      <StrictMode>
        <MiraInlineActivity active activity="checking-app-update" />
      </StrictMode>,
    );
    act(() => { vi.advanceTimersByTime(350); });
    expect(isActivityVisible('about-update')).toBe(true);

    unmount();
    expect(isActivityVisible('about-update')).toBe(false);
  });

  it('24. 连续两轮 device ready：第二轮 Orb 也立即退出，不等完剩余最短可见尾段', async () => {
    const announce = vi.fn(() => true);
    const { rerender } = render(<MiraActivityOverlay activity="device-initializing" />);

    // 第一轮：ready 后 Orb 立即退出，Beam 提交一次。
    act(() => { vi.advanceTimersByTime(550); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();
    rerender(<MiraActivityOverlay activity={null} />);
    act(() => {
      void announceAfterOrbExit('device:app', announce, request('ready-1', 'device:app'));
    });
    expect(announce).not.toHaveBeenCalled();
    act(() => { vi.advanceTimersByTime(1); });
    expect(screen.getByRole('status')).toHaveAttribute('data-phase', 'exiting');
    await act(async () => {
      vi.advanceTimersByTime(100);
      await Promise.resolve();
    });
    expect(announce).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();

    // 第二轮：同一组件、同一协调层，scope 重新建立后退出提示基线一并重置。
    rerender(<MiraActivityOverlay activity="device-initializing" />);
    act(() => { vi.advanceTimersByTime(550); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();
    rerender(<MiraActivityOverlay activity={null} />);
    act(() => {
      void announceAfterOrbExit('device:app', announce, request('ready-2', 'device:app'));
    });
    expect(announce).toHaveBeenCalledTimes(1);
    act(() => { vi.advanceTimersByTime(1); });
    expect(screen.getByRole('status')).toHaveAttribute('data-phase', 'exiting');
    await act(async () => {
      vi.advanceTimersByTime(100);
      await Promise.resolve();
    });
    // 第二轮依然立即退出（距出现仅约 50ms，未等完 420ms 尾段）并再次提交。
    expect(announce).toHaveBeenCalledTimes(2);
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
  });

  it('25. 已 active 的旧完成 Beam 与新任务 B 并存：B 文案保留、Beam 结束后 B Orb 出现', () => {
    // A 的完成 Beam 正在播放（同 scope）。
    act(() => {
      announceAttentionRequest(request('a-done-1', 'about-update'));
    });
    const { rerender } = render(
      <MiraActivityButton
        className="secondary"
        active
        activity="checking-app-update"
      >
        检查更新
      </MiraActivityButton>,
    );
    act(() => { vi.advanceTimersByTime(400); });
    // B 的过程文案仍在；Beam active 期间 B 的 Orb 被渲染层抑制，但不永久。
    expect(screen.getByText('检查更新')).toBeInTheDocument();
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();

    // 旧 Beam 结束：B 仍 active，Orb 立即恢复（无需重新等 300ms）。
    act(() => {
      finishActiveAttentionRequest();
    });
    act(() => { vi.advanceTimersByTime(0); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();
    expect(screen.getByTestId('thinking-orb')).toHaveAttribute('data-state', 'searching');

    // B 结束后正常退出，不残留。
    rerender(
      <MiraActivityButton
        className="secondary"
        active={false}
        activity="checking-app-update"
      >
        检查更新
      </MiraActivityButton>,
    );
    act(() => { vi.advanceTimersByTime(500); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
  });

  it('26. A completion 微任务间隙中 B 已开始（仍在 300ms 延迟）：旧 completion 不晚到覆盖 B', async () => {
    const announce = vi.fn(() => true);
    const first = render(<MiraInlineActivity active activity="checking-app-update" />);
    act(() => { vi.advanceTimersByTime(350); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    // A 完成事件：等待 A 的 Orb 退出。
    act(() => {
      void announceAfterOrbExit('about-update', announce, request('a-done-1', 'about-update'));
    });
    act(() => { vi.advanceTimersByTime(0); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();

    // B 任务立即开始（同 scope），此时仍处于 300ms 延迟期（Orb 尚未出现）。
    first.unmount();
    render(<MiraInlineActivity active activity="checking-app-update" />);
    // 冲刷微任务：A 的 completion continuation 在这里执行。
    await act(async () => { await Promise.resolve(); });
    // 期望契约：B 已真实开始，A 尚未提交的 completion 不应在之后覆盖 B。
    expect(announce).not.toHaveBeenCalled();

    // B 的过程语义不受影响：Orb 正常出现。
    act(() => { vi.advanceTimersByTime(350); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();
    expect(screen.getByTestId('thinking-orb')).toHaveAttribute('data-state', 'searching');
  });
});
