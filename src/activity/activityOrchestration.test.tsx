// SPDX-License-Identifier: AGPL-3.0-or-later
// P0-3：Orb 与 Attention Beam 生命周期边界的关键集成测试。
// mock 第三方视觉组件（thinking-orbs），不测试 Canvas 绘制。
import { act, render, screen } from '@testing-library/react';
import { StrictMode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MiraActivityOverlay } from './MiraActivityOverlay';
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
import { announceAttentionRequest, resetAttentionBusForTests } from '../attention/attentionCore';
import type { AttentionBeamRequest } from '../attention/attentionTypes';
import type { MiraActivityKind } from './activityCatalog';

vi.mock('thinking-orbs', () => ({
  ThinkingOrb: ({ state }: { state: string }) => (
    <span data-testid="thinking-orb" data-state={state} />
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

  it('1. 设备初始化超过 300ms 才显示 Orb', () => {
    render(<MiraActivityOverlay activity="device-initializing" />);
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    act(() => { vi.advanceTimersByTime(200); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    act(() => { vi.advanceTimersByTime(150); });
    expect(screen.getByTestId('thinking-orb')).toHaveAttribute('data-state', 'connecting');
    expect(screen.getByRole('status')).toHaveTextContent('正在识别并读取鼠标…');
  });

  it('2. 设备 ready 后 Orb 立即退出（不等 420ms 尾段），随后才播放 ready Beam', async () => {
    const announce = vi.fn(() => true);
    render(<MiraActivityOverlay activity="device-initializing" />);
    act(() => { vi.advanceTimersByTime(350); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    act(() => {
      void announceAfterOrbExit('device:app', announce, request('ready-1', 'device:app'));
    });
    expect(announce).not.toHaveBeenCalled();
    // 立即退出提示（0ms）+ rAF（~16ms）后，Orb 先退出，Beam 才提交。
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
    act(() => { vi.advanceTimersByTime(350); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();
  });

  it('8. 卸载后 timer/rAF/observer 清理，协调层状态清空', () => {
    const { unmount } = render(<MiraActivityOverlay activity="device-initializing" />);
    act(() => { vi.advanceTimersByTime(350); });
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
      act(() => { vi.advanceTimersByTime(350); });
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
    act(() => { vi.advanceTimersByTime(350); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    // 业务状态变 null，但 Orb 仍处于最短可见尾段。
    rerender(<MiraActivityOverlay activity={null} />);
    act(() => { vi.advanceTimersByTime(50); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    act(() => {
      void announceAfterOrbExit('device:app', announce, request('ready-app-1', 'device:app'));
    });
    expect(announce).not.toHaveBeenCalled();
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
    await act(async () => {
      vi.advanceTimersByTime(100);
      await Promise.resolve();
    });
    expect(announce).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
  });

  it('18. activity=null 但无完成事件：仍遵守最短可见尾段', () => {
    const { rerender } = render(<MiraActivityOverlay activity="device-initializing" />);
    act(() => { vi.advanceTimersByTime(300); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    rerender(<MiraActivityOverlay activity={null} />);
    act(() => { vi.advanceTimersByTime(300); });
    // 已显示 300ms（< 420ms），无 exit hint 时不得提前隐藏。
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    act(() => { vi.advanceTimersByTime(200); });
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

  it('21. fallback 图标 0–300ms 保持、Orb 出现替换、结束后恢复', () => {
    const fallback = <span data-testid="download-icon" />;
    const { rerender } = render(
      <button className="action-btn">
        <MiraInlineActivity active activity="exporting-battery-history" fallback={fallback} />
        <span>导出</span>
      </button>,
    );
    expect(screen.getByTestId('download-icon')).toBeInTheDocument();
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();

    act(() => { vi.advanceTimersByTime(350); });
    expect(screen.queryByTestId('download-icon')).not.toBeInTheDocument();
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    rerender(
      <button className="action-btn">
        <MiraInlineActivity active={false} activity="exporting-battery-history" fallback={fallback} />
        <span>导出</span>
      </button>,
    );
    act(() => { vi.advanceTimersByTime(500); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    expect(screen.getByTestId('download-icon')).toBeInTheDocument();
  });

  it.each<[MiraActivityKind, string]>([
    ['checking-app-update', '检查更新'],
    ['checking-plugin-updates', '检查插件更新'],
    ['checking-local-ai-updates', '检查本地 AI 更新'],
  ])('22. 纯文本按钮 overlay 布局（%s）：空闲与 0–300ms 无图标/空槽，Orb 结构正确，结束后恢复', (activity, label) => {
    const { rerender } = render(
      <button className="secondary mira-activity-button">
        <MiraInlineActivity active activity={activity} layout="overlay" />
        <span>{label}</span>
      </button>,
    );
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    expect(document.querySelector('.mira-inline-activity')).not.toBeInTheDocument();

    act(() => { vi.advanceTimersByTime(350); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();
    expect(document.querySelector('.mira-inline-activity')).toHaveClass('mira-inline-activity--overlay');

    rerender(
      <button className="secondary mira-activity-button">
        <MiraInlineActivity active={false} activity={activity} layout="overlay" />
        <span>{label}</span>
      </button>,
    );
    act(() => { vi.advanceTimersByTime(500); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    expect(document.querySelector('.mira-inline-activity')).not.toBeInTheDocument();
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
});
